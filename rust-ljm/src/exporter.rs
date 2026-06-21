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
    file::reader::{FileReader, SerializedFileReader},
    record::RowAccessor,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
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
        let line =
            format!("{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id}\n");
        self.chunk.extend_from_slice(line.as_bytes());
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
        let file = fs::File::open(path)
            .with_context(|| format!("failed to open parquet file {}", path.display()))?;
        let reader = SerializedFileReader::new(file)
            .with_context(|| format!("failed to create reader for {}", path.display()))?;
        let calibration = read_calibration_from_metadata(&reader, path);
        let calibration_id = calibration.id_or_default().to_string();
        let mut iter = reader.get_row_iter(None)?;
        while let Some(row) = iter.next() {
            let row = row?;
            let timestamp_unix_ns = row.get_long(0)?;
            let ts = match timestamp_unix_ns_to_rfc3339(timestamp_unix_ns) {
                Some(ts) => ts,
                None => continue,
            };
            let ts_parsed = match DateTime::parse_from_rfc3339(&ts) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };
            if ts_parsed < self.start || ts_parsed > self.end {
                continue;
            }
            let raw_value = row.get_double(1)?;
            let calibrated_value = calibration.apply(raw_value);
            *found = true;
            self.push_record(&ts, channel, raw_value, calibrated_value, &calibration_id)
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
        let line =
            format!("{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id}\n");
        self.chunk.extend_from_slice(line.as_bytes());
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
        let file = fs::File::open(path)
            .with_context(|| format!("failed to open parquet file {}", path.display()))?;
        let reader = SerializedFileReader::new(file)
            .with_context(|| format!("failed to create reader for {}", path.display()))?;
        let calibration = read_calibration_from_metadata(&reader, path);
        let calibration_id = calibration.id_or_default().to_string();
        let mut iter = reader.get_row_iter(None)?;
        while let Some(row) = iter.next() {
            let row = row?;
            let timestamp_unix_ns = row.get_long(0)?;
            let ts = match timestamp_unix_ns_to_rfc3339(timestamp_unix_ns) {
                Some(ts) => ts,
                None => continue,
            };
            let ts_parsed = match DateTime::parse_from_rfc3339(&ts) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };
            if ts_parsed < self.start || ts_parsed > self.end {
                continue;
            }
            let raw_value = row.get_double(1)?;
            let calibrated_value = calibration.apply(raw_value);
            *found = true;
            self.push_record(&ts, channel, raw_value, calibrated_value, &calibration_id)
                .await?;
        }
        Ok(())
    }
}

/// Converts a Unix nanosecond timestamp into RFC 3339 text.
fn timestamp_unix_ns_to_rfc3339(timestamp_unix_ns: i64) -> Option<String> {
    Some(DateTime::<Utc>::from_timestamp_nanos(timestamp_unix_ns).to_rfc3339())
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
