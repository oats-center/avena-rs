use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
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
use serde::Deserialize;
use serde_json::json;

mod calibration;

use calibration::CalibrationSpec;

#[derive(Clone)]
struct AppState {
    parquet_root: Arc<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExportFormat {
    Csv,
    Parquet,
}

#[derive(Debug, Deserialize)]
struct ExportRequest {
    asset: u32,
    channels: Vec<u8>,
    start: String,
    end: String,
    #[serde(default = "default_format")]
    format: ExportFormat,
    download_name: Option<String>,
}

fn default_format() -> ExportFormat {
    ExportFormat::Csv
}

#[tokio::main]
async fn main() -> Result<()> {
    let listen_addr = std::env::var("EXPORTER_ADDR").unwrap_or_else(|_| "0.0.0.0:9001".into());
    let parquet_root = std::env::var("PARQUET_DIR").unwrap_or_else(|_| "parquet".into());
    let root_path = PathBuf::from(parquet_root);
    if !root_path.exists() {
        println!(
            "[exporter] Warning: parquet directory '{}' does not exist.",
            root_path.display()
        );
    }

    let state = AppState {
        parquet_root: Arc::new(root_path),
    };

    let app = Router::new()
        .route("/export", get(handle_ws))
        .with_state(state);

    println!("[exporter] Listening for WebSocket exports on ws://{listen_addr}/export");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = process_socket(socket, state).await {
            eprintln!("[exporter] websocket error: {err:?}");
        }
    })
}

async fn process_socket(mut socket: WebSocket, state: AppState) -> Result<()> {
    let Some(msg) = socket.next().await else {
        return Ok(());
    };

    let Message::Text(text) = msg? else {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"expected JSON request"}).to_string(),
            ))
            .await
            .ok();
        return Ok(());
    };

    let mut req: ExportRequest =
        serde_json::from_str(&text).map_err(|e| anyhow!("invalid request payload: {e}"))?;

    if req.channels.is_empty() {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"no channels requested"}).to_string(),
            ))
            .await
            .ok();
        return Ok(());
    }
    req.channels.sort_unstable();
    req.channels.dedup();

    let (start, end) = parse_range(&req.start, &req.end)?;
    if end < start {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"end must be after start"}).to_string(),
            ))
            .await
            .ok();
        return Ok(());
    }

    if !matches!(req.format, ExportFormat::Csv) {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"parquet streaming not yet supported"}).to_string(),
            ))
            .await
            .ok();
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

    socket
        .send(Message::Text(
            json!({
                "type": "meta",
                "fileName": file_name,
                "contentType": "text/csv"
            })
            .to_string(),
        ))
        .await?;

    let mut stream = CsvStreamer::new(socket, req.asset, start, end);
    let missing = stream
        .stream_channels(&state.parquet_root, &req.channels)
        .await?;
    stream.finish(missing).await?;
    Ok(())
}

struct CsvStreamer {
    socket: WebSocket,
    chunk: Vec<u8>,
    bytes_sent: usize,
    asset: u32,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl CsvStreamer {
    const CHUNK_SIZE: usize = 128 * 1024;

    fn new(
        socket: WebSocket,
        asset: u32,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Self {
        let mut chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        chunk.extend_from_slice(b"timestamp,channel,raw_value,calibrated_value,calibration_id\n");
        Self {
            socket,
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
        self.socket.send(Message::Binary(data)).await?;
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
        let line = format!(
            "{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id}\n"
        );
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
        let summary = json!({
            "type": "summary",
            "bytesSent": self.bytes_sent,
            "missingChannels": missing_channels
        });
        self.socket.send(Message::Text(summary.to_string())).await?;
        self.socket
            .send(Message::Text(json!({"type":"complete"}).to_string()))
            .await?;
        self.socket.close().await.ok();
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
            let ts = row.get_string(0)?;
            let ts_parsed = match DateTime::parse_from_rfc3339(ts) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };
            if ts_parsed < self.start || ts_parsed > self.end {
                continue;
            }
            let raw_value = row.get_double(1)?;
            let calibrated_value = calibration.apply(raw_value);
            *found = true;
            self.push_record(ts, channel, raw_value, calibrated_value, &calibration_id)
                .await?;
        }
        Ok(())
    }
}

fn read_calibration_from_metadata(
    reader: &SerializedFileReader<fs::File>,
    path: &Path,
) -> CalibrationSpec {
    let Some(kv) = reader
        .metadata()
        .file_metadata()
        .key_value_metadata()
    else {
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
