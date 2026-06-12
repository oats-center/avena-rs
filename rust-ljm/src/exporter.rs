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

use calibration::CalibrationSpec;

const DEFAULT_EXPORTER_ADDR: &str = "0.0.0.0:9001";
const DEFAULT_EXPORTER_MODE: &str = "direct";
const DEFAULT_EXPORT_SUBJECT_PREFIX: &str = "avenars.export";
const EXPORT_FRAME_HEADER: &str = "X-Avena-Export-Frame";
const EXPORT_FRAME_META: &str = "meta";
const EXPORT_FRAME_CHUNK: &str = "chunk";
const EXPORT_FRAME_SUMMARY: &str = "summary";
const EXPORT_FRAME_COMPLETE: &str = "complete";
const EXPORT_FRAME_ERROR: &str = "error";

#[derive(Clone)]
struct AppState {
    mode: ExporterMode,
    parquet_root: Arc<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExporterMode {
    Direct,
    Worker,
}

impl ExporterMode {
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
enum ExportFormat {
    Csv,
    Parquet,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ExportRequest {
    asset: u32,
    channels: Vec<u8>,
    start: String,
    end: String,
    #[serde(default = "default_format")]
    format: ExportFormat,
    download_name: Option<String>,
    #[serde(default)]
    box_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NatsExportRequest {
    job_id: String,
    response_subject: String,
    asset: u32,
    channels: Vec<u8>,
    start: String,
    end: String,
    #[serde(default = "default_format")]
    format: ExportFormat,
    download_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct MetaFrame<'a> {
    #[serde(rename = "type")]
    frame_type: &'static str,
    #[serde(rename = "fileName")]
    file_name: &'a str,
    #[serde(rename = "contentType")]
    content_type: &'a str,
}

#[derive(Debug, Serialize)]
struct SummaryFrame<'a> {
    #[serde(rename = "type")]
    frame_type: &'static str,
    #[serde(rename = "bytesSent")]
    bytes_sent: usize,
    #[serde(rename = "missingChannels")]
    missing_channels: &'a [u8],
}

#[derive(Debug, Serialize)]
struct ErrorFrame<'a> {
    #[serde(rename = "type")]
    frame_type: &'static str,
    message: &'a str,
}

fn default_format() -> ExportFormat {
    ExportFormat::Csv
}

#[tokio::main]
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

fn export_subject_prefix_from_env() -> String {
    std::env::var("EXPORT_NATS_SUBJECT_PREFIX")
        .unwrap_or_else(|_| DEFAULT_EXPORT_SUBJECT_PREFIX.to_string())
        .trim()
        .to_string()
}

fn worker_box_id_from_env() -> Result<String> {
    std::env::var("EXPORT_BOX_ID")
        .or_else(|_| std::env::var("BOX_ID"))
        .map(|value| sanitize_token(&value))
        .map_err(|_| anyhow!("worker mode requires EXPORT_BOX_ID or BOX_ID"))
}

fn request_subject(prefix: &str, box_id: &str) -> String {
    format!(
        "{}.request.{}",
        prefix.trim_end_matches('.'),
        sanitize_token(box_id)
    )
}

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

async fn run_worker(parquet_root: PathBuf) -> Result<()> {
    let client = connect_nats_from_env().await?;
    let subject_prefix = export_subject_prefix_from_env();
    let box_id = worker_box_id_from_env()?;
    let subject = request_subject(&subject_prefix, &box_id);
    let mut subscriber = client
        .subscribe(subject.clone())
        .await
        .map_err(|e| anyhow!("failed to subscribe to export worker subject '{subject}': {e}"))?;

    println!("[exporter] worker listening on NATS subject '{subject}'");

    while let Some(message) = subscriber.next().await {
        let client = client.clone();
        let parquet_root = parquet_root.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_worker_request(client, parquet_root, message).await {
                eprintln!("[exporter] worker request failed: {err:#}");
            }
        });
    }

    Ok(())
}

async fn handle_worker_request(
    client: async_nats::Client,
    parquet_root: PathBuf,
    message: async_nats::Message,
) -> Result<()> {
    let req: NatsExportRequest = serde_json::from_slice(&message.payload)
        .map_err(|e| anyhow!("invalid export request payload: {e}"))?;
    let export_req = ExportRequest {
        asset: req.asset,
        channels: req.channels,
        start: req.start,
        end: req.end,
        format: req.format,
        download_name: req.download_name,
        box_id: None,
    };

    let mut sink = NatsReplySink::new(client, req.response_subject.clone());
    if let Err(err) = serve_export_request(&parquet_root, &mut sink, &export_req).await {
        eprintln!(
            "[exporter] job {} failed for response subject {}: {err:#}",
            req.job_id, req.response_subject
        );
        sink.send_error(&err.to_string()).await.ok();
        sink.send_complete().await.ok();
    }

    Ok(())
}

