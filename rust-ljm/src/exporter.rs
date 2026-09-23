//! Serves archived Parquet sample files as streamed CSV exports.
//!
//! The streamer publishes LabJack samples to the local NATS JetStream, and the
//! archiver writes them to Parquet files under
//! `<PARQUET_DIR>/assetNNN/YYYY-MM-DD/chNN/*.parquet`. This binary reads those files
//! back for a requested asset, channel list and time range, applies the calibration
//! stored in each file's metadata, and sends the rows as CSV with the columns
//! `timestamp,channel,raw_value,calibrated_value,calibration_id`.
//!
//! It runs in one of two modes that share the request format ([`ExportRequest`])
//! and the scanning logic:
//!
//! * Worker mode (the default and the normal edge-box setup) subscribes to the core
//!   NATS subject `<NATS_SUBJECT>.<site>.<box>.<source_id>.export.request` and
//!   answers each request on its reply subject. Every reply message carries the
//!   header `Avena-Export-Frame` naming the frame: `meta`, then one or more
//!   `chunk` frames with raw CSV bytes (about 512 KiB each, the last one smaller),
//!   then `summary` and `complete`. Failures produce an `error` frame. See
//!   [`process_nats_request`] and [`NatsCsvStreamer`] for the exact payloads and
//!   the ack-subject backpressure.
//! * Direct mode serves the same export over a WebSocket at `ws://<EXPORTER_ADDR>/export`.
//!   The client sends one JSON request as a text message and receives the same
//!   frames as JSON text messages, with CSV chunks (about 128 KiB) as binary
//!   messages. See [`process_socket`].
//!
//! # Configuration
//!
//! * `EXPORTER_MODE` - `worker`, or `direct` (alias `local`). Trimmed and
//!   case-insensitive; any other value is a startup error. Default: `worker`.
//! * `EXPORTER_ADDR` - TCP listen address for the WebSocket server in direct mode.
//!   Default: `0.0.0.0:9001`.
//! * `PARQUET_DIR` - Root of the Parquet archive. A missing directory only logs a
//!   warning at startup. Default: `parquet`.
//! * `NATS_SERVERS` - Worker mode. Comma-separated NATS server URLs. Default:
//!   `nats://127.0.0.1:4222`.
//! * `NATS_CREDS_FILE` - Worker mode. NATS credentials file. Default: `apt.creds`.
//! * `NATS_SUBJECT` - Worker mode. Root token of the request subject. Default:
//!   `avenars`.
//! * `SITE_ID` - Worker mode. Site token of the request subject. Default:
//!   `unknown-site`.
//! * `EXPORT_BOX_ID` - Worker mode. Box token of the request subject; falls back to
//!   `BOX_ID`. One of the two is required in worker mode.
//! * `BOX_ID` - Worker mode. Used when `EXPORT_BOX_ID` is unset.
//! * `SOURCE_ID` - Worker mode. Source token of the request subject. Default:
//!   `unknown-source`.
//! * `SOURCE_TYPE` - Worker mode. Read and passed to the subject builder, which
//!   currently ignores it. Default: `labjack`.
//!
//! # Design
//!
//! * Worker mode exists so the webapp needs only its existing WebSocket connection
//!   to central NATS. The browser publishes a request with a reply inbox, the NATS
//!   leaf routes it to the worker on the box that holds the Parquet files, and the
//!   CSV comes back the same way. Direct mode requires the client to reach the box
//!   itself. Core NATS has no flow control, so the worker waits for client acks
//!   on `ack_subject` every 8 chunks instead of flooding a slow browser.
//! * Each Parquet row group stores min/max statistics for the timestamp column.
//!   [`read_matching_rows`] skips row groups whose range lies outside the request
//!   without decoding them, and reads the two columns with typed readers rather than
//!   building a row object per sample.
//! * Rows within a file are in time order, so [`Rfc3339Formatter`] builds the date
//!   and time up to the second with chrono once per second and appends only the
//!   fractional part per row. This is faster than a full chrono format per row and
//!   produces identical text to chrono's `to_rfc3339`.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use async_nats::{ConnectOptions, HeaderMap};
use async_trait::async_trait;
use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use futures_util::StreamExt;
use parquet::{
    column::reader::get_typed_column_reader,
    data_type::{DataType, DoubleType, Int64Type},
    file::{
        metadata::RowGroupMetaData,
        reader::{FileReader, RowGroupReader, SerializedFileReader},
        statistics::Statistics,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Write as _;
mod calibration;
mod nats_config;
mod subjects;

use calibration::CalibrationSpec;

/// NATS header whose value names the frame type of each worker-mode reply message.
const EXPORT_FRAME_HEADER: &str = "Avena-Export-Frame";
/// Frame name for the metadata frame (`fileName`, `contentType`) sent before any CSV.
const EXPORT_FRAME_META: &str = "meta";
/// Frame name for a CSV payload chunk (raw bytes, not JSON).
const EXPORT_FRAME_CHUNK: &str = "chunk";
/// Frame name for the final byte count and missing-channel list.
const EXPORT_FRAME_SUMMARY: &str = "summary";
/// Frame name marking the end of a successful export.
const EXPORT_FRAME_COMPLETE: &str = "complete";
/// Frame name for request validation or processing errors (`message`).
const EXPORT_FRAME_ERROR: &str = "error";
/// Runtime mode used when `EXPORTER_MODE` is unset.
const DEFAULT_EXPORTER_MODE: &str = "worker";
/// WebSocket listen address used in direct mode when `EXPORTER_ADDR` is unset.
const DEFAULT_EXPORTER_ADDR: &str = "0.0.0.0:9001";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
/// Direct-mode metadata frame sent before CSV chunks.
///
/// Serializes as `{"type":"meta","fileName":...,"contentType":...}`.
struct MetaFrame<'a> {
    /// Always [`EXPORT_FRAME_META`]; serialized as `type`.
    #[serde(rename = "type")]
    frame_type: &'static str,
    /// Suggested download file name; serialized as `fileName`.
    file_name: &'a str,
    /// MIME type of the payload (`text/csv`); serialized as `contentType`.
    content_type: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
/// Direct-mode summary frame sent after all CSV chunks are delivered.
///
/// Serializes as `{"type":"summary","bytesSent":...,"missingChannels":[...]}`.
struct SummaryFrame<'a> {
    /// Always [`EXPORT_FRAME_SUMMARY`]; serialized as `type`.
    #[serde(rename = "type")]
    frame_type: &'static str,
    /// Total CSV bytes sent, including the header line; serialized as `bytesSent`.
    bytes_sent: usize,
    /// Requested channels that produced no rows; serialized as `missingChannels`.
    missing_channels: &'a [u8],
}

#[derive(Serialize)]
/// Direct-mode error frame sent when an export request cannot be served.
///
/// Serializes as `{"type":"error","message":...}`.
struct ErrorFrame<'a> {
    /// Always [`EXPORT_FRAME_ERROR`]; serialized as `type`.
    #[serde(rename = "type")]
    frame_type: &'static str,
    /// Human-readable error text.
    message: &'a str,
}

