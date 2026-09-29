//! Serves archived Parquet sample files as streamed CSV exports.
//!
//! The streamer publishes LabJack samples to the local NATS JetStream, and the
//! archiver writes them to Parquet files under
//! `<PARQUET_DIR>/assetNNN/YYYY-MM-DD/chNN/*.parquet`. This binary reads those files
//! back for a requested asset, channel list and time range, applies the calibration
//! stored in each file's metadata, and sends the rows as CSV with the columns
//! `timestamp,channel,raw_value,calibrated_value,calibration_id`. A request that
//! names noise filters for a channel ([`ExportRequest::filters`]) gets a sixth
//! column, `filtered_value`, computed by the zero-phase pipeline in [`filters`] over
//! the range plus margins; the archive itself is never changed.
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
//!   [`process_nats_request`] and [`NatsCsvStreamer`] for the exact payloads, the
//!   ack-subject backpressure and the cancel message a client can send there.
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
//! * `SOURCE_ID` - Worker mode. Source token of the request subject. When it is
//!   unset or empty the token falls back to `LABJACK_NAME`, then `asset<ASSET_NUMBER>`,
//!   then `unknown-source`, the same order the streamer and the webapp use (see
//!   `subjects::archive_export_request_subject`).
//! * `LABJACK_NAME` - Worker mode. The source's LabJack name, used only when
//!   `SOURCE_ID` is not set.
//! * `ASSET_NUMBER` - Worker mode. The source's asset number, used only when neither
//!   `SOURCE_ID` nor `LABJACK_NAME` is set.
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
//! * The archive can hold more than one copy of a sample: an archiver fed the same
//!   JetStream messages again writes them into new part files, sometimes with other
//!   window boundaries or another calibration. Each channel-day's files are grouped
//!   by overlapping time spans ([`overlapping_file_groups`]), found from the footer
//!   statistics. A file that overlaps nothing is read in one pass, as before.
//!   Overlapping files are read together in five-minute slices so memory stays
//!   bounded by one slice rather than a day. Either way the rows are sorted by time
//!   and each exact `(timestamp, raw value)` pair is sent once ([`read_merged_rows`]);
//!   a clean file in time order comes out exactly as stored. Rows with the same
//!   timestamp but different values are all kept.
//! * Rows are sent in time order, so [`Rfc3339Formatter`] builds the date
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
mod filters;
mod nats_config;
mod subjects;
#[cfg(test)]
mod test_nats;

use calibration::{CalibrationFormula, CalibrationSpec};
use filters::{ChannelFilterSettings, ExportFilter};

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
/// Frame name a client sends on its `ack_subject` to stop an export.
const EXPORT_FRAME_CANCEL: &str = "cancel";

/// Error returned when the client cancels a worker-mode export.
///
/// [`run_worker`] logs it and sends no `error` frame, since the client has stopped
/// listening.
#[derive(Debug)]
struct ExportCancelled {
    /// `chunk` frames published before the cancel was seen.
    chunks_sent: usize,
    /// CSV bytes published before the cancel was seen.
    bytes_sent: usize,
}

impl std::fmt::Display for ExportCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "export cancelled by the client after {} chunk(s), {} bytes",
            self.chunks_sent, self.bytes_sent
        )
    }
}

impl std::error::Error for ExportCancelled {}

/// Tells whether a message on the ack subject is a cancel request.
///
/// A cancel carries the header `Avena-Export-Frame: cancel`, or a JSON body whose
/// `type` is `cancel` for clients that cannot set headers. Every other message,
/// including the empty acks, is an ack.
fn is_cancel_message(message: &async_nats::Message) -> bool {
    if message
        .headers
        .as_ref()
        .and_then(|h| h.get(EXPORT_FRAME_HEADER))
        .is_some_and(|v| v.as_str() == EXPORT_FRAME_CANCEL)
    {
        return true;
    }
    !message.payload.is_empty()
        && serde_json::from_slice::<serde_json::Value>(&message.payload)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
            .is_some_and(|t| t == EXPORT_FRAME_CANCEL)
}
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
    /// Noise filters per channel, keyed by channel number as a string, in the shape
    /// of the config's `sensor_settings.filters` (see [`filters`]). When a requested
    /// channel has any filter on, every row gets a `filtered_value` column. Absent,
    /// empty or all off: the CSV is exactly as without this field.
    #[serde(default)]
    filters: Option<serde_json::Value>,
}

/// CSV header of an export without filters.
const CSV_HEADER: &[u8] = b"timestamp,channel,raw_value,calibrated_value,calibration_id\n";
/// CSV header of an export with filters.
const CSV_HEADER_FILTERED: &[u8] =
    b"timestamp,channel,raw_value,calibrated_value,calibration_id,filtered_value\n";

/// Filters asked for by an export request, per channel.
#[derive(Debug, Clone, Default)]
struct ExportFilters {
    /// Active settings of the requested channels that have any filter on.
    by_channel: std::collections::HashMap<u8, ChannelFilterSettings>,
}

impl ExportFilters {
    /// Reads the request's `filters` for its channels.
    ///
    /// # Returns
    ///
    /// `None` when no requested channel has a filter on (the export is then exactly
    /// as without filters). Keys that are not channel numbers, entries that are not
    /// objects and switches that are not `true` are ignored.
    fn from_request(req: &ExportRequest) -> Option<Self> {
        let map = req.filters.as_ref()?.as_object()?;
        let by_channel: std::collections::HashMap<u8, ChannelFilterSettings> = map
            .iter()
            .filter_map(|(key, value)| {
                let channel = key.trim().parse::<u8>().ok()?;
                let settings = ChannelFilterSettings::from_value(value);
                (req.channels.contains(&channel) && settings.is_active())
                    .then_some((channel, settings))
            })
            .collect();
        (!by_channel.is_empty()).then_some(Self { by_channel })
    }

    /// The channel's settings when it has any filter on.
    fn for_channel(&self, channel: u8) -> Option<&ChannelFilterSettings> {
        self.by_channel.get(&channel)
    }
}

/// Calibration of a row in a filtered export, shared by the rows of one file.
type SharedCalibration = Arc<(CalibrationSpec, String)>;

/// One row of a filtered export.
struct FilterInputRow {
    timestamp_unix_ns: i64,
    raw_value: f64,
    calibration: SharedCalibration,
}

/// Seconds read before the export range so the templates have learnt the
/// interference by the first exported row (their memory).
const FILTER_LEAD_IN_S: f64 = filters::TEMPLATE_MEMORY_S;

/// Reads one channel over the export range plus lead-in and settling margins and
/// runs it through the zero-phase filter pipeline, one merge slice at a time.
///
/// Rows come out in time order through [`Self::next_batch`], only those inside the
/// export range, each with its filtered calibrated value. Memory stays bounded by
/// one slice plus the filter's backward-pass block (see [`filters::ExportFilter`]).
struct FilteredChannelRows {
    /// File groups of the channel's day folders, in time order.
    groups: Vec<Vec<FileSpan>>,
    /// `(group index, from_ns, to_ns)` slices still to read.
    slices: std::collections::VecDeque<(usize, i64, i64)>,
    filter: ExportFilter<FilterInputRow>,
    start_ns: i64,
    end_ns: i64,
    finished: bool,
}

impl FilteredChannelRows {
    /// Lists the slices to read.
    ///
    /// # Errors
    ///
    /// Returns an error if an existing day folder cannot be listed.
    fn new(
        root: &Path,
        asset: u32,
        channel: u8,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        settings: &ChannelFilterSettings,
    ) -> Result<Self> {
        let start_ns = datetime_to_unix_ns(start);
        let end_ns = datetime_to_unix_ns(end);
        let lead_in = ChronoDuration::milliseconds((FILTER_LEAD_IN_S * 1000.0) as i64);
        let settle = ChronoDuration::milliseconds(
            ((filters::settle_seconds(settings) + 0.1) * 1000.0).ceil() as i64,
        );
        let (from, to) = (start - lead_in, end + settle);
        let (from_ns, to_ns) = (datetime_to_unix_ns(from), datetime_to_unix_ns(to));
        let mut groups = Vec::new();
        let mut slices = std::collections::VecDeque::new();
        for day in date_range(from.date_naive(), to.date_naive()) {
            let day_dir = root
                .join(format!("asset{asset:03}"))
                .join(day.format("%Y-%m-%d").to_string())
                .join(format!("ch{channel:02}"));
            if !day_dir.exists() {
                continue;
            }
            for group in overlapping_file_groups(&day_dir, from_ns, to_ns)? {
                for (a, b) in merge_slices(&group, from_ns, to_ns) {
                    slices.push_back((groups.len(), a, b));
                }
                groups.push(group);
            }
        }
        Ok(Self {
            groups,
            slices,
            filter: ExportFilter::new(settings.clone()),
            start_ns,
            end_ns,
            finished: false,
        })
    }

