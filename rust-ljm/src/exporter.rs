//! Exports archived Parquet data as streamed CSV.
//!
//! The exporter can run as a direct WebSocket server or as a NATS worker. Both
//! modes share the same export request format and CSV scanning logic: they read
//! channel/date Parquet partitions, apply calibration metadata, and stream
//! framed CSV chunks back to the caller.

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

/// NATS header used to label each export response frame.
const EXPORT_FRAME_HEADER: &str = "Avena-Export-Frame";
/// Frame name for export metadata such as file name and content type.
const EXPORT_FRAME_META: &str = "meta";
/// Frame name for a binary CSV payload chunk.
const EXPORT_FRAME_CHUNK: &str = "chunk";
/// Frame name for final byte count and missing-channel summary.
const EXPORT_FRAME_SUMMARY: &str = "summary";
/// Frame name indicating the export stream is complete.
const EXPORT_FRAME_COMPLETE: &str = "complete";
/// Frame name for request or processing errors.
const EXPORT_FRAME_ERROR: &str = "error";
/// Default runtime mode used when `EXPORTER_MODE` is unset.
const DEFAULT_EXPORTER_MODE: &str = "worker";
/// Default WebSocket listen address for direct mode.
const DEFAULT_EXPORTER_ADDR: &str = "0.0.0.0:9001";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
/// Metadata frame sent before CSV chunks.
struct MetaFrame<'a> {
    #[serde(rename = "type")]
    frame_type: &'static str,
    file_name: &'a str,
    content_type: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
/// Summary frame sent after all CSV chunks are delivered.
struct SummaryFrame<'a> {
    #[serde(rename = "type")]
    frame_type: &'static str,
    bytes_sent: usize,
    missing_channels: &'a [u8],
}

#[derive(Serialize)]
/// Error frame sent when an export request cannot be served.
struct ErrorFrame<'a> {
    #[serde(rename = "type")]
    frame_type: &'static str,
    message: &'a str,
}

#[derive(Clone)]
/// Shared Axum/NATS worker state.
struct AppState {
    mode: ExporterMode,
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
/// Requested export format.
enum ExportFormat {
    /// Stream a generated CSV file.
    Csv,
    /// Placeholder for future Parquet passthrough support.
    Parquet,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
/// Export request accepted by both WebSocket and NATS worker modes.
///
/// The time range is expressed as RFC 3339 strings. `ack_subject` is used only
/// by NATS worker mode to apply simple receiver-driven backpressure.
struct ExportRequest {
    asset: u32,
    channels: Vec<u8>,
    start: String,
    end: String,
    #[serde(default = "default_format")]
    format: ExportFormat,
    download_name: Option<String>,
    ack_subject: Option<String>,
}

/// Default export format used when a request omits `format`.
fn default_format() -> ExportFormat {
    ExportFormat::Csv
}

#[tokio::main]
/// Starts the exporter in direct WebSocket mode or NATS worker mode.
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
fn worker_box_id_from_env() -> Result<String> {
    std::env::var("EXPORT_BOX_ID")
        .or_else(|_| std::env::var("BOX_ID"))
        .map(|value| sanitize_token(&value))
        .map_err(|_| anyhow!("worker mode requires EXPORT_BOX_ID or BOX_ID"))
}

/// Normalizes namespace text into a subject-safe token.
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
/// Each incoming request is handled in its own task and replies to the message
/// reply subject with JSON control frames and binary CSV chunks.
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
async fn handle_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = process_socket(socket, state).await {
            eprintln!("[exporter] websocket error: {err:#}");
        }
    })
}

/// Handles one direct-mode WebSocket export session.
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
trait ExportSink {
    /// Sends the file metadata frame.
    async fn send_meta(&mut self, file_name: &str, content_type: &str) -> Result<()>;
    /// Sends one CSV payload chunk.
    async fn send_chunk(&mut self, data: Vec<u8>) -> Result<()>;
    /// Sends the final export summary.
    async fn send_summary(&mut self, bytes_sent: usize, missing_channels: &[u8]) -> Result<()>;
    /// Sends the completion frame.
    async fn send_complete(&mut self) -> Result<()>;
    /// Sends an error frame.
    async fn send_error(&mut self, message: &str) -> Result<()>;
}

/// WebSocket implementation of [`ExportSink`].
struct WebSocketSink {
    socket: WebSocket,
}

impl WebSocketSink {
    /// Wraps a WebSocket as an export sink.
    fn new(socket: WebSocket) -> Self {
        Self { socket }
    }

    /// Sends a WebSocket close frame.
    async fn send_close(&mut self) -> Result<()> {
        self.socket.send(Message::Close(None)).await?;
        Ok(())
    }
}

#[async_trait]
impl ExportSink for WebSocketSink {
    /// Sends metadata as a text JSON WebSocket message.
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

    /// Sends byte count and missing channels as a text JSON message.
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

    /// Sends a text JSON completion message.
    async fn send_complete(&mut self) -> Result<()> {
        self.socket
            .send(Message::Text(json!({"type":"complete"}).to_string()))
            .await?;
        Ok(())
    }

    /// Sends a text JSON error message.
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
struct CsvStreamer<'a, S: ExportSink + Send> {
    sink: &'a mut S,
    chunk: Vec<u8>,
    bytes_sent: usize,
    asset: u32,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

/// CSV stream builder for NATS worker exports.
///
/// This variant uses larger chunks and optional acknowledgement subjects to
/// avoid flooding slower clients while sending through NATS.
struct NatsCsvStreamer {
    client: async_nats::Client,
    reply: async_nats::Subject,
    ack_sub: Option<async_nats::Subscriber>,
    chunk: Vec<u8>,
    chunks_since_flush: usize,
    chunks_since_ack: usize,
    bytes_sent: usize,
    asset: u32,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl NatsCsvStreamer {
    /// Target chunk size for NATS CSV frames.
    const CHUNK_SIZE: usize = 512 * 1024;
    /// Number of chunks to publish before forcing a NATS flush.
    const FLUSH_EVERY_CHUNKS: usize = 8;
    /// Maximum time to wait for a client chunk acknowledgement.
    const ACK_TIMEOUT_SECS: u64 = 30;

    /// Creates a NATS CSV streamer and writes the CSV header into the buffer.
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

    /// Streams all requested channels and returns channels with no matching rows.
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

    /// Publishes the current CSV buffer as one NATS chunk frame.
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
    /// Target chunk size for direct WebSocket CSV frames.
    const CHUNK_SIZE: usize = 128 * 1024;

    /// Creates a direct CSV streamer and writes the CSV header into the buffer.
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

    /// Streams all requested channels and returns channels with no matching rows.
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

    /// Sends the current CSV buffer through the sink.
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
struct Rfc3339Formatter {
    second: Option<i64>,
    prefix: String,
    buf: String,
}

impl Rfc3339Formatter {
    fn new() -> Self {
        Self {
            second: None,
            prefix: String::new(),
            buf: String::with_capacity(40),
        }
    }

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
    calibration: CalibrationSpec,
    calibration_id: String,
    rows: Vec<(i64, f64)>,
}

/// Returns whether a row group's timestamp statistics overlap `[start_ns, end_ns]`.
///
/// Row groups without statistics are always read.
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
/// column readers instead of building a generic row object per sample.
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

    fn at(ns: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_nanos(ns)
    }

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