#[derive(Clone)]
/// State shared by the Axum WebSocket handler and the NATS worker tasks.
struct AppState {
    /// Mode the process was started in.
    mode: ExporterMode,
    /// Root of the Parquet archive (`PARQUET_DIR`).
    parquet_root: Arc<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Exporter runtime mode.
enum ExporterMode {
    /// Serve export requests directly over a local WebSocket endpoint.
    Direct,
    /// Subscribe to NATS export request subjects and reply with framed chunks.
    Worker,
}

impl ExporterMode {
    /// Reads and validates `EXPORTER_MODE`.
    ///
    /// The value is trimmed and lowercased. `direct` and `local` select
    /// [`ExporterMode::Direct`], `worker` selects [`ExporterMode::Worker`], and an
    /// unset variable falls back to [`DEFAULT_EXPORTER_MODE`].
    ///
    /// # Errors
    ///
    /// Returns an error if the variable is set to any other value.
    fn from_env() -> Result<Self> {
        match std::env::var("EXPORTER_MODE")
            .unwrap_or_else(|_| DEFAULT_EXPORTER_MODE.to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "direct" | "local" => Ok(Self::Direct),
            "worker" => Ok(Self::Worker),
            other => Err(anyhow!(
                "invalid EXPORTER_MODE '{other}', expected direct or worker"
            )),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "lowercase")]
/// Requested export format, written in JSON as `"csv"` or `"parquet"`.
enum ExportFormat {
    /// Stream a generated CSV file.
    Csv,
    /// Placeholder for future Parquet passthrough support. Requests with this
    /// format are rejected with the error `parquet streaming not yet supported`.
    Parquet,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
/// Export request accepted by both WebSocket and NATS worker modes.
///
/// Sent as a JSON object with snake_case field names, for example:
///
/// ```text
/// {"asset":1,"channels":[0,1],"start":"2026-09-21T14:00:00Z",
///  "end":"2026-09-21T15:00:00Z","format":"csv",
///  "download_name":"run.csv","ack_subject":"_INBOX.abc.ack"}
/// ```
///
/// Validation (in [`process_nats_request`] and [`serve_export_request`]) rejects an
/// empty channel list, unparseable timestamps, `end` before `start`, and any format
/// other than CSV.
struct ExportRequest {
    /// Asset number; selects the `assetNNN` directory under the Parquet root.
    asset: u32,
    /// LabJack channel numbers to export. Sorted and deduplicated before use, and
    /// exported one channel after another in that order. Must not be empty.
    channels: Vec<u8>,
    /// Start of the range, RFC 3339 (any offset, converted to UTC). Inclusive.
    start: String,
    /// End of the range, RFC 3339 (any offset, converted to UTC). Inclusive, and
    /// may equal `start`.
    end: String,
    /// Output format. Default: `csv`.
    #[serde(default = "default_format")]
    format: ExportFormat,
    /// File name reported in the `meta` frame. Default:
    /// `labjack_asset<NNN>_<start>_<end>.csv` with times as `%Y%m%dT%H%M%S` in UTC.
    download_name: Option<String>,
    /// Worker mode only. Subject on which the client publishes one message per
    /// received chunk; the worker subscribes to it and pauses every 8 chunks until
    /// the acks arrive. Missing or blank disables backpressure. Ignored in direct
    /// mode.
    ack_subject: Option<String>,
}

/// Default export format used when a request omits `format`.
fn default_format() -> ExportFormat {
    ExportFormat::Csv
}

#[tokio::main]
/// Starts the exporter in direct WebSocket mode or NATS worker mode.
///
/// Reads `EXPORTER_MODE`, `EXPORTER_ADDR` and `PARQUET_DIR`, and prints a warning if
/// the Parquet directory does not exist. Worker mode then hands off to
/// [`run_worker`]. Direct mode binds `EXPORTER_ADDR` and serves the `/export`
/// WebSocket route with [`handle_ws`] until the server stops.
///
/// # Errors
///
/// Returns an error if `EXPORTER_MODE` is invalid, if the listen address cannot be
/// bound or the server fails (direct mode), or if [`run_worker`] fails.
async fn main() -> Result<()> {
    let mode = ExporterMode::from_env()?;
    let listen_addr =
        std::env::var("EXPORTER_ADDR").unwrap_or_else(|_| DEFAULT_EXPORTER_ADDR.into());
    let parquet_root = std::env::var("PARQUET_DIR").unwrap_or_else(|_| "parquet".into());
    let root_path = PathBuf::from(parquet_root);

    if matches!(mode, ExporterMode::Direct | ExporterMode::Worker) && !root_path.exists() {
        println!(
            "[exporter] Warning: parquet directory '{}' does not exist.",
            root_path.display()
        );
    }

    match mode {
        ExporterMode::Worker => run_worker(root_path).await,
        ExporterMode::Direct => {
            let state = AppState {
                mode,
                parquet_root: Arc::new(root_path),
            };

            let app = Router::new()
                .route("/export", get(handle_ws))
                .with_state(state);

            println!(
                "[exporter] mode={} listening on ws://{listen_addr}/export",
                match mode {
                    ExporterMode::Direct => "direct",
                    ExporterMode::Worker => "worker",
                }
            );

            let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
            axum::serve(listener, app).await?;
            Ok(())
        }
    }
}

/// Connects to NATS using the standard credentials and server environment.
///
/// Loads the credentials file named by `NATS_CREDS_FILE` (default `apt.creds`) and
/// connects to the servers listed in `NATS_SERVERS` (see
/// [`nats_config::servers_from_env`]).
///
/// # Errors
///
/// Returns an error if the credentials file cannot be loaded, if `NATS_SERVERS`
/// is invalid or empty, or if the connection fails.
async fn connect_nats_from_env() -> Result<async_nats::Client> {
    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| anyhow!("failed to load NATS creds: {e}"))?;
    let servers = nats_config::servers_from_env().map_err(|e| anyhow!("{e}"))?;
    let client = opts
        .connect(servers)
        .await
        .map_err(|e| anyhow!("NATS connect failed: {e}"))?;
    Ok(client)
}

/// Reads the box identifier required to construct the worker request subject.
///
/// Uses `EXPORT_BOX_ID`, falling back to `BOX_ID`, and passes the value through
/// [`sanitize_token`].
///
/// # Errors
///
/// Returns an error if neither variable is set (or both hold invalid Unicode).
fn worker_box_id_from_env() -> Result<String> {
    std::env::var("EXPORT_BOX_ID")
        .or_else(|_| std::env::var("BOX_ID"))
        .map(|value| sanitize_token(&value))
        .map_err(|_| anyhow!("worker mode requires EXPORT_BOX_ID or BOX_ID"))
}

/// Normalizes namespace text into a subject-safe token.
///
/// Trims the input, lowercases ASCII letters, keeps ASCII digits, `-` and `_`,
/// turns whitespace, `.` and `/` into `-`, and drops every other character. Leading
/// and trailing `-` are removed.
///
/// # Arguments
///
/// * `raw` - Text to normalize, such as a box id.
///
/// # Returns
///
/// The normalized token, or `unknown` if nothing is left.
///
/// # Examples
///
/// ```text
/// sanitize_token(" MU1.Box ") == "mu1-box"
/// sanitize_token("***")       == "unknown"
/// ```
fn sanitize_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || ch == '.' || ch == '/' {
            out.push('-');
        }
    }

    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

/// Runs exporter worker mode by subscribing to one NATS request subject.
///
/// Connects with [`connect_nats_from_env`] and subscribes to
/// `<NATS_SUBJECT>.<SITE_ID>.<box>.<SOURCE_ID>.export.request` built by
/// [`subjects::archive_export_request_subject`]. Each incoming request is handled in
/// its own Tokio task, with no limit on concurrent exports, and answered on the
/// message's reply subject by [`process_nats_request`]. Requests without a reply
/// subject are logged and dropped. If a request fails, the error is logged and a
/// best-effort `error` frame (`{"type":"error","message":...}`) is published to the
/// reply subject, possibly after `meta` and some `chunk` frames were already sent.
///
/// # Arguments
///
/// * `parquet_root` - Root of the Parquet archive.
///
/// # Returns
///
/// `Ok(())` when the subscription ends.
///
/// # Errors
///
/// Returns an error if the NATS connection fails, if neither `EXPORT_BOX_ID` nor
/// `BOX_ID` is set, or if the subscription cannot be created.
async fn run_worker(parquet_root: PathBuf) -> Result<()> {
    let client = connect_nats_from_env().await?;
    let state = AppState {
        mode: ExporterMode::Worker,
        parquet_root: Arc::new(parquet_root),
    };
    let nats_subject = std::env::var("NATS_SUBJECT").unwrap_or_else(|_| "avenars".to_string());
    let site_id = std::env::var("SITE_ID").unwrap_or_else(|_| "unknown-site".to_string());
    let box_id = worker_box_id_from_env()?;
    let source_type = std::env::var("SOURCE_TYPE").unwrap_or_else(|_| "labjack".to_string());
    let source_id = std::env::var("SOURCE_ID").unwrap_or_else(|_| "unknown-source".to_string());
    let subject = subjects::archive_export_request_subject(
        &nats_subject,
        Some(&site_id),
        Some(&box_id),
        Some(&source_type),
        Some(&source_id),
    );
    let mut subscriber = client
        .subscribe(subject.clone())
        .await
        .map_err(|e| anyhow!("failed to subscribe to export worker subject '{subject}': {e}"))?;

    println!("[exporter] worker listening on NATS subject '{subject}'");

    while let Some(message) = subscriber.next().await {
        let client = client.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let Some(reply) = message.reply.clone() else {
                eprintln!("[exporter] ignoring NATS export request without reply subject");
                return;
            };
            if let Err(err) = process_nats_request(
                client.clone(),
                reply.clone(),
                message.payload.to_vec(),
                state,
            )
            .await
            {
                eprintln!("[exporter] worker request failed: {err:#}");
                let _ = publish_nats_json(
                    &client,
                    reply,
                    EXPORT_FRAME_ERROR,
                    &json!({"type":"error","message": err.to_string()}),
                )
                .await;
            }
        });
    }

    Ok(())
}