    /// Reads the next slice and returns the rows it completes; `None` when done.
    fn next_batch(&mut self) -> Option<Vec<filters::FilteredRow<FilterInputRow>>> {
        let calibrate = |row: &FilterInputRow, value: f64| row.calibration.0.apply(value);
        let mut out = Vec::new();
        if let Some((group, from_ns, to_ns)) = self.slices.pop_front() {
            let merged = read_merged_rows(&self.groups[group], from_ns, to_ns);
            let calibrations: Vec<SharedCalibration> =
                merged.calibrations.into_iter().map(Arc::new).collect();
            for (timestamp_unix_ns, raw_value, calibration) in merged.rows {
                let row = FilterInputRow {
                    timestamp_unix_ns,
                    raw_value,
                    calibration: calibrations[calibration].clone(),
                };
                self.filter
                    .push(timestamp_unix_ns, raw_value, row, &calibrate, &mut out);
            }
        } else if !self.finished {
            self.finished = true;
            self.filter.finish(&calibrate, &mut out);
        } else {
            return None;
        }
        let (start_ns, end_ns) = (self.start_ns, self.end_ns);
        out.retain(|r| (start_ns..=end_ns).contains(&r.row.timestamp_unix_ns));
        Some(out)
    }
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
/// `<NATS_SUBJECT>.<SITE_ID>.<box>.<source>.export.request` built by
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
    let source_id = std::env::var("SOURCE_ID").ok();
    let labjack_name = std::env::var("LABJACK_NAME").ok();
    let asset = std::env::var("ASSET_NUMBER")
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok());
    let subject = subjects::archive_export_request_subject(
        &nats_subject,
        asset,
        Some(&site_id),
        Some(&box_id),
        labjack_name.as_deref(),
        Some(&source_type),
        source_id.as_deref(),
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
                if let Some(cancelled) = err.downcast_ref::<ExportCancelled>() {
                    println!("[exporter] {cancelled}; stopped the export on {reply}");
                    return;
                }
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
/// subscribes to it before streaming. A cancel message on that subject stops the
/// export before the next chunk with an [`ExportCancelled`] error.
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
/// The caller ([`run_worker`]) turns these into an `error` frame, except
/// [`ExportCancelled`], which it only logs.
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
    let mut stream = NatsCsvStreamer::new(nc, reply, ack_sub, req.asset, start, end)
        .with_filters(ExportFilters::from_request(&req));
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

    let mut stream = CsvStreamer::new(sink, req.asset, start, end)
        .with_filters(ExportFilters::from_request(&req));
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
    /// Filters asked for; `None` for a plain export.
    filters: Option<ExportFilters>,
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
/// each. The content of ack messages is ignored, except that a cancel message
/// (see [`is_cancel_message`]) stops the export: messages already on the ack
/// subject are read before every chunk, so at most the chunk being published when
/// the cancel arrives still goes out. Without an ack subject the worker only
/// flushes, and nothing stops it from outrunning a slow client.
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
    /// Chunks published and not yet acknowledged by the client.
    unacked: usize,
    /// Chunks published so far.
    chunks_sent: usize,
    /// CSV bytes published so far.
    bytes_sent: usize,
    /// Asset number used to locate the Parquet partitions.
    asset: u32,
    /// Inclusive start of the export range.
    start: DateTime<Utc>,
    /// Inclusive end of the export range.
    end: DateTime<Utc>,
    /// Filters asked for; `None` for a plain export.
    filters: Option<ExportFilters>,
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
        chunk.extend_from_slice(CSV_HEADER);
        Self {
            client,
            reply,
            ack_sub,
            chunk,
            chunks_since_flush: 0,
            unacked: 0,
            chunks_sent: 0,
            bytes_sent: 0,
            asset,
            start,
            end,
            filters: None,
        }
    }

    /// Sets the filters of the export. With `Some`, the header gets a
    /// `filtered_value` column and every row a value for it.
    fn with_filters(mut self, filters: Option<ExportFilters>) -> Self {
        if filters.is_some() {
            self.chunk.clear();
            self.chunk.extend_from_slice(CSV_HEADER_FILTERED);
        }
        self.filters = filters;
        self
    }

    /// Appends one row of a filtered export: the plain columns, then the filtered
    /// value (empty when it is not finite).
    ///
    /// # Errors
    ///
    /// Returns an error if the resulting flush fails.
    async fn push_filtered_record(
        &mut self,
        timestamp: &str,
        channel: u8,
        raw_value: f64,
        calibrated_value: f64,
        calibration_id: &str,
        filtered_value: f64,
    ) -> Result<()> {
        write!(
            self.chunk,
            "{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id},"
        )?;
        if filtered_value.is_finite() {
            write!(self.chunk, "{filtered_value}")?;
        }
        self.chunk.push(b'\n');
        if self.chunk.len() >= Self::CHUNK_SIZE {
            self.flush().await?;
        }
        Ok(())
    }

    /// Streams one channel through its filters (see [`FilteredChannelRows`]).
    ///
    /// # Returns
    ///
    /// `true` if at least one row in the range was emitted.
    ///
    /// # Errors
    ///
    /// Returns an error if a day folder cannot be listed or a row cannot be sent.
    async fn stream_channel_filtered(
        &mut self,
        root: &Path,
        channel: u8,
        settings: &ChannelFilterSettings,
    ) -> Result<bool> {
        let mut rows =
            FilteredChannelRows::new(root, self.asset, channel, self.start, self.end, settings)?;
        let mut formatter = Rfc3339Formatter::new();
        let mut found = false;
        while let Some(batch) = rows.next_batch() {
            for filtered in batch {
                let row = filtered.row;
                let (spec, calibration_id) = &*row.calibration;
                let ts = formatter.format(row.timestamp_unix_ns);
                let calibrated_value = spec.apply(row.raw_value);
                found = true;
                self.push_filtered_record(
                    ts,
                    channel,
                    row.raw_value,
                    calibrated_value,
                    calibration_id,
                    filtered.filtered,
                )
                .await?;
            }
        }
        Ok(found)
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
                .map_err(|e| {
                    if e.is::<ExportCancelled>() {
                        e
                    } else {
                        anyhow!("channel {channel:02}: {e}")
                    }
                })?;
            if !found {
                missing.push(channel);
            }
        }
        Ok(missing)
    }

    /// Publishes the current CSV buffer as one `chunk` frame.
    ///
    /// Does nothing if the buffer is empty. First reads what has arrived on the ack
    /// subject ([`Self::poll_ack_subject`]), so a cancel stops the export before the
    /// next chunk. Adds the chunk to `bytes_sent`, flushes the NATS client every
    /// [`Self::FLUSH_EVERY_CHUNKS`] chunks, and calls [`Self::wait_for_acks`] once
    /// that many chunks are unacknowledged.
    ///
    /// # Errors
    ///
    /// Returns [`ExportCancelled`] if the client cancelled, or an error if the
    /// publish or flush fails, or if [`Self::wait_for_acks`] fails.
    async fn flush(&mut self) -> Result<()> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        self.poll_ack_subject()?;
        let data = std::mem::take(&mut self.chunk);
        let len = data.len();
        let mut headers = HeaderMap::new();
        headers.insert(EXPORT_FRAME_HEADER, EXPORT_FRAME_CHUNK);
        self.client
            .publish_with_headers(self.reply.clone(), headers, data.into())
            .await?;
        self.bytes_sent += len;
        self.chunks_sent += 1;
        self.chunks_since_flush += 1;
        if self.ack_sub.is_some() {
            self.unacked += 1;
        }
        if self.chunks_since_flush >= Self::FLUSH_EVERY_CHUNKS {
            self.client.flush().await?;
            self.chunks_since_flush = 0;
        }
        if self.unacked >= Self::FLUSH_EVERY_CHUNKS {
            self.wait_for_acks().await?;
        }
        self.chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        Ok(())
    }

    /// Returns the error for a cancel seen after the chunks sent so far.
    fn cancelled(&self) -> anyhow::Error {
        ExportCancelled {
            chunks_sent: self.chunks_sent,
            bytes_sent: self.bytes_sent,
        }
        .into()
    }

    /// Counts one message from the ack subject: an ack, or a cancel.
    ///
    /// # Errors
    ///
    /// Returns [`ExportCancelled`] if the message is a cancel.
    fn handle_ack_message(&mut self, message: &async_nats::Message) -> Result<()> {
        if is_cancel_message(message) {
            return Err(self.cancelled());
        }
        self.unacked = self.unacked.saturating_sub(1);
        Ok(())
    }

    /// Reads, without waiting, every message already received on the ack subject.
    ///
    /// # Errors
    ///
    /// Returns [`ExportCancelled`] if one of them is a cancel.
    fn poll_ack_subject(&mut self) -> Result<()> {
        use futures_util::FutureExt as _;
        loop {
            let Some(ack_sub) = self.ack_sub.as_mut() else {
                return Ok(());
            };
            match ack_sub.next().now_or_never() {
                Some(Some(message)) => self.handle_ack_message(&message)?,
                _ => return Ok(()),
            }
        }
    }

    /// Waits until the client has acknowledged every chunk sent so far.
    ///
    /// Returns at once if there is no ack subscription or no unacknowledged chunk.
    /// Otherwise flushes the client so the chunks actually leave, then waits for
    /// messages on the ack subject until every chunk is acknowledged, allowing
    /// [`Self::ACK_TIMEOUT_SECS`] for each.
    ///
    /// # Errors
    ///
    /// Returns [`ExportCancelled`] if a cancel arrives, or an error if the flush
    /// fails, if any ack takes longer than [`Self::ACK_TIMEOUT_SECS`], or if the ack
    /// subscription closes.
    async fn wait_for_acks(&mut self) -> Result<()> {
        if self.ack_sub.is_none() || self.unacked == 0 {
            return Ok(());
        }
        self.client.flush().await?;
        while self.unacked > 0 {
            let Some(ack_sub) = self.ack_sub.as_mut() else {
                return Ok(());
            };
            let message = tokio::time::timeout(
                std::time::Duration::from_secs(Self::ACK_TIMEOUT_SECS),
                ack_sub.next(),
            )
            .await
            .map_err(|_| anyhow!("timed out waiting for export chunk acknowledgement"))?
            .ok_or_else(|| anyhow!("export acknowledgement subscription closed"))?;
            self.handle_ack_message(&message)?;
        }
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
    /// For each UTC date from the start date to the end date inclusive, groups the
    /// `.parquet` files in `<root>/assetNNN/YYYY-MM-DD/chNN/` (skipping days whose
    /// directory does not exist) with [`overlapping_file_groups`], and sends each
    /// group's rows in time order with every exact duplicate sample sent once (see
    /// [`read_merged_rows`]). For an archive without overlapping files this is the
    /// same output as sending each file's rows in turn. A file that fails to read is
    /// logged and skipped, and the export continues.
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
    /// Returns an error if an existing day directory cannot be listed or a row cannot
    /// be sent.
    async fn stream_channel(&mut self, root: &Path, channel: u8) -> Result<bool> {
        if let Some(settings) = self
            .filters
            .as_ref()
            .and_then(|f| f.for_channel(channel))
            .cloned()
        {
            return self.stream_channel_filtered(root, channel, &settings).await;
        }
        let start_ns = datetime_to_unix_ns(self.start);
        let end_ns = datetime_to_unix_ns(self.end);
        let mut found = false;
        for day in date_range(self.start.date_naive(), self.end.date_naive()) {
            let day_dir = root
                .join(format!("asset{:03}", self.asset))
                .join(day.format("%Y-%m-%d").to_string())
                .join(format!("ch{:02}", channel));
            if !day_dir.exists() {
                continue;
            }

            for group in overlapping_file_groups(&day_dir, start_ns, end_ns)? {
                for (from_ns, to_ns) in merge_slices(&group, start_ns, end_ns) {
                    let merged = read_merged_rows(&group, from_ns, to_ns);
                    self.stream_rows(merged, channel, &mut found).await?;
                }
            }
        }
        Ok(found)
    }

    /// Emits merged rows as CSV records.
    ///
    /// Applies each row's calibration to its raw value and formats timestamps with a
    /// fresh [`Rfc3339Formatter`].
    ///
    /// # Arguments
    ///
    /// * `merged` - Rows from [`read_merged_rows`].
    /// * `channel` - LabJack channel number written in each row.
    /// * `found` - Set to `true` when at least one row is emitted; never reset.
    ///
    /// # Errors
    ///
    /// Returns an error if a row cannot be sent.
    async fn stream_rows(
        &mut self,
        merged: MergedRows,
        channel: u8,
        found: &mut bool,
    ) -> Result<()> {
        let mut formatter = Rfc3339Formatter::new();
        for (timestamp_unix_ns, raw_value, calibration) in merged.rows {
            let (spec, calibration_id) = &merged.calibrations[calibration];
            let ts = formatter.format(timestamp_unix_ns);
            let calibrated_value = spec.apply(raw_value);
            *found = true;
            if self.filters.is_some() {
                // A channel without filters in a filtered export: its filtered value is
                // its calibrated value.
                self.push_filtered_record(
                    ts,
                    channel,
                    raw_value,
                    calibrated_value,
                    calibration_id,
                    calibrated_value,
                )
                .await?;
            } else {
                self.push_record(ts, channel, raw_value, calibrated_value, calibration_id)
                    .await?;
            }
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
        chunk.extend_from_slice(CSV_HEADER);
        Self {
            sink,
            chunk,
            bytes_sent: 0,
            asset,
            start,
            end,
            filters: None,
        }
    }

    /// Sets the filters of the export. With `Some`, the header gets a
    /// `filtered_value` column and every row a value for it.
    fn with_filters(mut self, filters: Option<ExportFilters>) -> Self {
        if filters.is_some() {
            self.chunk.clear();
            self.chunk.extend_from_slice(CSV_HEADER_FILTERED);
        }
        self.filters = filters;
        self
    }

    /// Appends one row of a filtered export: the plain columns, then the filtered
    /// value (empty when it is not finite).
    ///
    /// # Errors
    ///
    /// Returns an error if the resulting flush fails.
    async fn push_filtered_record(
        &mut self,
        timestamp: &str,
        channel: u8,
        raw_value: f64,
        calibrated_value: f64,
        calibration_id: &str,
        filtered_value: f64,
    ) -> Result<()> {
        write!(
            self.chunk,
            "{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id},"
        )?;
        if filtered_value.is_finite() {
            write!(self.chunk, "{filtered_value}")?;
        }
        self.chunk.push(b'\n');
        if self.chunk.len() >= Self::CHUNK_SIZE {
            self.flush().await?;
        }
        Ok(())
    }

    /// Streams one channel through its filters (see [`FilteredChannelRows`]).
    ///
    /// # Returns
    ///
    /// `true` if at least one row in the range was emitted.
    ///
    /// # Errors
    ///
    /// Returns an error if a day folder cannot be listed or a row cannot be sent.
    async fn stream_channel_filtered(
        &mut self,
        root: &Path,
        channel: u8,
        settings: &ChannelFilterSettings,
    ) -> Result<bool> {
        let mut rows =
            FilteredChannelRows::new(root, self.asset, channel, self.start, self.end, settings)?;
        let mut formatter = Rfc3339Formatter::new();
        let mut found = false;
        while let Some(batch) = rows.next_batch() {
            for filtered in batch {
                let row = filtered.row;
                let (spec, calibration_id) = &*row.calibration;
                let ts = formatter.format(row.timestamp_unix_ns);
                let calibrated_value = spec.apply(row.raw_value);
                found = true;
                self.push_filtered_record(
                    ts,
                    channel,
                    row.raw_value,
                    calibrated_value,
                    calibration_id,
                    filtered.filtered,
                )
                .await?;
            }
        }
        Ok(found)
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
    /// For each UTC date from the start date to the end date inclusive, groups the
    /// `.parquet` files in `<root>/assetNNN/YYYY-MM-DD/chNN/` (skipping days whose
    /// directory does not exist) with [`overlapping_file_groups`], and sends each
    /// group's rows in time order with every exact duplicate sample sent once (see
    /// [`read_merged_rows`]). For an archive without overlapping files this is the
    /// same output as sending each file's rows in turn. A file that fails to read is
    /// logged and skipped, and the export continues.
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
    /// Returns an error if an existing day directory cannot be listed or a row cannot
    /// be sent.
    async fn stream_channel(&mut self, root: &Path, channel: u8) -> Result<bool> {
        if let Some(settings) = self
            .filters
            .as_ref()
            .and_then(|f| f.for_channel(channel))
            .cloned()
        {
            return self.stream_channel_filtered(root, channel, &settings).await;
        }
        let start_ns = datetime_to_unix_ns(self.start);
        let end_ns = datetime_to_unix_ns(self.end);
        let mut found = false;
        for day in date_range(self.start.date_naive(), self.end.date_naive()) {
            let day_dir = root
                .join(format!("asset{:03}", self.asset))
                .join(day.format("%Y-%m-%d").to_string())
                .join(format!("ch{:02}", channel));
            if !day_dir.exists() {
                continue;
            }

            for group in overlapping_file_groups(&day_dir, start_ns, end_ns)? {
                for (from_ns, to_ns) in merge_slices(&group, start_ns, end_ns) {
                    let merged = read_merged_rows(&group, from_ns, to_ns);
                    self.stream_rows(merged, channel, &mut found).await?;
                }
            }
        }
        Ok(found)
    }

    /// Emits merged rows as CSV records.
    ///
    /// Applies each row's calibration to its raw value and formats timestamps with a
    /// fresh [`Rfc3339Formatter`].
    ///
    /// # Arguments
    ///
    /// * `merged` - Rows from [`read_merged_rows`].
    /// * `channel` - LabJack channel number written in each row.
    /// * `found` - Set to `true` when at least one row is emitted; never reset.
    ///
    /// # Errors
    ///
    /// Returns an error if a row cannot be sent.
    async fn stream_rows(
        &mut self,
        merged: MergedRows,
        channel: u8,
        found: &mut bool,
    ) -> Result<()> {
        let mut formatter = Rfc3339Formatter::new();
        for (timestamp_unix_ns, raw_value, calibration) in merged.rows {
            let (spec, calibration_id) = &merged.calibrations[calibration];
            let ts = formatter.format(timestamp_unix_ns);
            let calibrated_value = spec.apply(raw_value);
            *found = true;
            if self.filters.is_some() {
                // A channel without filters in a filtered export: its filtered value is
                // its calibrated value.
                self.push_filtered_record(
                    ts,
                    channel,
                    raw_value,
                    calibrated_value,
                    calibration_id,
                    calibrated_value,
                )
                .await?;
            } else {
                self.push_record(ts, channel, raw_value, calibrated_value, calibration_id)
                    .await?;
            }
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

/// Length of the time slices in which overlapping files are merged (300 s).
///
/// Slices start at multiples of this length since the Unix epoch, the same windows
/// the archiver uses for its part files, so a slice of current data lines up with one
/// file. Only one slice of rows from each overlapping file is held in memory at a time.
const MERGE_SLICE_NS: i64 = 300 * 1_000_000_000;

/// Timestamp span of one archived part file, read from its footer.
struct FileSpan {
    /// Path of the part file.
    path: PathBuf,
    /// Earliest timestamp in the file, Unix nanoseconds.
    min_ns: i64,
    /// Latest timestamp in the file, Unix nanoseconds.
    max_ns: i64,
}

/// Rows of one or more files, sorted by time with exact duplicates removed.
struct MergedRows {
    /// Calibration and CSV calibration id of each file that contributed rows.
    calibrations: Vec<(CalibrationSpec, String)>,
    /// `(timestamp_unix_ns, raw_value, calibration index)` in timestamp order.
    rows: Vec<(i64, f64, usize)>,
}

/// Finds the earliest and latest timestamp in one archived file.
///
/// Uses the min/max statistics of the timestamp column in each row group, so
/// normally only the footer is read. A row group without usable statistics has its
/// timestamp column decoded instead.
///
/// # Arguments
///
/// * `path` - Parquet file to inspect.
///
/// # Returns
///
/// `(min_ns, max_ns)`, or `None` if the file has no rows.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or parsed, or a row group without
/// statistics cannot be read.
fn file_time_span(path: &Path) -> Result<Option<(i64, i64)>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open parquet file {}", path.display()))?;
    let reader = SerializedFileReader::new(file)
        .with_context(|| format!("failed to create reader for {}", path.display()))?;
    let mut span: Option<(i64, i64)> = None;
    let mut widen = |min: i64, max: i64| {
        span = Some(match span {
            Some((lo, hi)) => (lo.min(min), hi.max(max)),
            None => (min, max),
        });
    };
    for index in 0..reader.num_row_groups() {
        let meta = reader.metadata().row_group(index);
        if meta.num_rows() == 0 {
            continue;
        }
        let from_statistics = match meta.column(0).statistics() {
            Some(Statistics::Int64(stats)) => stats.min_opt().zip(stats.max_opt()),
            _ => None,
        };
        if let Some((min, max)) = from_statistics {
            widen(*min, *max);
            continue;
        }
        let row_group = reader.get_row_group(index)?;
        let timestamps = read_column::<Int64Type>(row_group.as_ref(), 0)?;
        if let (Some(min), Some(max)) = (timestamps.iter().min(), timestamps.iter().max()) {
            widen(*min, *max);
        }
    }
    Ok(span)
}

/// Lists one channel-day folder and groups its files by overlapping time spans.
///
/// Files are found the same way the export always has: every name ending in
/// `.parquet`. Files with no rows, or whose span lies outside `[start_ns, end_ns]`,
/// are left out. A file whose span cannot be read is logged and skipped, and the
/// export continues.
///
/// The remaining files are grouped so that files whose spans overlap (or touch at the
/// same nanosecond) share a group, directly or through other files. The aligned
/// five-minute files the archiver writes never overlap, so each is a group of its own.
/// Duplicate copies written by a replayed backlog overlap the originals and end up in
/// one group, whatever their boundaries, which is what lets [`read_merged_rows`] drop
/// the copies.
///
/// # Arguments
///
/// * `day_dir` - `<root>/assetNNN/YYYY-MM-DD/chNN/` folder.
/// * `start_ns` - Inclusive range start, Unix nanoseconds.
/// * `end_ns` - Inclusive range end, Unix nanoseconds.
///
/// # Returns
///
/// The groups in time order. Files inside a group are in file-name order.
///
/// # Errors
///
/// Returns an error if the folder cannot be listed.
fn overlapping_file_groups(
    day_dir: &Path,
    start_ns: i64,
    end_ns: i64,
) -> Result<Vec<Vec<FileSpan>>> {
    let mut spans = Vec::new();
    for entry in fs::read_dir(day_dir)?.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let is_parquet = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".parquet"))
            .unwrap_or(false);
        if !is_parquet {
            continue;
        }
        match file_time_span(&path) {
            Ok(Some((min_ns, max_ns))) if max_ns >= start_ns && min_ns <= end_ns => {
                spans.push(FileSpan {
                    path,
                    min_ns,
                    max_ns,
                });
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!("[exporter] skipping {} due to error: {err}", path.display());
            }
        }
    }
    spans.sort_by(|a, b| (a.min_ns, &a.path).cmp(&(b.min_ns, &b.path)));

    let mut groups: Vec<Vec<FileSpan>> = Vec::new();
    let mut group_max = i64::MIN;
    for span in spans {
        match groups.last_mut() {
            Some(group) if span.min_ns <= group_max => {
                group_max = group_max.max(span.max_ns);
                group.push(span);
            }
            _ => {
                group_max = span.max_ns;
                groups.push(vec![span]);
            }
        }
    }
    for group in &mut groups {
        group.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok(groups)
}

/// Splits the export range covered by one file group into merge slices.
///
/// A group of one file is read in a single pass, as before, since it holds no
/// copies of another file's rows. A group of overlapping files is read in slices
/// aligned to [`MERGE_SLICE_NS`], so memory is bounded by one slice of each file
/// instead of the whole group.
///
/// # Arguments
///
/// * `group` - Files from [`overlapping_file_groups`].
/// * `start_ns` - Inclusive range start, Unix nanoseconds.
/// * `end_ns` - Inclusive range end, Unix nanoseconds.
///
/// # Returns
///
/// Inclusive `(from_ns, to_ns)` slices in time order, together covering the part of
/// `[start_ns, end_ns]` that the group spans.
fn merge_slices(group: &[FileSpan], start_ns: i64, end_ns: i64) -> Vec<(i64, i64)> {
    let group_min = group.iter().map(|f| f.min_ns).min().unwrap_or(i64::MAX);
    let group_max = group.iter().map(|f| f.max_ns).max().unwrap_or(i64::MIN);
    let from = group_min.max(start_ns);
    let to = group_max.min(end_ns);
    if from > to {
        return Vec::new();
    }
    if group.len() == 1 {
        return vec![(from, to)];
    }

    let mut slices = Vec::new();
    let mut slice = from.div_euclid(MERGE_SLICE_NS);
    loop {
        let slice_start = slice.saturating_mul(MERGE_SLICE_NS).max(from);
        let slice_end = slice
            .saturating_add(1)
            .saturating_mul(MERGE_SLICE_NS)
            .saturating_sub(1)
            .min(to);
        slices.push((slice_start, slice_end));
        if slice_end >= to {
            break;
        }
        slice += 1;
    }
    slices
}

/// Returns how strongly a copy's calibration is preferred when copies collide.
///
/// Higher wins. The archiver writes an unnamed identity calibration
/// (`{"id":null,"type":"identity"}`) when a channel has no calibration configured,
/// so identity there means "no calibration known" rather than a deliberate choice.
/// The same raw sample has been found archived once that way and once with the
/// channel's real sensor calibration (for example `{"id":"tp3586","type":"linear",..}`),
/// when a backlog was archived again after the calibration was set. Both copies hold
/// the same raw value, so the calibrated one loses nothing and turns volts into the
/// sensor's units, while picking identity would report volts as if they were
/// calibrated and make the calibrated column jump between copies. Between two
/// non-identity calibrations there is no such rule, and the copy from the earliest
/// file (by name, which is the order the archiver wrote them) is kept.
///
/// # Arguments
///
/// * `spec` - Calibration of one copy.
fn calibration_preference(spec: &CalibrationSpec) -> u8 {
    match spec.formula {
        CalibrationFormula::Identity => 0,
        _ => 1,
    }
}

/// Sorts rows by timestamp and removes exact duplicates.
///
/// Two rows are duplicates when their timestamps are equal and their values have the
/// same bit pattern (`f64::to_bits`), so NaN copies collapse too. Rows with the same
/// timestamp but different values are all kept, in their original order. The sort
/// is stable and skipped when the rows are already in order, so rows of a single
/// clean file come out exactly as stored.
///
/// When duplicates carry different calibrations, the kept row takes the most
/// preferred one (see [`calibration_preference`]); on a tie the earliest row's
/// calibration stays.
///
/// # Arguments
///
/// * `rows` - `(timestamp_unix_ns, raw_value, calibration index)` rows, in file
///   order; replaced by the result.
/// * `calibrations` - Calibrations the indices refer to.
fn sort_and_drop_duplicates(
    rows: &mut Vec<(i64, f64, usize)>,
    calibrations: &[(CalibrationSpec, String)],
) {
    if !rows.is_sorted_by_key(|row| row.0) {
        rows.sort_by_key(|row| row.0);
    }
    // The usual case, a clean file: no timestamp repeats, so nothing to drop.
    if rows.windows(2).all(|pair| pair[0].0 != pair[1].0) {
        return;
    }
    let preference = |index: usize| calibration_preference(&calibrations[index].0);

    let mut kept: Vec<(i64, f64, usize)> = Vec::with_capacity(rows.len());
    let mut run_start = 0;
    while run_start < rows.len() {
        let timestamp = rows[run_start].0;
        let mut run_end = run_start + 1;
        while run_end < rows.len() && rows[run_end].0 == timestamp {
            run_end += 1;
        }
        let first_kept = kept.len();
        for &(ts, value, calibration) in &rows[run_start..run_end] {
            // Runs are a handful of copies at most, so a linear search is enough.
            match kept[first_kept..]
                .iter_mut()
                .find(|row| row.1.to_bits() == value.to_bits())
            {
                Some(row) => {
                    if preference(calibration) > preference(row.2) {
                        row.2 = calibration;
                    }
                }
                None => kept.push((ts, value, calibration)),
            }
        }
        run_start = run_end;
    }
    *rows = kept;
}

/// Reads one slice of a file group, merged into time order without duplicates.
///
/// Each file whose span overlaps the slice is read with [`read_matching_rows`], and
/// the rows are combined with [`sort_and_drop_duplicates`]. A file that fails to read
/// is logged and skipped, and the other files are still used.
///
/// # Arguments
///
/// * `group` - Files from [`overlapping_file_groups`], in file-name order.
/// * `from_ns` - Inclusive slice start, Unix nanoseconds.
/// * `to_ns` - Inclusive slice end, Unix nanoseconds.
fn read_merged_rows(group: &[FileSpan], from_ns: i64, to_ns: i64) -> MergedRows {
    let mut calibrations = Vec::new();
    let mut rows = Vec::new();
    for file in group {
        if file.max_ns < from_ns || file.min_ns > to_ns {
            continue;
        }
        match read_matching_rows(&file.path, from_ns, to_ns) {
            Ok(matched) => {
                let index = calibrations.len();
                calibrations.push((matched.calibration, matched.calibration_id));
                rows.extend(matched.rows.into_iter().map(|(ts, v)| (ts, v, index)));
            }
            Err(err) => {
                eprintln!(
                    "[exporter] skipping {} due to error: {err}",
                    file.path.display()
                );
            }
        }
    }
    sort_and_drop_duplicates(&mut rows, &calibrations);
    MergedRows { calibrations, rows }
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
        file::{metadata::KeyValue, properties::WriterProperties, writer::SerializedFileWriter},
        record::RowAccessor,
        schema::parser::parse_message_type,
    };
    use std::time::Instant;

    /// What a test client saw of one worker-mode export.
    struct ExportRun {
        /// Frame names in arrival order.
        frames: Vec<String>,
        /// `chunk` frames received after the cancel was sent.
        chunks_after_cancel: usize,
        /// What `process_nats_request` returned.
        result: Result<()>,
        /// Time from the cancel (or the request) until the exporter returned.
        elapsed: std::time::Duration,
    }

    /// How the test client ends an export.
    #[derive(Clone, Copy)]
    enum ClientEnd {
        /// Acks every chunk, like a client without cancel support.
        AckAll,
        /// Acks the first `n` chunks, then sends a cancel with the header, while
        /// the exporter keeps streaming.
        CancelAfter(usize),
        /// Acks the first eight chunks, lets the exporter block waiting for the next
        /// eight acks, then sends a cancel as a JSON body without the header.
        CancelWhileBlocked,
    }

    /// Runs one worker-mode export of about 20 chunks against a real NATS server.
    async fn run_worker_export(server: &crate::test_nats::TestNats, end: ClientEnd) -> ExportRun {
        let root = std::env::temp_dir().join(format!("exporter-cancel-{}", uuid::Uuid::new_v4()));
        let dir = root
            .join("asset1001")
            .join(at(DAY_START).date_naive().format("%Y-%m-%d").to_string())
            .join("ch08");
        write_part(&dir, 1, &samples(0, 150_000), IDENTITY);

        let client = async_nats::connect(&server.url).await.unwrap();
        let reply = client.new_inbox();
        let ack_subject = client.new_inbox();
        let mut replies = client.subscribe(reply.clone()).await.unwrap();
        client.flush().await.unwrap();
        let request = json!({
            "asset": 1001,
            "channels": [8],
            "start": at(DAY_START).to_rfc3339(),
            "end": at(DAY_START + 86_000 * 1_000_000_000).to_rfc3339(),
            "ack_subject": ack_subject,
        });
        let state = AppState {
            mode: ExporterMode::Worker,
            parquet_root: Arc::new(root.clone()),
        };
        let worker = tokio::spawn(process_nats_request(
            client.clone(),
            reply.into(),
            serde_json::to_vec(&request).unwrap(),
            state,
        ));

        let mut frames = Vec::new();
        let mut chunks = 0usize;
        let mut cancelled_at: Option<Instant> = None;
        let mut chunks_after_cancel = 0usize;
        let started = Instant::now();
        let send_cancel = |header: bool| {
            let client = client.clone();
            let subject = ack_subject.clone();
            async move {
                if header {
                    let mut headers = HeaderMap::new();
                    headers.insert(EXPORT_FRAME_HEADER, EXPORT_FRAME_CANCEL);
                    client
                        .publish_with_headers(subject, headers, "".into())
                        .await
                        .unwrap();
                } else {
                    client
                        .publish(subject, r#"{"type":"cancel"}"#.into())
                        .await
                        .unwrap();
                }
                client.flush().await.unwrap();
            }
        };
        loop {
            let next =
                tokio::time::timeout(std::time::Duration::from_secs(2), replies.next()).await;
            let Ok(Some(message)) = next else {
                // Quiet for 2 s: the exporter is blocked on acks or done.
                if let (ClientEnd::CancelWhileBlocked, None) = (end, cancelled_at) {
                    send_cancel(false).await;
                    cancelled_at = Some(Instant::now());
                    continue;
                }
                break;
            };
            let frame = message
                .headers
                .as_ref()
                .and_then(|h| h.get(EXPORT_FRAME_HEADER))
                .map(|v| v.as_str().to_string())
                .unwrap_or_default();
            frames.push(frame.clone());
            if frame == EXPORT_FRAME_COMPLETE || frame == EXPORT_FRAME_ERROR {
                break;
            }
            if frame != EXPORT_FRAME_CHUNK {
                continue;
            }
            chunks += 1;
            if cancelled_at.is_some() {
                chunks_after_cancel += 1;
                continue;
            }
            match end {
                ClientEnd::CancelAfter(n) if chunks == n => {
                    send_cancel(true).await;
                    cancelled_at = Some(Instant::now());
                }
                ClientEnd::CancelWhileBlocked if chunks > 8 => {}
                _ => {
                    client
                        .publish(ack_subject.clone(), "".into())
                        .await
                        .unwrap();
                }
            }
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), worker)
            .await
            .expect("exporter returns without waiting for its ack timeout")
            .unwrap();
        let elapsed = cancelled_at.unwrap_or(started).elapsed();
        fs::remove_dir_all(root).unwrap();
        ExportRun {
            frames,
            chunks_after_cancel,
            result,
            elapsed,
        }
    }

    /// A client without cancel support acks every chunk and gets the whole export.
    #[tokio::test]
    async fn worker_export_completes_for_a_client_that_only_acks() {
        let Some(server) = crate::test_nats::TestNats::start("export_ack_all") else {
            return;
        };
        let run = run_worker_export(&server, ClientEnd::AckAll).await;
        run.result.expect("export succeeds");
        let chunks = run.frames.iter().filter(|f| *f == "chunk").count();
        assert!(
            chunks > 16,
            "the test export spans several ack rounds: {chunks}"
        );
        assert_eq!(run.frames.first().map(String::as_str), Some("meta"));
        assert_eq!(
            &run.frames[run.frames.len() - 2..],
            ["summary".to_string(), "complete".to_string()]
        );
    }

    /// A cancel sent while the exporter is streaming stops it before its next ack
    /// wait: no summary or complete frame, and it returns without the ack timeout.
    #[tokio::test]
    async fn worker_export_stops_on_cancel_while_streaming() {
        let Some(server) = crate::test_nats::TestNats::start("export_cancel") else {
            return;
        };
        let run = run_worker_export(&server, ClientEnd::CancelAfter(3)).await;
        let err = run.result.expect_err("export is cancelled");
        let cancelled = err.downcast_ref::<ExportCancelled>().expect("cancel error");
        // Up to eight chunks may be in flight unacknowledged when the cancel is
        // sent; nothing is published after the exporter has read it.
        assert!(cancelled.chunks_sent < 3 + NatsCsvStreamer::FLUSH_EVERY_CHUNKS);
        assert!(run.chunks_after_cancel < NatsCsvStreamer::FLUSH_EVERY_CHUNKS);
        assert!(!run.frames.iter().any(|f| f == "summary" || f == "complete"));
        assert!(run.elapsed < std::time::Duration::from_secs(5));
    }

    /// A cancel (here as a JSON body without the header) ends an exporter that is
    /// blocked waiting for acks at once, with no further chunk.
    #[tokio::test]
    async fn worker_export_stops_on_cancel_while_waiting_for_acks() {
        let Some(server) = crate::test_nats::TestNats::start("export_cancel_blocked") else {
            return;
        };
        let run = run_worker_export(&server, ClientEnd::CancelWhileBlocked).await;
        let err = run.result.expect_err("export is cancelled");
        let cancelled = err.downcast_ref::<ExportCancelled>().expect("cancel error");
        assert_eq!(
            cancelled.chunks_sent,
            2 * NatsCsvStreamer::FLUSH_EVERY_CHUNKS
        );
        assert_eq!(run.chunks_after_cancel, 0);
        assert!(run.elapsed < std::time::Duration::from_secs(5));
    }

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

    /// Export sink that keeps every CSV chunk in memory.
    #[derive(Default)]
    struct CollectingSink {
        /// Chunks in the order they were sent.
        chunks: Vec<Vec<u8>>,
    }

    #[async_trait]
    impl ExportSink for CollectingSink {
        async fn send_meta(&mut self, _file_name: &str, _content_type: &str) -> Result<()> {
            Ok(())
        }
        async fn send_chunk(&mut self, data: Vec<u8>) -> Result<()> {
            self.chunks.push(data);
            Ok(())
        }
        async fn send_summary(&mut self, _bytes_sent: usize, _missing: &[u8]) -> Result<()> {
            Ok(())
        }
        async fn send_complete(&mut self) -> Result<()> {
            Ok(())
        }
        async fn send_error(&mut self, _message: &str) -> Result<()> {
            Ok(())
        }
    }

    /// Runs a direct-mode export and returns the CSV chunks it sent.
    async fn export_chunks(
        root: &Path,
        asset: u32,
        channels: &[u8],
        start: i64,
        end: i64,
    ) -> Vec<Vec<u8>> {
        let mut sink = CollectingSink::default();
        let mut streamer = CsvStreamer::new(&mut sink, asset, at(start), at(end));
        let missing = streamer.stream_channels(root, channels).await.unwrap();
        streamer.finish(missing).await.unwrap();
        sink.chunks
    }

    /// The export before duplicate removal: every file of every day in file-name
    /// order, rows as stored, each with its own file's calibration.
    fn export_without_merging(
        root: &Path,
        asset: u32,
        channels: &[u8],
        start: i64,
        end: i64,
    ) -> Vec<u8> {
        let mut csv = b"timestamp,channel,raw_value,calibrated_value,calibration_id\n".to_vec();
        for &channel in channels {
            for day in date_range(at(start).date_naive(), at(end).date_naive()) {
                let day_dir = root
                    .join(format!("asset{asset:03}"))
                    .join(day.format("%Y-%m-%d").to_string())
                    .join(format!("ch{channel:02}"));
                let Ok(entries) = fs::read_dir(&day_dir) else {
                    continue;
                };
                let mut files: Vec<PathBuf> = entries
                    .map(|e| e.unwrap().path())
                    .filter(|p| p.to_string_lossy().ends_with(".parquet"))
                    .collect();
                files.sort();
                for path in files {
                    let matched = read_matching_rows(&path, start, end).unwrap();
                    let mut formatter = Rfc3339Formatter::new();
                    for (ts, raw) in matched.rows {
                        let cal = matched.calibration.apply(raw);
                        let id = &matched.calibration_id;
                        writeln!(
                            csv,
                            "{},ch{channel:02},{raw},{cal},{id}",
                            formatter.format(ts)
                        )
                        .unwrap();
                    }
                }
            }
        }
        csv
    }

    /// Writes one part file with `calibration` metadata into a channel-day folder.
    fn write_part(dir: &Path, index: usize, rows: &[(i64, f64)], calibration: &str) {
        fs::create_dir_all(dir).unwrap();
        let schema = Arc::new(
            parse_message_type(
                "message schema { REQUIRED INT64 timestamp_unix_ns; REQUIRED DOUBLE value; }",
            )
            .unwrap(),
        );
        let props = Arc::new(
            WriterProperties::builder()
                .set_key_value_metadata(Some(vec![KeyValue::new(
                    "calibration".to_string(),
                    calibration.to_string(),
                )]))
                .build(),
        );
        let path = dir.join(format!("part-{index:04}.parquet"));
        let mut writer =
            SerializedFileWriter::new(fs::File::create(path).unwrap(), schema, props).unwrap();
        let mut rg = writer.next_row_group().unwrap();
        let ts: Vec<i64> = rows.iter().map(|(t, _)| *t).collect();
        let vs: Vec<f64> = rows.iter().map(|(_, v)| *v).collect();
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
        writer.close().unwrap();
    }

    /// Unix nanoseconds of 2026-09-21T00:00:00Z, a five-minute boundary.
    const DAY_START: i64 = 1_790_035_200_000_000_000;
    /// Sample interval of the test data (10 Hz).
    const STEP: i64 = 100_000_000;
    /// Identity calibration as the archiver writes it with nothing configured.
    const IDENTITY: &str = r#"{"id":null,"type":"identity"}"#;
    /// A real sensor calibration.
    const LINEAR: &str = r#"{"id":"tp3586","type":"linear","a":62.5,"b":-25.0}"#;

    /// `count` samples from sample index `from` of the test day.
    fn samples(from: i64, count: i64) -> Vec<(i64, f64)> {
        (from..from + count)
            .map(|i| (DAY_START + i * STEP, 0.4 + (i % 997) as f64 * 1e-4))
            .collect()
    }

    /// Without overlapping files the export is byte for byte what it was before
    /// duplicate removal, chunk boundaries included.
    #[tokio::test]
    async fn export_without_overlaps_is_unchanged() {
        let root = std::env::temp_dir().join(format!("exporter-clean-{}", uuid::Uuid::new_v4()));
        let day = |d: i64| {
            let date = at(DAY_START + d * 86_400_000_000_000).date_naive();
            root.join("asset1001")
                .join(date.format("%Y-%m-%d").to_string())
        };
        // Three aligned five-minute files on channel 8, one calibrated, plus a file
        // on the next day and one on channel 9.
        let window = 3_000; // samples per five minutes at 10 Hz
        write_part(&day(0).join("ch08"), 1, &samples(0, window), IDENTITY);
        write_part(&day(0).join("ch08"), 2, &samples(window, window), LINEAR);
        write_part(
            &day(0).join("ch08"),
            3,
            &samples(2 * window, 1_000),
            IDENTITY,
        );
        write_part(&day(1).join("ch08"), 1, &samples(864_000, 500), IDENTITY);
        write_part(&day(0).join("ch09"), 1, &samples(100, 700), LINEAR);

        for (start, end) in [
            (DAY_START - STEP, DAY_START + 2 * 86_400_000_000_000),
            (DAY_START + 1_234 * STEP, DAY_START + 7_000 * STEP),
            (DAY_START + 3_000 * STEP, DAY_START + 3_000 * STEP),
        ] {
            let chunks = export_chunks(&root, 1001, &[8, 9], start, end).await;
            let want = export_without_merging(&root, 1001, &[8, 9], start, end);
            assert!(
                want.len() > 3 * CsvStreamer::<CollectingSink>::CHUNK_SIZE || start > DAY_START
            );
            assert_eq!(chunks.concat(), want, "range {start}..{end}");
            // Chunk boundaries depend only on the bytes, so they match too.
            let sizes: Vec<usize> = chunks.iter().map(Vec::len).collect();
            let mut expected_sizes = Vec::new();
            let mut pending = 0;
            for line in want.split_inclusive(|b| *b == b'\n') {
                pending += line.len();
                if pending >= CsvStreamer::<CollectingSink>::CHUNK_SIZE {
                    expected_sizes.push(pending);
                    pending = 0;
                }
            }
            if pending > 0 {
                expected_sizes.push(pending);
            }
            assert_eq!(sizes, expected_sizes);
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// Overlapping copies with other boundaries and other calibrations are sent once,
    /// in time order, preferring the real calibration; conflicting values are kept.
    #[tokio::test]
    async fn overlapping_duplicate_files_are_exported_once() {
        let root = std::env::temp_dir().join(format!("exporter-dupes-{}", uuid::Uuid::new_v4()));
        let date = at(DAY_START).date_naive();
        let dir = root
            .join("asset1001")
            .join(date.format("%Y-%m-%d").to_string())
            .join("ch08");
        let window = 3_000;
        // Originals: two aligned windows without calibration.
        write_part(&dir, 1, &samples(0, window), IDENTITY);
        write_part(&dir, 2, &samples(window, window), IDENTITY);
        // A replayed backlog copied the second half of window 0 and the first half
        // of window 1 into one file, after the calibration was set.
        write_part(&dir, 3, &samples(1_500, window), LINEAR);
        // A second identity copy of window 1, with one sample whose value differs.
        let mut copy = samples(window, window);
        copy[10].1 = 9.75;
        write_part(&dir, 4, &copy, IDENTITY);
        // A later, separate file that overlaps nothing.
        write_part(&dir, 5, &samples(4 * window, 100), LINEAR);

        let chunks = export_chunks(
            &root,
            1001,
            &[8],
            DAY_START,
            DAY_START + 86_399 * 1_000_000_000,
        )
        .await;
        let csv = String::from_utf8(chunks.concat()).unwrap();
        let lines: Vec<&str> = csv.lines().skip(1).collect();

        let mut expected: Vec<(i64, f64, &str)> = Vec::new();
        for (ts, v) in samples(0, 2 * window) {
            let index = (ts - DAY_START) / STEP;
            let calibration = if (1_500..4_500).contains(&index) {
                LINEAR
            } else {
                IDENTITY
            };
            expected.push((ts, v, calibration));
            if index == window + 10 {
                expected.push((ts, 9.75, IDENTITY));
            }
        }
        for (ts, v) in samples(4 * window, 100) {
            expected.push((ts, v, LINEAR));
        }
        assert_eq!(lines.len(), expected.len());
        let mut formatter = Rfc3339Formatter::new();
        for (line, (ts, raw, calibration)) in lines.iter().zip(&expected) {
            let spec: CalibrationSpec = serde_json::from_str(calibration).unwrap();
            let want = format!(
                "{},ch08,{raw},{},{}",
                formatter.format(*ts),
                spec.apply(*raw),
                spec.id_or_default()
            );
            assert_eq!(*line, want);
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// Duplicate removal keeps order, conflicts and the preferred calibration.
    #[test]
    fn duplicates_are_dropped_by_timestamp_and_value_bits() {
        let calibrations = vec![
            (CalibrationSpec::default(), "identity".to_string()),
            (
                serde_json::from_str::<CalibrationSpec>(LINEAR).unwrap(),
                "tp3586".to_string(),
            ),
        ];
        let mut rows = vec![
            (30, 1.0, 0),
            (10, 1.0, 0),
            (20, 2.0, 0),
            (20, 3.0, 0),
            (10, 1.0, 1),
            (20, 2.0, 1),
            (20, f64::NAN, 0),
            (20, f64::NAN, 1),
            (30, 1.0, 0),
        ];
        sort_and_drop_duplicates(&mut rows, &calibrations);
        let got: Vec<(i64, u64, usize)> =
            rows.iter().map(|(t, v, c)| (*t, v.to_bits(), *c)).collect();
        assert_eq!(
            got,
            vec![
                (10, 1.0f64.to_bits(), 1),
                (20, 2.0f64.to_bits(), 1),
                (20, 3.0f64.to_bits(), 0),
                (20, f64::NAN.to_bits(), 1),
                (30, 1.0f64.to_bits(), 0),
            ]
        );
    }

    /// Overlapping files form one group read in aligned slices; others stay whole.
    #[test]
    fn overlapping_files_are_grouped_and_sliced() {
        let span = |name: &str, min_ns: i64, max_ns: i64| FileSpan {
            path: PathBuf::from(name),
            min_ns,
            max_ns,
        };
        let single = [span("a", DAY_START + 5, DAY_START + MERGE_SLICE_NS * 2)];
        assert_eq!(
            merge_slices(&single, i64::MIN, i64::MAX),
            vec![(DAY_START + 5, DAY_START + MERGE_SLICE_NS * 2)]
        );
        let pair = [
            span("a", DAY_START + 5, DAY_START + MERGE_SLICE_NS),
            span("b", DAY_START + 10, DAY_START + MERGE_SLICE_NS + 7),
        ];
        assert_eq!(
            merge_slices(&pair, i64::MIN, DAY_START + MERGE_SLICE_NS + 3),
            vec![
                (DAY_START + 5, DAY_START + MERGE_SLICE_NS - 1),
                (DAY_START + MERGE_SLICE_NS, DAY_START + MERGE_SLICE_NS + 3),
            ]
        );

        let root = std::env::temp_dir().join(format!("exporter-groups-{}", uuid::Uuid::new_v4()));
        write_part(&root, 1, &samples(0, 3_000), IDENTITY);
        write_part(&root, 2, &samples(3_000, 3_000), IDENTITY);
        write_part(&root, 3, &samples(2_999, 2), IDENTITY);
        write_part(&root, 4, &samples(9_000, 10), IDENTITY);
        let groups = overlapping_file_groups(&root, i64::MIN, i64::MAX).unwrap();
        let names: Vec<Vec<String>> = groups
            .iter()
            .map(|g| {
                g.iter()
                    .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
                    .collect()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                vec![
                    "part-0001.parquet",
                    "part-0002.parquet",
                    "part-0003.parquet"
                ],
                vec!["part-0004.parquet"],
            ]
        );
        // A range that misses a file leaves it out.
        let groups = overlapping_file_groups(&root, DAY_START + 9_000 * STEP, i64::MAX).unwrap();
        assert_eq!(groups.len(), 1);
        fs::remove_dir_all(root).unwrap();
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

    /// A calibration with a unit and no id, serialized as the archiver does, is read
    /// back from Parquet metadata unchanged and labelled by its formula type.
    #[test]
    fn calibration_unit_round_trips_through_parquet_metadata() {
        let dir = std::env::temp_dir().join(format!("exporter-unit-{}", uuid::Uuid::new_v4()));
        let spec: CalibrationSpec =
            serde_json::from_str(r#"{"type":"linear","a":70.1,"b":-8.1,"unit":"kPa"}"#).unwrap();
        write_part(&dir, 1, &[(1, 1.0)], &serde_json::to_string(&spec).unwrap());
        let path = dir.join("part-0001.parquet");
        let reader = SerializedFileReader::new(fs::File::open(&path).unwrap()).unwrap();
        let read = read_calibration_from_metadata(&reader, &path);
        assert_eq!(read, spec);
        assert_eq!(read.unit.as_deref(), Some("kPa"));
        assert_eq!(read.id, None);
        let matched = read_matching_rows(&path, 0, 10).unwrap();
        assert_eq!(matched.calibration_id, "linear");
        assert_eq!(matched.rows, vec![(1, 1.0)]);
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

    /// Runs a direct-mode export with a `filters` field and returns the CSV.
    async fn export_filtered(
        root: &Path,
        channels: &[u8],
        start: i64,
        end: i64,
        filters: serde_json::Value,
    ) -> Vec<u8> {
        let req = ExportRequest {
            asset: 1001,
            channels: channels.to_vec(),
            start: at(start).to_rfc3339(),
            end: at(end).to_rfc3339(),
            format: ExportFormat::Csv,
            download_name: None,
            ack_subject: None,
            filters: Some(filters),
        };
        let mut sink = CollectingSink::default();
        serve_export_request(root, &mut sink, &req).await.unwrap();
        sink.chunks.concat()
    }

    /// 100 Hz samples from `from` for `count` samples: 0.5 V, a 10 Hz square wave of
    /// ±3 mV locked to the sample count and a 12 ms pulse of 20 mV at sample 7000.
    fn square_samples(from: i64, count: i64) -> Vec<(i64, f64)> {
        (from..from + count)
            .map(|i| {
                let square = if i % 10 < 5 { 0.003 } else { -0.003 };
                let pulse = if i == 7000 { 0.02 } else { 0.0 };
                (DAY_START + i * 10_000_000, 0.5 + square + pulse)
            })
            .collect()
    }

    /// Filters that are off, or on for channels not requested, leave the export
    /// byte for byte as without them.
    #[tokio::test]
    async fn exports_without_active_filters_are_unchanged() {
        let root = std::env::temp_dir().join(format!("exporter-nofilt-{}", uuid::Uuid::new_v4()));
        let dir = root
            .join("asset1001")
            .join(at(DAY_START).date_naive().format("%Y-%m-%d").to_string());
        write_part(&dir.join("ch08"), 1, &samples(0, 3_000), LINEAR);
        let (start, end) = (DAY_START + 100 * STEP, DAY_START + 2_000 * STEP);
        let plain = export_chunks(&root, 1001, &[8], start, end).await.concat();
        assert!(plain.starts_with(CSV_HEADER));
        for filters in [
            serde_json::json!({}),
            serde_json::json!({"8": {"despike": false, "highpass_hz": null, "lowpass_hz": 0}}),
            serde_json::json!({"9": {"remove_10hz": true}}),
            serde_json::json!("not a map"),
        ] {
            assert_eq!(
                export_filtered(&root, &[8], start, end, filters).await,
                plain
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// A filtered export keeps every plain column and adds `filtered_value`: the
    /// zero-phase pipeline over the range plus the lead-in and settling margins,
    /// and the calibrated value on channels without filters.
    #[tokio::test]
    async fn filtered_exports_add_a_filtered_value_column() {
        let root = std::env::temp_dir().join(format!("exporter-filt-{}", uuid::Uuid::new_v4()));
        let dir = root
            .join("asset1001")
            .join(at(DAY_START).date_naive().format("%Y-%m-%d").to_string());
        // 120 s at 100 Hz in files of 30 s, and channel 9 unfiltered.
        for part in 0..4 {
            write_part(
                &dir.join("ch08"),
                part,
                &square_samples(part as i64 * 3_000, 3_000),
                LINEAR,
            );
        }
        write_part(&dir.join("ch09"), 1, &samples(0, 900), IDENTITY);
        let start = DAY_START + 6_000 * 10_000_000;
        let end = DAY_START + 8_000 * 10_000_000;
        let settings = serde_json::json!({"remove_10hz": true, "highpass_hz": 1.0});
        let filtered = export_filtered(
            &root,
            &[8, 9],
            start,
            end,
            serde_json::json!({"8": settings.clone()}),
        )
        .await;
        let plain = export_chunks(&root, 1001, &[8, 9], start, end)
            .await
            .concat();

        let text = String::from_utf8(filtered).unwrap();
        let plain = String::from_utf8(plain).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next().unwrap().as_bytes(),
            CSV_HEADER_FILTERED.strip_suffix(b"\n").unwrap()
        );
        let rows: Vec<&str> = lines.collect();
        let plain_rows: Vec<&str> = plain.lines().skip(1).collect();
        assert_eq!(rows.len(), plain_rows.len());

        // The reference: the filter over exactly the rows the exporter reads.
        let settings = ChannelFilterSettings::from_value(&settings);
        let from = start - (FILTER_LEAD_IN_S * 1e9) as i64;
        let to =
            end + ((filters::settle_seconds(&settings) + 0.1) * 1000.0).ceil() as i64 * 1_000_000;
        let spec: CalibrationSpec = serde_json::from_str(LINEAR).unwrap();
        let calibrate = |_: &i64, v: f64| spec.apply(v);
        let mut filter = ExportFilter::new(settings);
        let mut out = Vec::new();
        for (ts, v) in square_samples(0, 12_000) {
            if (from..=to).contains(&ts) {
                filter.push(ts, v, ts, &calibrate, &mut out);
            }
        }
        filter.finish(&calibrate, &mut out);
        let want: std::collections::HashMap<i64, f64> =
            out.into_iter().map(|r| (r.row, r.filtered)).collect();

        let mut square_left: f64 = 0.0;
        for (row, plain_row) in rows.iter().zip(&plain_rows) {
            let (head, value) = row.rsplit_once(',').unwrap();
            assert_eq!(head, *plain_row);
            let fields: Vec<&str> = head.split(',').collect();
            let value: f64 = value.parse().unwrap();
            if fields[1] == "ch09" {
                assert_eq!(value, fields[3].parse::<f64>().unwrap());
                continue;
            }
            let ts = datetime_to_unix_ns(DateTime::parse_from_rfc3339(fields[0]).unwrap().into());
            assert_eq!(value, want[&ts], "row {row}");
            let index = (ts - DAY_START) / 10_000_000;
            if (index - 7000).abs() > 150 {
                square_left = square_left.max(value.abs());
            }
        }
        // The square wave is 0.1875 kPa after calibration; away from the pulse and
        // the high-pass tail around it the filtered rows stay near zero, and the
        // pulse is kept.
        assert!(square_left < 0.01, "square wave left: {square_left}");
        let pulse = rows
            .iter()
            .find(|r| {
                r.contains("ch08")
                    && r.starts_with(&at(DAY_START + 7000 * 10_000_000).to_rfc3339()[..22])
            })
            .unwrap();
        let pulse: f64 = pulse.rsplit_once(',').unwrap().1.parse().unwrap();
        assert!((pulse - 1.25).abs() < 0.1, "pulse {pulse}");
        fs::remove_dir_all(root).unwrap();
    }
}