async fn handle_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = process_socket(socket, state).await {
            eprintln!("[exporter] websocket error: {err:#}");
        }
    })
}

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
trait ExportSink {
    async fn send_meta(&mut self, file_name: &str, content_type: &str) -> Result<()>;
    async fn send_chunk(&mut self, data: Vec<u8>) -> Result<()>;
    async fn send_summary(&mut self, bytes_sent: usize, missing_channels: &[u8]) -> Result<()>;
    async fn send_complete(&mut self) -> Result<()>;
    async fn send_error(&mut self, message: &str) -> Result<()>;
}

struct WebSocketSink {
    socket: WebSocket,
}

impl WebSocketSink {
    fn new(socket: WebSocket) -> Self {
        Self { socket }
    }

    async fn send_close(&mut self) -> Result<()> {
        self.socket.send(Message::Close(None)).await?;
        Ok(())
    }
}

#[async_trait]
impl ExportSink for WebSocketSink {
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

    async fn send_chunk(&mut self, data: Vec<u8>) -> Result<()> {
        self.socket.send(Message::Binary(data)).await?;
        Ok(())
    }

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

    async fn send_complete(&mut self) -> Result<()> {
        self.socket
            .send(Message::Text(json!({"type":"complete"}).to_string()))
            .await?;
        Ok(())
    }

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

struct NatsReplySink {
    client: async_nats::Client,
    subject: String,
}

impl NatsReplySink {
    fn new(client: async_nats::Client, subject: String) -> Self {
        Self { client, subject }
    }

    async fn publish_json<T: Serialize>(
        &self,
        frame_type: &'static str,
        payload: &T,
    ) -> Result<()> {
        let mut headers = HeaderMap::new();
        headers.insert(EXPORT_FRAME_HEADER, frame_type);
        self.client
            .publish_with_headers(
                self.subject.clone(),
                headers,
                serde_json::to_vec(payload)?.into(),
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl ExportSink for NatsReplySink {
    async fn send_meta(&mut self, file_name: &str, content_type: &str) -> Result<()> {
        self.publish_json(
            EXPORT_FRAME_META,
            &MetaFrame {
                frame_type: EXPORT_FRAME_META,
                file_name,
                content_type,
            },
        )
        .await
    }

    async fn send_chunk(&mut self, data: Vec<u8>) -> Result<()> {
        let mut headers = HeaderMap::new();
        headers.insert(EXPORT_FRAME_HEADER, EXPORT_FRAME_CHUNK);
        self.client
            .publish_with_headers(self.subject.clone(), headers, data.into())
            .await?;
        Ok(())
    }

    async fn send_summary(&mut self, bytes_sent: usize, missing_channels: &[u8]) -> Result<()> {
        self.publish_json(
            EXPORT_FRAME_SUMMARY,
            &SummaryFrame {
                frame_type: EXPORT_FRAME_SUMMARY,
                bytes_sent,
                missing_channels,
            },
        )
        .await
    }

    async fn send_complete(&mut self) -> Result<()> {
        self.publish_json(EXPORT_FRAME_COMPLETE, &json!({"type":"complete"}))
            .await
    }

    async fn send_error(&mut self, message: &str) -> Result<()> {
        self.publish_json(
            EXPORT_FRAME_ERROR,
            &ErrorFrame {
                frame_type: EXPORT_FRAME_ERROR,
                message,
            },
        )
        .await
    }
}

struct CsvStreamer<'a, S: ExportSink + Send> {
    sink: &'a mut S,
    chunk: Vec<u8>,
    bytes_sent: usize,
    asset: u32,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl<'a, S: ExportSink + Send> CsvStreamer<'a, S> {
    const CHUNK_SIZE: usize = 128 * 1024;

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

fn timestamp_unix_ns_to_rfc3339(timestamp_unix_ns: i64) -> Option<String> {
    Some(DateTime::<Utc>::from_timestamp_nanos(timestamp_unix_ns).to_rfc3339())
}

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

fn parse_range(start: &str, end: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start = DateTime::parse_from_rfc3339(start)
        .map_err(|e| anyhow!("invalid start timestamp: {e}"))?
        .with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(end)
        .map_err(|e| anyhow!("invalid end timestamp: {e}"))?
        .with_timezone(&Utc);
    Ok((start, end))
}

fn date_range(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    let mut days = Vec::new();
    let mut current = start;
    while current <= end {
        days.push(current);
        current += ChronoDuration::days(1);
    }
    days
}