/// Validates and serves one NATS export request payload.
///
/// Every frame is published to `reply` with the header `Avena-Export-Frame` set to
/// the frame name. The sequence is:
///
/// 1. `meta`: JSON `{"type":"meta","fileName":...,"contentType":"text/csv"}`.
/// 2. `chunk` (one or more): raw CSV bytes, not JSON. The first chunk starts with
///    the header line. See [`NatsCsvStreamer`] for sizes and acks.
/// 3. `summary`: JSON `{"type":"summary","bytesSent":N,"missingChannels":[...]}`.
/// 4. `complete`: JSON `{"type":"complete"}`.
///
/// Validation failures (no channels, `end` before `start`, non-CSV format) publish
/// only an `error` frame (`{"type":"error","message":...}`) and return `Ok`, with
/// no `complete` frame. If `ack_subject` is set and not blank, the function
/// subscribes to it before streaming.
///
/// # Arguments
///
/// * `nc` - Connected NATS client.
/// * `reply` - Reply subject of the request message.
/// * `payload` - JSON-encoded [`ExportRequest`].
/// * `state` - Shared state holding the Parquet root.
///
/// # Errors
///
/// Returns an error if the payload is not a valid [`ExportRequest`], if `start` or
/// `end` is not RFC 3339, if a publish, flush or the ack subscription fails, if an
/// ack does not arrive in time, or if a channel's day directory cannot be listed.
/// The caller ([`run_worker`]) turns these into an `error` frame.
async fn process_nats_request(
    nc: async_nats::Client,
    reply: async_nats::Subject,
    payload: Vec<u8>,
    state: AppState,
) -> Result<()> {
    let mut req: ExportRequest =
        serde_json::from_slice(&payload).map_err(|e| anyhow!("invalid request payload: {e}"))?;

    if req.channels.is_empty() {
        publish_nats_json(
            &nc,
            reply,
            EXPORT_FRAME_ERROR,
            &json!({"type":"error","message":"no channels requested"}),
        )
        .await?;
        return Ok(());
    }
    req.channels.sort_unstable();
    req.channels.dedup();

    let (start, end) = parse_range(&req.start, &req.end)?;
    if end < start {
        publish_nats_json(
            &nc,
            reply,
            EXPORT_FRAME_ERROR,
            &json!({"type":"error","message":"end must be after start"}),
        )
        .await?;
        return Ok(());
    }

    if !matches!(req.format, ExportFormat::Csv) {
        publish_nats_json(
            &nc,
            reply,
            EXPORT_FRAME_ERROR,
            &json!({"type":"error","message":"parquet streaming not yet supported"}),
        )
        .await?;
        return Ok(());
    }

    let file_name = req.download_name.clone().unwrap_or_else(|| {
        format!(
            "labjack_asset{:03}_{}_{}.csv",
            req.asset,
            start.format("%Y%m%dT%H%M%S"),
            end.format("%Y%m%dT%H%M%S"),
        )
    });

    publish_nats_json(
        &nc,
        reply.clone(),
        EXPORT_FRAME_META,
        &json!({
            "type": "meta",
            "fileName": file_name,
            "contentType": "text/csv"
        }),
    )
    .await?;

    let ack_sub = match req.ack_subject.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(subject) => Some(nc.subscribe(subject.to_string()).await?),
        None => None,
    };
    let mut stream = NatsCsvStreamer::new(nc, reply, ack_sub, req.asset, start, end);
    let missing = stream
        .stream_channels(&state.parquet_root, &req.channels)
        .await?;
    stream.finish(missing).await?;
    Ok(())
}

/// Publishes a JSON export control frame on a NATS reply subject.
///
/// # Arguments
///
/// * `nc` - Connected NATS client.
/// * `reply` - Subject to publish to.
/// * `frame` - Frame name written to the `Avena-Export-Frame` header.
/// * `value` - JSON body of the message.
///
/// # Errors
///
/// Returns an error if `value` cannot be serialized or the publish fails.
async fn publish_nats_json(
    nc: &async_nats::Client,
    reply: async_nats::Subject,
    frame: &str,
    value: &serde_json::Value,
) -> Result<()> {
    let mut headers = HeaderMap::new();
    headers.insert(EXPORT_FRAME_HEADER, frame);
    nc.publish_with_headers(reply, headers, serde_json::to_vec(value)?.into())
        .await?;
    Ok(())
}

/// Upgrades an HTTP request into a WebSocket export session.
///
/// The session runs [`process_socket`]; its errors are logged, not returned.
///
/// # Arguments
///
/// * `ws` - Axum WebSocket upgrade extractor.
/// * `state` - Shared exporter state.
async fn handle_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = process_socket(socket, state).await {
            eprintln!("[exporter] websocket error: {err:#}");
        }
    })
}

/// Handles one direct-mode WebSocket export session.
///
/// Reads the request with [`read_export_request`] and serves it with
/// [`serve_export_request`]. If serving fails, it sends an `error` frame, a
/// `complete` frame and a close frame, ignoring send failures. On success the
/// socket is dropped after the `complete` frame without an explicit close frame.
///
/// # Arguments
///
/// * `socket` - Upgraded WebSocket connection.
/// * `state` - Shared exporter state.
///
/// # Errors
///
/// Returns an error if the request cannot be read or parsed, or if the process is
/// in worker mode (not reached in practice, since worker mode starts no WebSocket
/// server).
async fn process_socket(mut socket: WebSocket, state: AppState) -> Result<()> {
    let request = read_export_request(&mut socket).await?;

    match state.mode {
        ExporterMode::Direct => {
            let mut sink = WebSocketSink::new(socket);
            if let Err(err) = serve_export_request(&state.parquet_root, &mut sink, &request).await {
                sink.send_error(&err.to_string()).await.ok();
                sink.send_complete().await.ok();
                sink.send_close().await.ok();
            }
        }
        ExporterMode::Worker => {
            return Err(anyhow!("worker mode does not serve websocket exports"));
        }
    }

    Ok(())
}

/// Reads the initial JSON export request from a WebSocket.
///
/// The first message must be a text message holding an [`ExportRequest`]. If it
/// is any other message type, an `error` frame with `expected JSON request` is
/// sent first.
///
/// # Arguments
///
/// * `socket` - WebSocket to read from.
///
/// # Errors
///
/// Returns an error if the socket closes or fails before a message arrives, if the
/// first message is not text, or if the text is not a valid [`ExportRequest`].
async fn read_export_request(socket: &mut WebSocket) -> Result<ExportRequest> {
    let Some(msg) = socket.next().await else {
        return Err(anyhow!("websocket closed before export request"));
    };

    let Message::Text(text) = msg? else {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"expected JSON request"}).to_string(),
            ))
            .await
            .ok();
        return Err(anyhow!("expected JSON request"));
    };

    let req: ExportRequest =
        serde_json::from_str(&text).map_err(|e| anyhow!("invalid request payload: {e}"))?;
    Ok(req)
}

/// Validates a request and streams its CSV response through an export sink.
///
/// Sorts and deduplicates the channels, checks the range and format, sends the
/// `meta` frame, then streams every channel with [`CsvStreamer`] and finishes with
/// `summary` and `complete`. Unlike [`process_nats_request`], validation failures
/// are returned as errors for the caller to report.
///
/// # Arguments
///
/// * `parquet_root` - Root of the Parquet archive.
/// * `sink` - Transport that receives the frames.
/// * `req` - Export request; cloned before normalization.
///
/// # Errors
///
/// Returns an error if no channels are requested, if `start` or `end` is not RFC
/// 3339, if `end` is before `start`, if the format is not CSV, if a channel's day
/// directory cannot be listed, or if the sink fails to send.
async fn serve_export_request<S: ExportSink + Send>(
    parquet_root: &Path,
    sink: &mut S,
    req: &ExportRequest,
) -> Result<()> {
    let mut req = req.clone();

    if req.channels.is_empty() {
        return Err(anyhow!("no channels requested"));
    }
    req.channels.sort_unstable();
    req.channels.dedup();

    let (start, end) = parse_range(&req.start, &req.end)?;
    if end < start {
        return Err(anyhow!("end must be after start"));
    }

    if !matches!(req.format, ExportFormat::Csv) {
        return Err(anyhow!("parquet streaming not yet supported"));
    }

    let file_name = req.download_name.clone().unwrap_or_else(|| {
        format!(
            "labjack_asset{:03}_{}_{}.csv",
            req.asset,
            start.format("%Y%m%dT%H%M%S"),
            end.format("%Y%m%dT%H%M%S"),
        )
    });

    sink.send_meta(&file_name, "text/csv").await?;

    let mut stream = CsvStreamer::new(sink, req.asset, start, end);
    let missing = stream.stream_channels(parquet_root, &req.channels).await?;
    stream.finish(missing).await?;
    Ok(())
}

#[async_trait]
/// Transport abstraction for framed export responses.
///
/// Used by [`CsvStreamer`] in direct mode. Worker mode publishes to NATS directly
/// through [`NatsCsvStreamer`] and [`publish_nats_json`] instead. Every method
/// returns an error if the underlying transport fails to send.
trait ExportSink {
    /// Sends the file metadata frame.
    ///
    /// # Arguments
    ///
    /// * `file_name` - Suggested download file name.
    /// * `content_type` - MIME type of the payload.
    async fn send_meta(&mut self, file_name: &str, content_type: &str) -> Result<()>;
    /// Sends one CSV payload chunk.
    ///
    /// # Arguments
    ///
    /// * `data` - Raw CSV bytes.
    async fn send_chunk(&mut self, data: Vec<u8>) -> Result<()>;
    /// Sends the final export summary.
    ///
    /// # Arguments
    ///
    /// * `bytes_sent` - Total CSV bytes sent in chunks, including the header line.
    /// * `missing_channels` - Requested channels that produced no rows.
    async fn send_summary(&mut self, bytes_sent: usize, missing_channels: &[u8]) -> Result<()>;
    /// Sends the completion frame.
    async fn send_complete(&mut self) -> Result<()>;
    /// Sends an error frame.
    ///
    /// # Arguments
    ///
    /// * `message` - Human-readable error text.
    async fn send_error(&mut self, message: &str) -> Result<()>;
}

/// WebSocket implementation of [`ExportSink`].
///
/// Control frames go out as JSON text messages and CSV chunks as binary messages.
struct WebSocketSink {
    /// Upgraded client connection.
    socket: WebSocket,
}

impl WebSocketSink {
    /// Wraps a WebSocket as an export sink.
    ///
    /// # Arguments
    ///
    /// * `socket` - Upgraded client connection.
    fn new(socket: WebSocket) -> Self {
        Self { socket }
    }

    /// Sends a WebSocket close frame.
    ///
    /// # Errors
    ///
    /// Returns an error if the send fails.
    async fn send_close(&mut self) -> Result<()> {
        self.socket.send(Message::Close(None)).await?;
        Ok(())
    }
}

#[async_trait]
impl ExportSink for WebSocketSink {
    /// Sends metadata as a text JSON WebSocket message ([`MetaFrame`]).
    async fn send_meta(&mut self, file_name: &str, content_type: &str) -> Result<()> {
        self.socket
            .send(Message::Text(serde_json::to_string(&MetaFrame {
                frame_type: EXPORT_FRAME_META,
                file_name,
                content_type,
            })?))
            .await?;
        Ok(())
    }

    /// Sends a CSV chunk as a binary WebSocket message.
    async fn send_chunk(&mut self, data: Vec<u8>) -> Result<()> {
        self.socket.send(Message::Binary(data)).await?;
        Ok(())
    }

    /// Sends byte count and missing channels as a text JSON message ([`SummaryFrame`]).
    async fn send_summary(&mut self, bytes_sent: usize, missing_channels: &[u8]) -> Result<()> {
        self.socket
            .send(Message::Text(serde_json::to_string(&SummaryFrame {
                frame_type: EXPORT_FRAME_SUMMARY,
                bytes_sent,
                missing_channels,
            })?))
            .await?;
        Ok(())
    }

    /// Sends `{"type":"complete"}` as a text message.
    async fn send_complete(&mut self) -> Result<()> {
        self.socket
            .send(Message::Text(json!({"type":"complete"}).to_string()))
            .await?;
        Ok(())
    }

    /// Sends a text JSON error message ([`ErrorFrame`]).
    async fn send_error(&mut self, message: &str) -> Result<()> {
        self.socket
            .send(Message::Text(serde_json::to_string(&ErrorFrame {
                frame_type: EXPORT_FRAME_ERROR,
                message,
            })?))
            .await?;
        Ok(())
    }
}

/// CSV stream builder for direct WebSocket exports.
///
/// Buffers CSV rows and hands them to an [`ExportSink`] in chunks of about
/// [`Self::CHUNK_SIZE`] bytes. There is no ack protocol; backpressure comes from
/// awaiting each send on the sink.
struct CsvStreamer<'a, S: ExportSink + Send> {
    /// Transport that receives the frames.
    sink: &'a mut S,
    /// CSV bytes not yet sent; starts with the header line.
    chunk: Vec<u8>,
    /// CSV bytes handed to the sink so far.
    bytes_sent: usize,
    /// Asset number used to locate the Parquet partitions.
    asset: u32,
    /// Inclusive start of the export range.
    start: DateTime<Utc>,
    /// Inclusive end of the export range.
    end: DateTime<Utc>,
}

/// CSV stream builder for NATS worker exports.
///
/// Buffers CSV rows and publishes them as `chunk` frames (raw bytes with the header
/// `Avena-Export-Frame: chunk`) to the reply subject once the buffer reaches
/// [`Self::CHUNK_SIZE`] (512 KiB), so every chunk except the last is at least that
/// size, exceeding it by less than one row. After every
/// [`Self::FLUSH_EVERY_CHUNKS`] (8) chunks it flushes the NATS client and, when
/// the request named an `ack_subject`, waits until the client has published one
/// ack message per chunk sent, allowing [`Self::ACK_TIMEOUT_SECS`] (30 s) for
/// each. The content of ack messages is ignored. Without an ack subject the
/// worker only flushes, and nothing stops it from outrunning a slow client.
struct NatsCsvStreamer {
    /// Connected NATS client.
    client: async_nats::Client,
    /// Reply subject of the export request.
    reply: async_nats::Subject,
    /// Subscription to the request's `ack_subject`, if one was given.
    ack_sub: Option<async_nats::Subscriber>,
    /// CSV bytes not yet published; starts with the header line.
    chunk: Vec<u8>,
    /// Chunks published since the last forced client flush.
    chunks_since_flush: usize,
    /// Chunks published since the last completed ack wait.
    chunks_since_ack: usize,
    /// CSV bytes published so far.
    bytes_sent: usize,
    /// Asset number used to locate the Parquet partitions.
    asset: u32,
    /// Inclusive start of the export range.
    start: DateTime<Utc>,
    /// Inclusive end of the export range.
    end: DateTime<Utc>,
}

impl NatsCsvStreamer {
    /// Buffer size in bytes at which a `chunk` frame is published (512 KiB).
    const CHUNK_SIZE: usize = 512 * 1024;
    /// Number of chunks published between forced NATS flushes and between ack
    /// waits.
    const FLUSH_EVERY_CHUNKS: usize = 8;
    /// Maximum time to wait for each client chunk acknowledgement, in seconds.
    const ACK_TIMEOUT_SECS: u64 = 30;

    /// Creates a NATS CSV streamer and writes the CSV header into the buffer.
    ///
    /// # Arguments
    ///
    /// * `client` - Connected NATS client.
    /// * `reply` - Reply subject of the export request.
    /// * `ack_sub` - Subscription to the client's ack subject, or `None` to skip
    ///   ack waits.
    /// * `asset` - Asset number used to locate the Parquet partitions.
    /// * `start` - Inclusive start of the export range.
    /// * `end` - Inclusive end of the export range.
    fn new(
        client: async_nats::Client,
        reply: async_nats::Subject,
        ack_sub: Option<async_nats::Subscriber>,
        asset: u32,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Self {
        let mut chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        chunk.extend_from_slice(b"timestamp,channel,raw_value,calibrated_value,calibration_id\n");
        Self {
            client,
            reply,
            ack_sub,
            chunk,
            chunks_since_flush: 0,
            chunks_since_ack: 0,
            bytes_sent: 0,
            asset,
            start,
            end,
        }
    }

    /// Streams all requested channels, one after another, in the given order.
    ///
    /// # Arguments
    ///
    /// * `parquet_root` - Root of the Parquet archive.
    /// * `channels` - Channels to export.
    ///
    /// # Returns
    ///
    /// The channels that produced no rows in the range, in request order.
    ///
    /// # Errors
    ///
    /// Returns the first error from [`Self::stream_channel`], prefixed with
    /// `channel NN:`.
    async fn stream_channels(&mut self, parquet_root: &Path, channels: &[u8]) -> Result<Vec<u8>> {
        let mut missing = Vec::new();
        for &channel in channels {
            let found = self
                .stream_channel(parquet_root, channel)
                .await
                .map_err(|e| anyhow!("channel {channel:02}: {e}"))?;
            if !found {
                missing.push(channel);
            }
        }
        Ok(missing)
    }

    /// Publishes the current CSV buffer as one `chunk` frame.
    ///
    /// Does nothing if the buffer is empty. Adds the chunk to `bytes_sent`, flushes
    /// the NATS client every [`Self::FLUSH_EVERY_CHUNKS`] chunks, and calls
    /// [`Self::wait_for_acks`] once that many chunks are unacknowledged.
    ///
    /// # Errors
    ///
    /// Returns an error if the publish or flush fails, or if [`Self::wait_for_acks`]
    /// fails.
    async fn flush(&mut self) -> Result<()> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.chunk);
        self.bytes_sent += data.len();
        let mut headers = HeaderMap::new();
        headers.insert(EXPORT_FRAME_HEADER, EXPORT_FRAME_CHUNK);
        self.client
            .publish_with_headers(self.reply.clone(), headers, data.into())
            .await?;
        self.chunks_since_flush += 1;
        self.chunks_since_ack += 1;
        if self.chunks_since_flush >= Self::FLUSH_EVERY_CHUNKS {
            self.client.flush().await?;
            self.chunks_since_flush = 0;
        }
        if self.chunks_since_ack >= Self::FLUSH_EVERY_CHUNKS {
            self.wait_for_acks().await?;
        }
        self.chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        Ok(())
    }

    /// Waits for client acknowledgements for recently published chunks.
    ///
    /// Returns at once if there is no ack subscription or no pending chunk.
    /// Otherwise flushes the client so the chunks actually leave, then waits for one
    /// message on the ack subject per pending chunk, allowing
    /// [`Self::ACK_TIMEOUT_SECS`] for each, and resets the pending count.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush fails, if any ack takes longer than
    /// [`Self::ACK_TIMEOUT_SECS`], or if the ack subscription closes.
    async fn wait_for_acks(&mut self) -> Result<()> {
        let Some(ack_sub) = self.ack_sub.as_mut() else {
            return Ok(());
        };
        let pending = self.chunks_since_ack;
        if pending == 0 {
            return Ok(());
        }

        self.client.flush().await?;
        for _ in 0..pending {
            tokio::time::timeout(
                std::time::Duration::from_secs(Self::ACK_TIMEOUT_SECS),
                ack_sub.next(),
            )
            .await
            .map_err(|_| anyhow!("timed out waiting for export chunk acknowledgement"))?
            .ok_or_else(|| anyhow!("export acknowledgement subscription closed"))?;
        }
        self.chunks_since_ack = 0;
        Ok(())
    }

    /// Appends one CSV row to the current chunk.
    ///
    /// The row is `<timestamp>,chNN,<raw_value>,<calibrated_value>,<calibration_id>`,
    /// with the floats in Rust's default `Display` format. When the buffer reaches
    /// [`Self::CHUNK_SIZE`] it is sent with [`Self::flush`].
    ///
    /// # Arguments
    ///
    /// * `timestamp` - RFC 3339 timestamp text.
    /// * `channel` - LabJack channel number, written as `chNN`.
    /// * `raw_value` - Value stored in the Parquet file.
    /// * `calibrated_value` - `raw_value` after the file's calibration.
    /// * `calibration_id` - Calibration id written in the last column.
    ///
    /// # Errors
    ///
    /// Returns an error if the resulting flush fails.
    async fn push_record(
        &mut self,
        timestamp: &str,
        channel: u8,
        raw_value: f64,
        calibrated_value: f64,
        calibration_id: &str,
    ) -> Result<()> {
        writeln!(
            self.chunk,
            "{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id}"
        )?;
        if self.chunk.len() >= Self::CHUNK_SIZE {
            self.flush().await?;
        }
        Ok(())
    }

    /// Flushes data and sends summary and completion frames.
    ///
    /// Publishes the remaining buffer, waits for all outstanding acks, then
    /// publishes `summary` (`{"type":"summary","bytesSent":N,"missingChannels":[...]}`
    /// with the channels sorted and deduplicated) and `complete`
    /// (`{"type":"complete"}`), and flushes the client.
    ///
    /// # Arguments
    ///
    /// * `missing_channels` - Channels that produced no rows.
    ///
    /// # Errors
    ///
    /// Returns an error if a publish, flush or ack wait fails.
    async fn finish(mut self, mut missing_channels: Vec<u8>) -> Result<()> {
        self.flush().await?;
        self.wait_for_acks().await?;
        missing_channels.sort_unstable();
        missing_channels.dedup();
        publish_nats_json(
            &self.client,
            self.reply.clone(),
            EXPORT_FRAME_SUMMARY,
            &json!({
                "type": "summary",
                "bytesSent": self.bytes_sent,
                "missingChannels": missing_channels
            }),
        )
        .await?;
        publish_nats_json(
            &self.client,
            self.reply.clone(),
            EXPORT_FRAME_COMPLETE,
            &json!({"type":"complete"}),
        )
        .await?;
        self.client.flush().await?;
        Ok(())
    }

    /// Streams all Parquet files for one asset/channel over the requested date range.
    ///
    /// For each UTC date from the start date to the end date inclusive, lists
    /// `<root>/assetNNN/YYYY-MM-DD/chNN/` (skipping days whose directory does not
    /// exist) and processes its `.parquet` files in file-name order with
    /// [`Self::stream_parquet_file`]. A file that fails to read or stream is logged
    /// and skipped, and the export continues.
    ///
    /// # Arguments
    ///
    /// * `root` - Root of the Parquet archive.
    /// * `channel` - LabJack channel number.
    ///
    /// # Returns
    ///
    /// `true` if at least one row in the range was emitted for this channel.
    ///
    /// # Errors
    ///
    /// Returns an error if an existing day directory cannot be listed.
    async fn stream_channel(&mut self, root: &Path, channel: u8) -> Result<bool> {
        let mut found = false;
        for day in date_range(self.start.date_naive(), self.end.date_naive()) {
            let day_dir = root
                .join(format!("asset{:03}", self.asset))
                .join(day.format("%Y-%m-%d").to_string())
                .join(format!("ch{:02}", channel));
            if !day_dir.exists() {
                continue;
            }

            let mut files: Vec<PathBuf> = fs::read_dir(&day_dir)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.ends_with(".parquet"))
                        .unwrap_or(false)
                })
                .collect();
            files.sort();

            for path in files {
                if let Err(err) = self.stream_parquet_file(&path, channel, &mut found).await {
                    eprintln!("[exporter] skipping {} due to error: {err}", path.display());
                }
            }
        }
        Ok(found)
    }

    /// Reads one Parquet file and emits matching rows as CSV records.
    ///
    /// Uses [`read_matching_rows`] to load the rows in the range, applies the file's
    /// calibration to each raw value, and formats timestamps with a fresh
    /// [`Rfc3339Formatter`]. Rows keep the order in which they are stored.
    ///
    /// # Arguments
    ///
    /// * `path` - Parquet file to read.
    /// * `channel` - LabJack channel number written in each row.
    /// * `found` - Set to `true` when at least one row is emitted; never reset.
    ///
    /// # Errors
    ///
    /// Returns an error if [`read_matching_rows`] fails or a row cannot be sent.
    async fn stream_parquet_file(
        &mut self,
        path: &Path,
        channel: u8,
        found: &mut bool,
    ) -> Result<()> {
        let matched = read_matching_rows(
            path,
            datetime_to_unix_ns(self.start),
            datetime_to_unix_ns(self.end),
        )?;
        let mut formatter = Rfc3339Formatter::new();
        for (timestamp_unix_ns, raw_value) in matched.rows {
            let ts = formatter.format(timestamp_unix_ns);
            let calibrated_value = matched.calibration.apply(raw_value);
            *found = true;
            self.push_record(
                ts,
                channel,
                raw_value,
                calibrated_value,
                &matched.calibration_id,
            )
            .await?;
        }
        Ok(())
    }
}

impl<'a, S: ExportSink + Send> CsvStreamer<'a, S> {
    /// Buffer size in bytes at which a chunk is sent to the sink (128 KiB).
    const CHUNK_SIZE: usize = 128 * 1024;

    /// Creates a direct CSV streamer and writes the CSV header into the buffer.
    ///
    /// # Arguments
    ///
    /// * `sink` - Transport that receives the frames.
    /// * `asset` - Asset number used to locate the Parquet partitions.
    /// * `start` - Inclusive start of the export range.
    /// * `end` - Inclusive end of the export range.
    fn new(sink: &'a mut S, asset: u32, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        let mut chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        chunk.extend_from_slice(b"timestamp,channel,raw_value,calibrated_value,calibration_id\n");
        Self {
            sink,
            chunk,
            bytes_sent: 0,
            asset,
            start,
            end,
        }
    }

    /// Streams all requested channels, one after another, in the given order.
    ///
    /// # Arguments
    ///
    /// * `parquet_root` - Root of the Parquet archive.
    /// * `channels` - Channels to export.
    ///
    /// # Returns
    ///
    /// The channels that produced no rows in the range, in request order.
    ///
    /// # Errors
    ///
    /// Returns the first error from [`Self::stream_channel`], prefixed with
    /// `channel NN:`.
    async fn stream_channels(&mut self, parquet_root: &Path, channels: &[u8]) -> Result<Vec<u8>> {
        let mut missing = Vec::new();
        for &channel in channels {
            let found = self
                .stream_channel(parquet_root, channel)
                .await
                .map_err(|e| anyhow!("channel {channel:02}: {e}"))?;
            if !found {
                missing.push(channel);
            }
        }
        Ok(missing)
    }

    /// Sends the current CSV buffer through the sink as one chunk.
    ///
    /// Does nothing if the buffer is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink fails to send.
    async fn flush(&mut self) -> Result<()> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.chunk);
        self.bytes_sent += data.len();
        self.sink.send_chunk(data).await?;
        self.chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        Ok(())
    }

    /// Appends one CSV row to the current chunk.
    ///
    /// The row is `<timestamp>,chNN,<raw_value>,<calibrated_value>,<calibration_id>`,
    /// with the floats in Rust's default `Display` format. When the buffer reaches
    /// [`Self::CHUNK_SIZE`] it is sent with [`Self::flush`].
    ///
    /// # Arguments
    ///
    /// * `timestamp` - RFC 3339 timestamp text.
    /// * `channel` - LabJack channel number, written as `chNN`.
    /// * `raw_value` - Value stored in the Parquet file.
    /// * `calibrated_value` - `raw_value` after the file's calibration.
    /// * `calibration_id` - Calibration id written in the last column.
    ///
    /// # Errors
    ///
    /// Returns an error if the resulting flush fails.
    async fn push_record(
        &mut self,
        timestamp: &str,
        channel: u8,
        raw_value: f64,
        calibrated_value: f64,
        calibration_id: &str,
    ) -> Result<()> {
        writeln!(
            self.chunk,
            "{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id}"
        )?;
        if self.chunk.len() >= Self::CHUNK_SIZE {
            self.flush().await?;
        }
        Ok(())
    }

    /// Flushes data and sends summary and completion frames.
    ///
    /// Sends the remaining buffer, then the summary (with the missing channels
    /// sorted and deduplicated) and the completion frame.
    ///
    /// # Arguments
    ///
    /// * `missing_channels` - Channels that produced no rows.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink fails to send.
    async fn finish(mut self, mut missing_channels: Vec<u8>) -> Result<()> {
        self.flush().await?;
        missing_channels.sort_unstable();
        missing_channels.dedup();
        self.sink
            .send_summary(self.bytes_sent, &missing_channels)
            .await?;
        self.sink.send_complete().await?;
        Ok(())
    }

    /// Streams all Parquet files for one asset/channel over the requested date range.
    ///
    /// For each UTC date from the start date to the end date inclusive, lists
    /// `<root>/assetNNN/YYYY-MM-DD/chNN/` (skipping days whose directory does not
    /// exist) and processes its `.parquet` files in file-name order with
    /// [`Self::stream_parquet_file`]. A file that fails to read or stream is logged
    /// and skipped, and the export continues.
    ///
    /// # Arguments
    ///
    /// * `root` - Root of the Parquet archive.
    /// * `channel` - LabJack channel number.
    ///
    /// # Returns
    ///
    /// `true` if at least one row in the range was emitted for this channel.
    ///
    /// # Errors
    ///
    /// Returns an error if an existing day directory cannot be listed.
    async fn stream_channel(&mut self, root: &Path, channel: u8) -> Result<bool> {
        let mut found = false;
        for day in date_range(self.start.date_naive(), self.end.date_naive()) {
            let day_dir = root
                .join(format!("asset{:03}", self.asset))
                .join(day.format("%Y-%m-%d").to_string())
                .join(format!("ch{:02}", channel));
            if !day_dir.exists() {
                continue;
            }

            let mut files: Vec<PathBuf> = fs::read_dir(&day_dir)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.ends_with(".parquet"))
                        .unwrap_or(false)
                })
                .collect();
            files.sort();

            for path in files {
                if let Err(err) = self.stream_parquet_file(&path, channel, &mut found).await {
                    eprintln!("[exporter] skipping {} due to error: {err}", path.display());
                }
            }
        }
        Ok(found)
    }

    /// Reads one Parquet file and emits matching rows as CSV records.
    ///
    /// Uses [`read_matching_rows`] to load the rows in the range, applies the file's
    /// calibration to each raw value, and formats timestamps with a fresh
    /// [`Rfc3339Formatter`]. Rows keep the order in which they are stored.
    ///
    /// # Arguments
    ///
    /// * `path` - Parquet file to read.
    /// * `channel` - LabJack channel number written in each row.
    /// * `found` - Set to `true` when at least one row is emitted; never reset.
    ///
    /// # Errors
    ///
    /// Returns an error if [`read_matching_rows`] fails or a row cannot be sent.
    async fn stream_parquet_file(
        &mut self,
        path: &Path,
        channel: u8,
        found: &mut bool,
    ) -> Result<()> {
        let matched = read_matching_rows(
            path,
            datetime_to_unix_ns(self.start),
            datetime_to_unix_ns(self.end),
        )?;
        let mut formatter = Rfc3339Formatter::new();
        for (timestamp_unix_ns, raw_value) in matched.rows {
            let ts = formatter.format(timestamp_unix_ns);
            let calibrated_value = matched.calibration.apply(raw_value);
            *found = true;
            self.push_record(
                ts,
                channel,
                raw_value,
                calibrated_value,
                &matched.calibration_id,
            )
            .await?;
        }
        Ok(())
    }
}

/// Converts a Unix nanosecond timestamp into RFC 3339 text.
///
/// Uses chrono's `to_rfc3339`, which writes a `+00:00` offset and the fraction as
/// 0, 3, 6 or 9 digits. Only the tests call it; the export path uses
/// [`Rfc3339Formatter`], which must produce the same text.
///
/// # Arguments
///
/// * `timestamp_unix_ns` - Nanoseconds since the Unix epoch.
///
/// # Examples
///
/// ```text
/// timestamp_unix_ns_to_rfc3339(1_790_000_000_008_000_000)
///     == "2026-09-21T14:13:20.008+00:00"
/// ```
fn timestamp_unix_ns_to_rfc3339(timestamp_unix_ns: i64) -> String {
    DateTime::<Utc>::from_timestamp_nanos(timestamp_unix_ns).to_rfc3339()
}

/// Formats timestamps exactly like [`timestamp_unix_ns_to_rfc3339`], faster.
///
/// Archived rows arrive in time order, so the date and time up to the second
/// changes only once per 100-2,000 rows. That part is built once per second
/// with chrono; only the fractional part is formatted per row, following
/// chrono's `to_rfc3339` rule: no fraction for whole seconds, otherwise 3, 6
/// or 9 digits for millisecond, microsecond or nanosecond precision.
///
/// Correctness does not depend on order: out-of-order timestamps only make the
/// prefix rebuild more often.
struct Rfc3339Formatter {
    /// Unix second the cached prefix belongs to, or `None` before the first call.
    second: Option<i64>,
    /// `YYYY-MM-DDTHH:MM:SS` text for the cached `second`.
    prefix: String,
    /// Output buffer reused across calls; holds the last formatted timestamp.
    buf: String,
}

impl Rfc3339Formatter {
    /// Creates a formatter with an empty cache.
    fn new() -> Self {
        Self {
            second: None,
            prefix: String::new(),
            buf: String::with_capacity(40),
        }
    }

    /// Formats one timestamp, reusing the date and time prefix if the second is
    /// unchanged since the previous call.
    ///
    /// # Arguments
    ///
    /// * `timestamp_unix_ns` - Nanoseconds since the Unix epoch; any `i64`,
    ///   including negative values.
    ///
    /// # Returns
    ///
    /// The RFC 3339 text, borrowed from the internal buffer until the next call.
    ///
    /// # Examples
    ///
    /// ```text
    /// format(1_790_000_000_008_000_000) == "2026-09-21T14:13:20.008+00:00"
    /// format(1_790_000_000_008_123_000) == "2026-09-21T14:13:20.008123+00:00"
    /// format(1_790_000_000_000_000_000) == "2026-09-21T14:13:20+00:00"
    /// ```
    fn format(&mut self, timestamp_unix_ns: i64) -> &str {
        use std::fmt::Write as _;
        let second = timestamp_unix_ns.div_euclid(1_000_000_000);
        let nanos = timestamp_unix_ns.rem_euclid(1_000_000_000) as u32;
        if self.second != Some(second) {
            // Seconds-based constructor: `second * 1e9` would overflow near i64::MIN.
            let text = DateTime::<Utc>::from_timestamp(second, 0)
                .map(|instant| instant.to_rfc3339())
                .unwrap_or_default();
            // Whole seconds render as "<date>T<time>+00:00"; keep the part before the offset.
            self.prefix = text.trim_end_matches("+00:00").to_string();
            self.second = Some(second);
        }
        self.buf.clear();
        self.buf.push_str(&self.prefix);
        let _ = if nanos == 0 {
            Ok(())
        } else if nanos % 1_000_000 == 0 {
            write!(self.buf, ".{:03}", nanos / 1_000_000)
        } else if nanos % 1_000 == 0 {
            write!(self.buf, ".{:06}", nanos / 1_000)
        } else {
            write!(self.buf, ".{nanos:09}")
        };
        self.buf.push_str("+00:00");
        &self.buf
    }
}

/// Converts an instant to Unix nanoseconds, clamping outside the i64 range.
///
/// Instants too far in the future become `i64::MAX` and instants too far in the
/// past become `i64::MIN` (roughly outside the years 1677 to 2262), so a very wide
/// request range still covers every stored sample.
///
/// # Arguments
///
/// * `instant` - Instant to convert.
fn datetime_to_unix_ns(instant: DateTime<Utc>) -> i64 {
    instant
        .timestamp_nanos_opt()
        .unwrap_or(if instant.timestamp() < 0 {
            i64::MIN
        } else {
            i64::MAX
        })
}

/// Rows of one Parquet file that fall inside an export range.
struct MatchedFile {
    /// Calibration read from the file's `calibration` metadata, or identity.
    calibration: CalibrationSpec,
    /// Calibration id for the CSV, from [`CalibrationSpec::id_or_default`].
    calibration_id: String,
    /// `(timestamp_unix_ns, raw_value)` pairs in the order stored in the file.
    rows: Vec<(i64, f64)>,
}

/// Returns whether a row group's timestamp statistics overlap `[start_ns, end_ns]`.
///
/// Uses the min/max statistics of column 0 (`timestamp_unix_ns`). Row groups
/// without usable INT64 statistics are always read, so the check can only skip
/// work, never lose rows.
///
/// # Arguments
///
/// * `meta` - Metadata of the row group.
/// * `start_ns` - Inclusive range start, Unix nanoseconds.
/// * `end_ns` - Inclusive range end, Unix nanoseconds.
fn row_group_may_match(meta: &RowGroupMetaData, start_ns: i64, end_ns: i64) -> bool {
    match meta.column(0).statistics() {
        Some(Statistics::Int64(stats)) => match (stats.min_opt(), stats.max_opt()) {
            (Some(min), Some(max)) => *max >= start_ns && *min <= end_ns,
            _ => true,
        },
        _ => true,
    }
}

/// Reads every value of one required column in a row group.
///
/// Reads in batches of up to the row group's row count until the reader returns no
/// more records. Definition and repetition levels are not read, so this is only
/// correct for `REQUIRED` columns, which is what the archive schema uses.
///
/// # Arguments
///
/// * `row_group` - Row group to read from.
/// * `index` - Column index within the row group.
///
/// # Errors
///
/// Returns an error if the column reader cannot be created or decoding fails.
///
/// # Panics
///
/// Panics (inside the parquet crate) if the column's physical type does not match
/// `T`.
fn read_column<T: DataType>(row_group: &dyn RowGroupReader, index: usize) -> Result<Vec<T::T>> {
    let rows = usize::try_from(row_group.metadata().num_rows()).unwrap_or(0);
    let mut reader = get_typed_column_reader::<T>(row_group.get_column_reader(index)?);
    let mut values = Vec::with_capacity(rows);
    loop {
        let (records, _, _) = reader.read_records(rows.max(1), None, None, &mut values)?;
        if records == 0 {
            break;
        }
    }
    Ok(values)
}

/// Reads the rows of one archived file whose timestamps fall in `[start_ns, end_ns]`.
///
/// Row groups whose timestamp statistics lie entirely outside the range are
/// skipped without being decoded, and the two columns are read with typed
/// column readers instead of building a generic row object per sample. Column 0
/// must be the INT64 timestamp and column 1 the DOUBLE value, as in the archive
/// schema. Calibration comes from [`read_calibration_from_metadata`].
///
/// # Arguments
///
/// * `path` - Parquet file to read.
/// * `start_ns` - Inclusive range start, Unix nanoseconds.
/// * `end_ns` - Inclusive range end, Unix nanoseconds.
///
/// # Returns
///
/// The matching rows in stored order, with the file's calibration and its id.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or parsed as Parquet, if a row
/// group or column cannot be read, or if a row group has different numbers of
/// timestamps and values.
///
/// # Panics
///
/// Panics if column 0 is not INT64 or column 1 is not DOUBLE (see [`read_column`]).
fn read_matching_rows(path: &Path, start_ns: i64, end_ns: i64) -> Result<MatchedFile> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open parquet file {}", path.display()))?;
    let reader = SerializedFileReader::new(file)
        .with_context(|| format!("failed to create reader for {}", path.display()))?;
    let calibration = read_calibration_from_metadata(&reader, path);
    let calibration_id = calibration.id_or_default().to_string();

    let mut rows = Vec::new();
    for index in 0..reader.num_row_groups() {
        if !row_group_may_match(reader.metadata().row_group(index), start_ns, end_ns) {
            continue;
        }
        let row_group = reader.get_row_group(index)?;
        let timestamps = read_column::<Int64Type>(row_group.as_ref(), 0)?;
        let values = read_column::<DoubleType>(row_group.as_ref(), 1)?;
        if timestamps.len() != values.len() {
            return Err(anyhow!(
                "row group {index} in {} has {} timestamps but {} values",
                path.display(),
                timestamps.len(),
                values.len()
            ));
        }
        rows.extend(
            timestamps
                .into_iter()
                .zip(values)
                .filter(|(ts, _)| *ts >= start_ns && *ts <= end_ns),
        );
    }

    Ok(MatchedFile {
        calibration,
        calibration_id,
        rows,
    })
}

/// Reads calibration metadata written by the archiver from a Parquet file.
///
/// Looks for the first key-value metadata entry named `calibration` and parses its
/// value as a JSON [`CalibrationSpec`].
///
/// # Arguments
///
/// * `reader` - Open reader for the file.
/// * `path` - File path, used only in the log message.
///
/// # Returns
///
/// The parsed calibration, or [`CalibrationSpec::default`] (unnamed identity) if
/// the entry is missing, has no value, or is not valid JSON. Invalid JSON is
/// logged.
fn read_calibration_from_metadata(
    reader: &SerializedFileReader<fs::File>,
    path: &Path,
) -> CalibrationSpec {
    let Some(kv) = reader.metadata().file_metadata().key_value_metadata() else {
        return CalibrationSpec::default();
    };

    let mut calibration_json = None;
    for item in kv {
        if item.key == "calibration" {
            calibration_json = item.value.as_deref();
            break;
        }
    }

    let Some(json) = calibration_json else {
        return CalibrationSpec::default();
    };

    match serde_json::from_str::<CalibrationSpec>(json) {
        Ok(spec) => spec,
        Err(err) => {
            eprintln!(
                "[exporter] invalid calibration metadata in {}: {err}",
                path.display()
            );
            CalibrationSpec::default()
        }
    }
}

/// Parses an RFC 3339 start/end range into UTC instants.
///
/// Does not check the order of the two instants; callers do that.
///
/// # Arguments
///
/// * `start` - Range start, RFC 3339 with any offset.
/// * `end` - Range end, RFC 3339 with any offset.
///
/// # Errors
///
/// Returns an error naming `start` or `end` if either is not valid RFC 3339.
fn parse_range(start: &str, end: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start = DateTime::parse_from_rfc3339(start)
        .map_err(|e| anyhow!("invalid start timestamp: {e}"))?
        .with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(end)
        .map_err(|e| anyhow!("invalid end timestamp: {e}"))?
        .with_timezone(&Utc);
    Ok((start, end))
}

/// Builds an inclusive list of UTC dates covered by an export request.
///
/// # Arguments
///
/// * `start` - First date.
/// * `end` - Last date, included. If it is before `start` the list is empty.
///
/// # Examples
///
/// ```text
/// date_range(2026-09-21, 2026-09-23) == [2026-09-21, 2026-09-22, 2026-09-23]
/// ```
fn date_range(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    let mut days = Vec::new();
    let mut current = start;
    while current <= end {
        days.push(current);
        current += ChronoDuration::days(1);
    }
    days
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::{
        column::writer::ColumnWriter,
        file::{properties::WriterProperties, writer::SerializedFileWriter},
        record::RowAccessor,
        schema::parser::parse_message_type,
    };
    use std::time::Instant;

    /// The exporter's reading logic before this change, kept as a reference.
    fn reference_rows(path: &Path, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<(i64, f64)> {
        let reader = SerializedFileReader::new(fs::File::open(path).unwrap()).unwrap();
        let mut out = Vec::new();
        let mut iter = reader.get_row_iter(None).unwrap();
        while let Some(row) = iter.next() {
            let row = row.unwrap();
            let ts_ns = row.get_long(0).unwrap();
            let text = DateTime::<Utc>::from_timestamp_nanos(ts_ns).to_rfc3339();
            let parsed = DateTime::parse_from_rfc3339(&text)
                .unwrap()
                .with_timezone(&Utc);
            if parsed < start || parsed > end {
                continue;
            }
            out.push((ts_ns, row.get_double(1).unwrap()));
        }
        out
    }

    /// Writes a two-column file with the given row groups (plain, uncompressed,
    /// like the archiver's older output).
    fn write_file(path: &Path, row_groups: &[Vec<(i64, f64)>]) {
        let schema = Arc::new(
            parse_message_type(
                "message schema { REQUIRED INT64 timestamp_unix_ns; REQUIRED DOUBLE value; }",
            )
            .unwrap(),
        );
        let props = Arc::new(WriterProperties::builder().build());
        let mut writer =
            SerializedFileWriter::new(fs::File::create(path).unwrap(), schema, props).unwrap();
        for group in row_groups {
            let mut rg = writer.next_row_group().unwrap();
            let ts: Vec<i64> = group.iter().map(|(t, _)| *t).collect();
            let vs: Vec<f64> = group.iter().map(|(_, v)| *v).collect();
            let mut col = rg.next_column().unwrap().unwrap();
            if let ColumnWriter::Int64ColumnWriter(w) = col.untyped() {
                w.write_batch(&ts, None, None).unwrap();
            }
            col.close().unwrap();
            let mut col = rg.next_column().unwrap().unwrap();
            if let ColumnWriter::DoubleColumnWriter(w) = col.untyped() {
                w.write_batch(&vs, None, None).unwrap();
            }
            col.close().unwrap();
            rg.close().unwrap();
        }
        writer.close().unwrap();
    }

    /// Converts Unix nanoseconds to a UTC instant.
    fn at(ns: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_nanos(ns)
    }

    /// Checks the typed reader against the row-iterator reference for many ranges.
    #[test]
    fn new_reader_matches_reference_for_every_range_shape() {
        let dir = std::env::temp_dir().join(format!("exporter-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part-0001.parquet");
        let base = 1_790_000_000_000_000_000_i64;
        let group = |from: i64, n: i64| -> Vec<(i64, f64)> {
            (0..n)
                .map(|i| (base + (from + i) * 10_000_000, 3.7 + i as f64 * 1e-4))
                .collect()
        };
        // Three row groups, the last one stepping backwards in time like a
        // clock re-anchor, plus a NaN value.
        let mut last = group(250, 40);
        last[3].1 = f64::NAN;
        write_file(
            &path,
            &[group(0, 100), group(100, 100), group(200, 100), last],
        );

        let ns = |k: i64| base + k * 10_000_000;
        for (start, end) in [
            (ns(-50), ns(1_000)), // everything
            (ns(0), ns(0)),       // single inclusive sample
            (ns(120), ns(180)),   // inside one row group
            (ns(99), ns(100)),    // across a row-group boundary
            (ns(260), ns(270)),   // overlapping, out-of-order timestamps
            (ns(500), ns(600)),   // after all data
            (ns(-100), ns(-1)),   // before all data
        ] {
            let got = read_matching_rows(&path, start, end).unwrap().rows;
            let want = reference_rows(&path, at(start), at(end));
            assert_eq!(got.len(), want.len(), "range {start}..{end}");
            for (g, w) in got.iter().zip(&want) {
                assert_eq!(g.0, w.0);
                assert!(g.1 == w.1 || (g.1.is_nan() && w.1.is_nan()));
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    /// Checks the row-group statistics filter on inclusive boundaries.
    #[test]
    fn row_groups_outside_the_range_are_skipped() {
        let dir = std::env::temp_dir().join(format!("exporter-skip-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part-0001.parquet");
        let group =
            |from: i64| -> Vec<(i64, f64)> { (from..from + 10).map(|t| (t, 1.0)).collect() };
        write_file(&path, &[group(0), group(100), group(200)]);
        let reader = SerializedFileReader::new(fs::File::open(&path).unwrap()).unwrap();
        let meta = reader.metadata();
        assert!(row_group_may_match(meta.row_group(0), 5, 105));
        assert!(row_group_may_match(meta.row_group(1), 5, 105));
        assert!(!row_group_may_match(meta.row_group(2), 5, 105));
        assert!(!row_group_may_match(meta.row_group(0), 10, 99));
        assert!(row_group_may_match(meta.row_group(0), 9, 9));
        fs::remove_dir_all(dir).unwrap();
    }

    /// Checks that `writeln!` rows match the earlier `format!` rows byte for byte.
    #[test]
    fn csv_lines_are_byte_identical_to_the_previous_format() {
        let ts = timestamp_unix_ns_to_rfc3339(1_790_000_000_008_000_000);
        for (raw, cal, id) in [
            (3.7215, 252.1, "tp3505"),
            (-0.0546, f64::NAN, "default"),
            (0.0, -8.3148, ""),
        ] {
            let old = format!("{ts},ch{:02},{raw},{cal},{id}\n", 8);
            let mut new = Vec::new();
            writeln!(new, "{ts},ch{:02},{raw},{cal},{id}", 8).unwrap();
            assert_eq!(new, old.as_bytes());
        }
        assert_eq!(ts, "2026-09-21T14:13:20.008+00:00");
    }

    /// Checks [`Rfc3339Formatter`] against chrono on edge cases and random values.
    #[test]
    fn fast_formatter_matches_chrono_exactly() {
        let mut f = Rfc3339Formatter::new();
        let mut check =
            |ns: i64| assert_eq!(f.format(ns), timestamp_unix_ns_to_rfc3339(ns), "ns={ns}");
        for ns in [
            0,
            1,
            999,
            1_000,
            1_000_000,
            999_999_999,
            1_000_000_000,
            -1,
            -1_000,
            -1_000_000,
            -999_999_999,
            -1_000_000_000,
            -1_000_000_001,
            i64::MIN,
            i64::MAX,
            1_790_000_000_000_000_000,
            1_790_000_000_008_000_000,
            1_790_000_000_008_123_000,
            1_790_000_000_008_123_456,
            1_790_000_000_010_000_000,
            1_790_000_000_500_000_000,
        ] {
            check(ns);
        }
        // Consecutive samples at 2 kHz and 100 Hz across second boundaries.
        for k in 0..5_000_i64 {
            check(1_790_000_000_000_000_000 + k * 500_000);
            check(1_789_999_999_478_000_000 + k * 10_000_000);
        }
        // Pseudo-random timestamps across the whole i64 range, in random order.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..1_000_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            check(x as i64);
        }
    }

    /// Checks nanosecond conversion and clamping of out-of-range instants.
    #[test]
    fn range_bounds_convert_to_nanoseconds_and_clamp() {
        assert_eq!(
            datetime_to_unix_ns(at(1_790_000_000_008_000_000)),
            1_790_000_000_008_000_000
        );
        let far = DateTime::parse_from_rfc3339("3000-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(datetime_to_unix_ns(far), i64::MAX);
        let early = DateTime::parse_from_rfc3339("1000-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(datetime_to_unix_ns(early), i64::MIN);
    }

    /// Compares old and new readers on real archive files for speed and exact
    /// equality. Run with `AVENA_EXPORT_BENCH_DIR=<dir of .parquet files>`.
    #[test]
    #[ignore = "benchmark on real files; set AVENA_EXPORT_BENCH_DIR"]
    fn benchmark_against_reference_on_real_files() {
        let Ok(dir) = std::env::var("AVENA_EXPORT_BENCH_DIR") else {
            return;
        };
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "parquet"))
            .collect();
        files.sort();
        let (start, end) = (at(i64::MIN / 2), at(i64::MAX / 2));
        let (mut t_old, mut t_new, mut rows) = (0.0, 0.0, 0usize);
        for path in &files {
            let t = Instant::now();
            let want = reference_rows(path, start, end);
            t_old += t.elapsed().as_secs_f64();
            let t = Instant::now();
            let got =
                read_matching_rows(path, datetime_to_unix_ns(start), datetime_to_unix_ns(end))
                    .unwrap()
                    .rows;
            t_new += t.elapsed().as_secs_f64();
            assert_eq!(got.len(), want.len(), "{}", path.display());
            assert!(
                got.iter()
                    .zip(&want)
                    .all(|(g, w)| g.0 == w.0 && (g.1 == w.1 || (g.1.is_nan() && w.1.is_nan())))
            );
            rows += got.len();
        }
        println!(
            "{} files, {rows} rows: reading: reference {t_old:.3} s, new {t_new:.3} s ({:.1}x)",
            files.len(),
            t_old / t_new
        );

        // End to end: rows to CSV bytes, the old way and the new way.
        let (mut e_old, mut e_new) = (0.0, 0.0);
        for path in &files {
            let t = Instant::now();
            let mut old_csv = Vec::new();
            for (ts_ns, raw) in reference_rows(path, start, end) {
                let ts = DateTime::<Utc>::from_timestamp_nanos(ts_ns).to_rfc3339();
                let line = format!("{ts},ch{:02},{raw},{raw},id\n", 8);
                old_csv.extend_from_slice(line.as_bytes());
            }
            e_old += t.elapsed().as_secs_f64();
            let t = Instant::now();
            let mut new_csv = Vec::new();
            let matched =
                read_matching_rows(path, datetime_to_unix_ns(start), datetime_to_unix_ns(end))
                    .unwrap();
            let mut formatter = Rfc3339Formatter::new();
            for (ts_ns, raw) in matched.rows {
                let ts = formatter.format(ts_ns);
                writeln!(new_csv, "{ts},ch{:02},{raw},{raw},id", 8).unwrap();
            }
            e_new += t.elapsed().as_secs_f64();
            assert_eq!(old_csv, new_csv, "CSV differs for {}", path.display());
        }
        println!(
            "end to end CSV: reference {e_old:.3} s, new {e_new:.3} s ({:.1}x)",
            e_old / e_new
        );
    }
}
