//! The `archiver` binary: writes LabJack scans from NATS JetStream to Parquet files.
//!
//! For every LabJack read, the streamer publishes one FlatBuffer `Scan` per channel to
//! the local NATS JetStream. The archiver reads the same dashboard configuration as the
//! streamer (from KV bucket `avenabox` by default), attaches one durable pull consumer
//! per enabled channel, rebuilds each sample's timestamp from the scan header, and writes the
//! samples to Parquet files under
//! `<PARQUET_DIR>/asset<NNN>/<YYYY-MM-DD>/ch<NN>/part-<NNNN>.parquet`. Each file carries
//! the channel's calibration as key-value metadata. The exporter later serves these files
//! as CSV over NATS.
//!
//! At startup the archiver can mirror the configuration entry from a central NATS server
//! into local KV, and it keeps mirroring updates in a background task. It then watches
//! the local KV key and starts, stops or restarts channel writers as the configuration
//! changes. Ctrl-C closes every open file and acks its messages before the process exits.
//!
//! # Configuration
//!
//! * `NATS_SERVERS` - Comma-separated local NATS server URLs. Default:
//!   `nats://127.0.0.1:4222`.
//! * `NATS_CREDS_FILE` - Credentials file for the local NATS connection. Default:
//!   `apt.creds`.
//! * `JS_DOMAIN` - Optional JetStream domain for the local connection.
//! * `PARQUET_DIR` - Root directory for Parquet output. Default: `parquet`.
//! * `CFG_BUCKET` - Local KV bucket holding the dashboard configuration. Default:
//!   `avenabox`. Also the fallback for `CENTRAL_CFG_BUCKET`.
//! * `CFG_KEY` - Local KV key holding the dashboard configuration. Default:
//!   `labjackd.config.macbook`. Also the fallback for `CENTRAL_CFG_KEY`.
//! * `CENTRAL_NATS_SERVERS` - Central NATS server URLs to mirror configuration from.
//!   Central mirroring is off when neither this nor `CFG_NATS_SERVERS` is set.
//! * `CFG_NATS_SERVERS` - Fallback for `CENTRAL_NATS_SERVERS`.
//! * `CENTRAL_CFG_BUCKET` - Central KV bucket to mirror from. Default: `CFG_BUCKET`,
//!   then `avenabox`.
//! * `CENTRAL_CFG_KEY` - Central KV key to mirror from. Default: `CFG_KEY`, then
//!   `unknown-site.macbook.unknown-source.config`.
//! * `CENTRAL_JS_DOMAIN` - Optional JetStream domain on the central server. Falls back to
//!   `CFG_JS_DOMAIN`.
//! * `CENTRAL_NATS_CREDS_FILE` - Credentials file for the central server. Default: the
//!   value used for `NATS_CREDS_FILE`.
//!
//! # Design
//!
//! * **Aligned source-time rotation.** A file covers one window of `rotate_secs` that
//!   starts at a multiple of `rotate_secs` since the Unix epoch (see [`rotation_window`]),
//!   measured in sample time rather than wall-clock time. Files line up across channels,
//!   and replaying a backlog of several hours still produces one file per window. A new
//!   file also starts at a UTC date change or when sample time goes backwards.
//! * **One row group per file.** A row group holds up to [`ROWS_PER_ROW_GROUP`] rows, which
//!   is more than a normal window, so a file is usually a single row group. Files are
//!   written with zstd compression and delta-encoded timestamps (see
//!   [`archive_format::writer_properties`]).
//! * **Acks after close and fsync.** A JetStream message is acked only after every sample
//!   in it is inside a file that has been closed, fsynced and renamed from
//!   `.parquet.inprogress` to `.parquet` (see [`ParquetLogger::close`]). If the process
//!   or machine dies first, the messages stay unacked and JetStream redelivers them once
//!   the consumer's `ack_wait` expires, so they are written again into a new file.
//! * **Early close at the pending-ack cap.** When a file holds
//!   [`MAX_PENDING_ACKS_PER_FILE`] unacked messages it is closed early, which keeps the
//!   consumer below its `max_ack_pending` limit ([`CONSUMER_MAX_ACK_PENDING`]) so delivery
//!   never stalls.
//! * **Idle close.** A file that has received no data for [`IDLE_CLOSE_AFTER`] is closed,
//!   for example after the streamer stops, so its messages are acked promptly.
//! * **Quarantine of unfinished files.** At startup every leftover `.parquet.inprogress`
//!   file (no footer, from a crash) is renamed aside and kept for diagnosis (see
//!   [`quarantine_incomplete_files`]). Its samples come back through redelivery.

use async_nats;
use async_nats::ConnectOptions;
use async_nats::jetstream::{
    self,
    consumer::pull,
    kv::{self, Operation},
};
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::StreamExt;
use parquet::{
    column::writer::ColumnWriter, file::writer::SerializedFileWriter,
    schema::parser::parse_message_type,
};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{oneshot, watch};
use tokio::time::Duration;

mod archive_format;
mod calibration;
mod nats_config;
mod subjects;
mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs");
}
use sample_data_generated::sampler;

use archive_format::{SAMPLE_SCHEMA, writer_properties_for_calibration};
use calibration::CalibrationSpec;
use serde::{Deserialize, Serialize};

/// Error type used by the archiver for fallible async setup and IO paths.
type DynError = Box<dyn std::error::Error + Send + Sync>;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
/// Raw top-level dashboard configuration loaded from NATS KV.
///
/// This is the JSON shape the dashboard writes. [`sample_config_from_nested`] turns it
/// into the archiver's [`SampleConfig`].
struct NestedConfig {
    /// LabJack device name; used as the source ID when `source_id` is absent.
    labjack_name: String,
    /// Asset number; names the `asset<NNN>` output directory.
    asset_number: u32,
    /// Maximum channel count from the dashboard. Parsed but not used by the archiver.
    max_channels: u32,
    /// Site identifier used in the structured subject namespace.
    #[serde(default)]
    site_id: Option<String>,
    /// Box identifier used in the subject and the durable consumer name.
    #[serde(default)]
    box_id: Option<String>,
    /// Source type used in the durable consumer name (`labjack` when absent).
    #[serde(default)]
    source_type: Option<String>,
    /// Source identifier used in the subject and the durable consumer name.
    #[serde(default)]
    source_id: Option<String>,
    /// Subject root, for example `avenabox` or `avenars`.
    nats_subject: String,
    /// Name of the JetStream stream holding the channel subjects.
    nats_stream: String,
    /// Length of one file rotation window, in seconds.
    rotate_secs: u64,
    /// Nested sensor settings section.
    sensor_settings: SensorConfig,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
/// Raw sensor settings section from the dashboard configuration.
///
/// The archiver uses the enabled channel list and the calibration map from this section.
/// It copies the sampling fields into [`SampleConfig`] and ignores the rest. The serde
/// aliases accept the older names `scan_rate` and `sampling_rate`.
struct SensorConfig {
    /// Scans per LabJack stream read (older name `scan_rate`).
    #[serde(rename = "scans_per_read", alias = "scan_rate")]
    scans_per_read: i32,
    /// Requested scan rate in Hz (older name `sampling_rate`).
    #[serde(rename = "scan_rate_hz", alias = "sampling_rate")]
    scan_rate_hz: f64,
    /// Channels to archive; one consumer and writer task is started per entry.
    channels_enabled: Vec<u8>,
    /// Gain setting. Parsed but not used by the archiver.
    gains: i32,
    /// Per-channel data formats. Parsed but not used by the archiver.
    data_formats: Vec<String>,
    /// Per-channel measurement units. Parsed but not used by the archiver.
    measurement_units: Vec<String>,
    /// Streamer on/off switch. Parsed but not used by the archiver.
    labjack_on_off: bool,
    /// Calibrations keyed by channel number as a string, for example `"8"`.
    calibrations: Option<HashMap<String, CalibrationSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Normalized archiver configuration used to manage channel loggers.
///
/// This shape combines stream identity, channel list, rotation cadence, and
/// parsed per-channel calibrations. `main` compares each channel's derived subject,
/// consumer name, stream, asset and `rotate_secs` against the running writer to decide
/// whether to restart it.
struct SampleConfig {
    /// Scans per LabJack stream read. Not used by the archiver.
    scans_per_read: i32,
    /// Requested scan rate in Hz. Not used by the archiver.
    scan_rate_hz: f64,
    /// Channels to archive.
    channels: Vec<u8>,
    /// Asset number; names the `asset<NNN>` output directory.
    asset_number: u32,
    /// LabJack device name; fallback source ID for subjects and consumer names.
    labjack_name: String,
    /// Site identifier for the subject namespace.
    site_id: Option<String>,
    /// Box identifier for the subject namespace and consumer names.
    box_id: Option<String>,
    /// Source type for consumer names.
    source_type: Option<String>,
    /// Source identifier for the subject namespace and consumer names.
    source_id: Option<String>,
    /// Subject root, for example `avenabox` or `avenars`.
    nats_subject: String,
    /// JetStream stream that holds the channel subjects.
    nats_stream: String,
    /// Length of one file rotation window, in seconds.
    rotate_secs: u64,
    /// Calibration per channel; channels without an entry use the default spec.
    calibrations: HashMap<u8, CalibrationSpec>,
}

impl From<(SensorConfig, &SampleConfig)> for SampleConfig {
    /// Replaces the sensor settings of a config while keeping its identity fields.
    ///
    /// Channels, sampling fields and calibrations come from the new sensor section.
    /// Asset, names, subject, stream and `rotate_secs` are copied from `base`.
    ///
    /// # Arguments
    ///
    /// * `raw` - New sensor settings section.
    /// * `base` - Existing config whose identity fields are kept.
    fn from((raw, base): (SensorConfig, &SampleConfig)) -> Self {
        let calibrations = parse_calibrations(&raw);
        SampleConfig {
            scans_per_read: raw.scans_per_read,
            scan_rate_hz: raw.scan_rate_hz,
            channels: raw.channels_enabled,
            asset_number: base.asset_number,
            labjack_name: base.labjack_name.clone(),
            site_id: base.site_id.clone(),
            box_id: base.box_id.clone(),
            source_type: base.source_type.clone(),
            source_id: base.source_id.clone(),
            nats_subject: base.nats_subject.clone(),
            nats_stream: base.nats_stream.clone(),
            rotate_secs: base.rotate_secs,
            calibrations,
        }
    }
}

/// Parses dashboard calibration map keys into numeric LabJack channels.
///
/// Invalid channel keys are ignored after logging because one malformed
/// calibration entry should not prevent unrelated channels from being archived.
///
/// # Arguments
///
/// * `raw` - Sensor settings whose `calibrations` map is read.
///
/// # Returns
///
/// Calibrations keyed by channel number. Empty when the section has no
/// `calibrations` map.
fn parse_calibrations(raw: &SensorConfig) -> HashMap<u8, CalibrationSpec> {
    let mut out = HashMap::new();
    let Some(calibrations) = raw.calibrations.as_ref() else {
        return out;
    };
    for (key, spec) in calibrations {
        match key.parse::<u8>() {
            Ok(ch) => {
                out.insert(ch, spec.clone());
            }
            Err(_) => {
                eprintln!("[logger] Invalid calibration channel key '{key}', expected u8.");
            }
        }
    }
    out
}

/// Converts the nested dashboard configuration into archiver runtime config.
///
/// Calibration keys are parsed with [`parse_calibrations`]; `max_channels` and the
/// unused sensor fields are dropped.
///
/// # Arguments
///
/// * `nested` - Configuration as deserialized from the KV entry.
fn sample_config_from_nested(nested: NestedConfig) -> SampleConfig {
    let calibrations = parse_calibrations(&nested.sensor_settings);
    let raw = nested.sensor_settings;
    SampleConfig {
        scans_per_read: raw.scans_per_read,
        scan_rate_hz: raw.scan_rate_hz,
        channels: raw.channels_enabled,
        asset_number: nested.asset_number,
        labjack_name: nested.labjack_name,
        site_id: nested.site_id,
        box_id: nested.box_id,
        source_type: nested.source_type,
        source_id: nested.source_id,
        nats_subject: nested.nats_subject,
        nats_stream: nested.nats_stream,
        rotate_secs: nested.rotate_secs,
        calibrations,
    }
}

/// Returns a trimmed environment variable value when it is set and non-empty.
///
/// # Arguments
///
/// * `name` - Environment variable name.
///
/// # Returns
///
/// `None` when the variable is unset, not valid Unicode, or only whitespace.
fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone)]
/// Connection and key details for mirroring dashboard config from central NATS.
struct CentralKvSyncConfig {
    /// Central NATS server addresses.
    servers: Vec<async_nats::ServerAddr>,
    /// Credentials file for the central connection.
    creds_path: String,
    /// Central KV bucket holding the configuration.
    bucket: String,
    /// Central KV key holding the configuration.
    key: String,
    /// Optional JetStream domain on the central server.
    domain: Option<String>,
}

/// Builds optional central KV mirroring settings from environment variables.
///
/// Reads `CENTRAL_NATS_SERVERS` (or `CFG_NATS_SERVERS`), `CENTRAL_CFG_BUCKET` (or
/// `CFG_BUCKET`, default `avenabox`), `CENTRAL_CFG_KEY` (or `CFG_KEY`, default
/// `unknown-site.macbook.unknown-source.config`), `CENTRAL_JS_DOMAIN` (or
/// `CFG_JS_DOMAIN`) and `CENTRAL_NATS_CREDS_FILE`.
///
/// # Arguments
///
/// * `creds_path` - Local credentials file, used when `CENTRAL_NATS_CREDS_FILE` is unset.
///
/// # Returns
///
/// `None` when no central server list is configured, which turns mirroring off.
///
/// # Errors
///
/// Returns an error if the server list contains an entry that is not a valid NATS
/// server address, or no usable entry at all.
fn central_kv_sync_config_from_env(
    creds_path: &str,
) -> Result<Option<CentralKvSyncConfig>, DynError> {
    let Some(raw_servers) =
        env_nonempty("CENTRAL_NATS_SERVERS").or_else(|| env_nonempty("CFG_NATS_SERVERS"))
    else {
        return Ok(None);
    };

    let servers = nats_config::servers_from_env_var("CENTRAL_NATS_SERVERS", &raw_servers)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let bucket = env_nonempty("CENTRAL_CFG_BUCKET")
        .or_else(|| env_nonempty("CFG_BUCKET"))
        .unwrap_or_else(|| "avenabox".to_string());
    let key = env_nonempty("CENTRAL_CFG_KEY")
        .or_else(|| env_nonempty("CFG_KEY"))
        .unwrap_or_else(|| "unknown-site.macbook.unknown-source.config".to_string());
    let domain = env_nonempty("CENTRAL_JS_DOMAIN").or_else(|| env_nonempty("CFG_JS_DOMAIN"));
    let creds_path =
        env_nonempty("CENTRAL_NATS_CREDS_FILE").unwrap_or_else(|| creds_path.to_string());

    Ok(Some(CentralKvSyncConfig {
        servers,
        creds_path,
        bucket,
        key,
        domain,
    }))
}

/// Connects to NATS with a credentials file and explicit server list.
///
/// # Arguments
///
/// * `servers` - Server addresses to connect to.
/// * `creds_path` - Path of the NATS credentials file.
///
/// # Errors
///
/// Returns an error if the credentials file cannot be loaded or the connection fails.
async fn connect_nats_with_creds(
    servers: Vec<async_nats::ServerAddr>,
    creds_path: String,
) -> Result<async_nats::Client, DynError> {
    let opts = ConnectOptions::with_credentials_file(creds_path).await?;
    Ok(opts.connect(servers).await?)
}

/// Opens or creates the KV bucket that contains dashboard configuration.
///
/// If opening the bucket fails for any reason, the function tries to create it with a
/// history of 5 revisions per key.
///
/// # Arguments
///
/// * `js` - JetStream context of the server that holds the bucket.
/// * `bucket` - Bucket name, for example `avenabox`.
///
/// # Errors
///
/// Returns an error if the bucket cannot be opened and creating it also fails.
async fn ensure_kv_bucket(js: &jetstream::Context, bucket: &str) -> Result<kv::Store, DynError> {
    if let Ok(store) = js.get_key_value(bucket).await {
        return Ok(store);
    }
    Ok(js
        .create_key_value(kv::Config {
            bucket: bucket.to_string(),
            history: 5,
            ..Default::default()
        })
        .await?)
}

/// Copies the configured central KV entry into local KV during startup.
///
/// The remote payload is deserialized before writing locally so invalid central
/// configuration does not replace the local copy. The local key is written only when
/// its value differs from the central one. The central bucket is created if it does not
/// exist (through [`ensure_kv_bucket`]).
///
/// # Arguments
///
/// * `sync_cfg` - Central server, credentials, bucket and key.
/// * `local_store` - Local KV bucket to write into.
/// * `local_key` - Local key to write.
///
/// # Errors
///
/// Returns an error if connecting to the central server or opening its bucket fails, if
/// the central key does not exist, if its value is not a valid [`NestedConfig`], or if
/// reading or writing the local key fails.
async fn mirror_central_kv_once(
    sync_cfg: &CentralKvSyncConfig,
    local_store: &kv::Store,
    local_key: &str,
) -> Result<(), DynError> {
    let client =
        connect_nats_with_creds(sync_cfg.servers.clone(), sync_cfg.creds_path.clone()).await?;
    let remote_js = nats_config::jetstream_context_for_domain(client, sync_cfg.domain.as_deref());
    let remote_store = ensure_kv_bucket(&remote_js, &sync_cfg.bucket).await?;
    let remote_entry = remote_store
        .entry(sync_cfg.key.as_str())
        .await?
        .ok_or_else(|| format!("central KV key '{}' not found", sync_cfg.key))?;
    serde_json::from_slice::<NestedConfig>(&remote_entry.value)?;
    let should_put = match local_store.entry(local_key).await? {
        Some(local_entry) => local_entry.value.as_ref() != remote_entry.value.as_ref(),
        None => true,
    };
    if should_put {
        local_store
            .put(local_key, remote_entry.value.clone())
            .await?;
        println!(
            "[logger] Mirrored central KV '{}:{}' into local '{}'",
            sync_cfg.bucket, sync_cfg.key, local_key
        );
    }
    Ok(())
}

/// Continuously mirrors central KV updates into the local configuration bucket.
///
/// This task reconnects after setup or watch failures and only writes updates
/// that parse as a valid dashboard configuration. Each retry waits 5 seconds. Delete and
/// purge events are logged and ignored, and a value equal to the local one is not
/// rewritten. The function never returns; `main` runs it with `tokio::spawn`.
///
/// # Arguments
///
/// * `sync_cfg` - Central server, credentials, bucket and key.
/// * `local_store` - Local KV bucket to write into.
/// * `local_key` - Local key to write.
async fn run_central_kv_sync(
    sync_cfg: CentralKvSyncConfig,
    local_store: kv::Store,
    local_key: String,
) {
    loop {
        let client =
            match connect_nats_with_creds(sync_cfg.servers.clone(), sync_cfg.creds_path.clone())
                .await
            {
                Ok(client) => client,
                Err(err) => {
                    let err = err.to_string();
                    eprintln!("[logger] Central KV connect failed: {err}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
        let remote_js =
            nats_config::jetstream_context_for_domain(client, sync_cfg.domain.as_deref());
        let remote_store = match ensure_kv_bucket(&remote_js, &sync_cfg.bucket).await {
            Ok(store) => store,
            Err(err) => {
                let err = err.to_string();
                eprintln!("[logger] Central KV bucket setup failed: {err}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let mut watch = match remote_store.watch(sync_cfg.key.as_str()).await {
            Ok(watch) => watch,
            Err(err) => {
                let err = err.to_string();
                eprintln!("[logger] Central KV watch setup failed: {err}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        println!(
            "[logger] Watching central KV '{}:{}' for local key '{}'",
            sync_cfg.bucket, sync_cfg.key, local_key
        );
        while let Some(event) = watch.next().await {
            match event {
                Ok(entry) if entry.operation == Operation::Put => {
                    if let Err(err) = serde_json::from_slice::<NestedConfig>(&entry.value) {
                        eprintln!("[logger] Ignoring invalid central KV update: {err}");
                        continue;
                    }
                    match local_store.entry(local_key.as_str()).await {
                        Ok(Some(local_entry))
                            if local_entry.value.as_ref() == entry.value.as_ref() => {}
                        Ok(_) => {
                            if let Err(err) = local_store
                                .put(local_key.as_str(), entry.value.clone())
                                .await
                            {
                                eprintln!("[logger] Failed to mirror central KV update: {err}");
                            } else {
                                println!(
                                    "[logger] Mirrored central KV rev {} into local KV",
                                    entry.revision
                                );
                            }
                        }
                        Err(err) => eprintln!("[logger] Failed to inspect local KV key: {err}"),
                    }
                }
                Ok(entry) => {
                    eprintln!(
                        "[logger] Ignoring central KV {:?} for '{}'",
                        entry.operation, entry.key
                    );
                }
                Err(err) => {
                    eprintln!("[logger] Central KV watch error: {err}");
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[allow(dead_code)]
/// Buffered writer for one channel's Parquet file.
///
/// Rows are buffered to form row groups. Files are partitioned by asset, UTC
/// date, channel, and monotonically increasing part index. While open, the file is
/// named `part-<NNNN>.parquet.inprogress`; [`Self::close`] renames it to
/// `part-<NNNN>.parquet`.
struct ParquetLogger {
    /// Parquet writer over the `.inprogress` file.
    writer: SerializedFileWriter<fs::File>,
    /// Path of the file while it is being written.
    inprogress_path: PathBuf,
    /// Path the file is renamed to when it is closed.
    final_path: PathBuf,
    /// Buffered timestamps (Unix nanoseconds) of the current row group.
    timestamps: Vec<i64>,
    /// Buffered sample values of the current row group, parallel to `timestamps`.
    values: Vec<f64>,
    /// Row count at which the buffer is written as a row group.
    max_rows: usize,
    /// Timestamp (Unix nanoseconds) of the first row written to this file.
    first_timestamp_unix_ns: Option<i64>,
    /// Number of row groups written to the file so far.
    row_groups_written: usize,
    /// UTC date of the samples in this file.
    date: NaiveDate,
    /// Asset number the file belongs to.
    asset: u32,
    /// LabJack channel the file belongs to.
    channel: u8,
    /// Part index in the file name.
    file_index: usize,
}

/// Maximum row groups in one file before it is rotated.
///
/// Parquet's hard limit is 32,767 row groups per file. With [`ROWS_PER_ROW_GROUP`] rows
/// per group a normal rotation window never comes close; this is a safety limit that
/// starts a new file well before the hard limit.
const MAX_ROW_GROUPS_PER_FILE: usize = 30_000;

/// Rows buffered before they are written as one row group (1,048,576).
///
/// One row group normally holds a whole rotation period (600,000 rows for five
/// minutes at 2 kHz). Large row groups let dictionary and delta encoding work
/// across the file instead of restarting every 1,000 rows. An unfinished file
/// has no footer and is quarantined whole, so smaller row groups would not make
/// buffered rows any safer; unacked JetStream messages are what protect them.
const ROWS_PER_ROW_GROUP: usize = 1 << 20;

/// Unacked messages that force the open file to close early.
///
/// Messages are acked only after the file holding their samples is closed. The
/// file is closed early if this many messages are waiting, so the consumer can
/// never reach its `max_ack_pending` limit ([`CONSUMER_MAX_ACK_PENDING`]) and stall.
const MAX_PENDING_ACKS_PER_FILE: usize = 20_000;
/// `max_ack_pending` set on each durable consumer, in messages.
///
/// Kept well above [`MAX_PENDING_ACKS_PER_FILE`] so the per-file cap is always reached
/// first.
const CONSUMER_MAX_ACK_PENDING: i64 = 50_000;

/// Time without new data after which the open file is closed (60 s).
///
/// Close an open file when no data has arrived for this long, for example when
/// the streamer is stopped, so its messages are acked promptly.
const IDLE_CLOSE_AFTER: Duration = Duration::from_secs(60);
/// How often each writer task checks for an idle file (15 s).
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(15);

/// Returns the index of the aligned rotation window holding a timestamp.
///
/// Windows start at multiples of `rotate_secs` since the Unix epoch, so with a
/// five-minute period every file covers :00-:05, :05-:10 and so on. Floor division is
/// used, so timestamps before the epoch get negative indices.
///
/// # Arguments
///
/// * `timestamp_unix_ns` - Sample time in Unix nanoseconds.
/// * `rotate_secs` - Window length in seconds. `0` is treated as `1`.
///
/// # Returns
///
/// The number of whole windows between the Unix epoch and the timestamp.
///
/// # Examples
///
/// ```text
/// rotation_window(299_999_999_999, 300) == 0
/// rotation_window(300_000_000_000, 300) == 1
/// rotation_window(-1, 300)              == -1
/// ```
fn rotation_window(timestamp_unix_ns: i64, rotate_secs: u64) -> i64 {
    let rotate_ns = (rotate_secs.max(1) as i64).saturating_mul(1_000_000_000);
    timestamp_unix_ns.div_euclid(rotate_ns)
}

/// Runtime state for one active channel consumer and writer task.
///
/// The archiver keeps this state so KV updates can stop removed channels,
/// rotate calibration metadata, or respawn consumers when subject identity
/// changes. The identity fields are the values the task was started with.
struct ChannelLogger {
    /// Handle of the writer task; `main` restarts the channel when it has finished.
    handle: tokio::task::JoinHandle<()>,
    /// Sends the graceful stop request; taken by [`Self::stop`].
    stop_tx: Option<oneshot::Sender<()>>,
    /// Sends calibration updates to the writer task.
    calibration_tx: watch::Sender<CalibrationSpec>,
    /// Calibration last sent to the writer task.
    calibration: CalibrationSpec,
    /// JetStream subject the consumer filters on.
    subject: String,
    /// JetStream stream the consumer belongs to.
    stream_name: String,
    /// Durable consumer name.
    consumer_name: String,
    /// Asset number used in output paths.
    asset: u32,
    /// Rotation window length in seconds.
    rotate_secs: u64,
}

impl ChannelLogger {
    /// Requests an orderly writer shutdown and waits for the task to finish.
    ///
    /// On a graceful stop the task closes its open file (writing the Parquet footer) and
    /// acks the messages in it before it ends. If the task has already ended, the stop
    /// request is ignored and only the join happens. A join error (for example a panic in
    /// the task) is logged.
    async fn stop(mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Err(err) = self.handle.await {
            eprintln!("[logger] Channel writer task failed to join: {err}");
        }
    }
}

impl ParquetLogger {
    /// Creates a new Parquet file for one asset, channel, date, and part index.
    ///
    /// The file is created at
    /// `<parquet_root>/asset<NNN>/<YYYY-MM-DD>/ch<NN>/part-<NNNN>.parquet.inprogress`,
    /// creating directories as needed. The calibration spec is written into file-level
    /// key-value metadata (key `calibration`) so exported data can be interpreted
    /// without consulting the live config. If the spec cannot be serialized, `{}` is
    /// stored instead.
    ///
    /// # Arguments
    ///
    /// * `asset` - Asset number for the `asset<NNN>` directory.
    /// * `channel` - LabJack channel for the `ch<NN>` directory.
    /// * `file_index` - Part index for the file name, usually from [`next_file_index`].
    /// * `date` - UTC date of the samples, for the date directory.
    /// * `calibration` - Calibration stored in the file metadata.
    /// * `parquet_root` - Root output directory.
    ///
    /// # Panics
    ///
    /// Panics if the directory or file cannot be created, or if the Parquet writer
    /// cannot be set up.
    fn new(
        asset: u32,
        channel: u8,
        file_index: usize,
        date: NaiveDate,
        calibration: CalibrationSpec,
        parquet_root: &Path,
    ) -> Self {
        let dir = parquet_root
            .join(format!("asset{:03}", asset))
            .join(date.format("%Y-%m-%d").to_string())
            .join(format!("ch{:02}", channel));

        fs::create_dir_all(&dir).unwrap();
        let final_path = dir.join(format!("part-{:04}.parquet", file_index));
        let inprogress_path = dir.join(format!("part-{:04}.parquet.inprogress", file_index));

        let schema = Arc::new(parse_message_type(SAMPLE_SCHEMA).unwrap());
        let calibration_json =
            serde_json::to_string(&calibration).unwrap_or_else(|_| "{}".to_string());
        let props = Arc::new(writer_properties_for_calibration(calibration_json));
        let file = fs::File::create(&inprogress_path).unwrap();
        let writer = SerializedFileWriter::new(file, schema, props).unwrap();

        Self {
            writer,
            inprogress_path,
            final_path,
            timestamps: Vec::new(),
            values: Vec::new(),
            max_rows: ROWS_PER_ROW_GROUP,
            first_timestamp_unix_ns: None,
            row_groups_written: 0,
            date,
            asset,
            channel,
            file_index,
        }
    }

    /// Buffers one timestamped value and flushes when the row group is full.
    ///
    /// The first call also records the file's first timestamp, which
    /// [`Self::should_rotate_before`] uses to find the file's rotation window.
    ///
    /// # Arguments
    ///
    /// * `timestamp_unix_ns` - Sample time in Unix nanoseconds.
    /// * `val` - Raw sample value.
    ///
    /// # Panics
    ///
    /// Panics if writing the row group fails (see [`Self::flush`]).
    fn write_row(&mut self, timestamp_unix_ns: i64, val: f64) {
        self.first_timestamp_unix_ns
            .get_or_insert(timestamp_unix_ns);
        self.timestamps.push(timestamp_unix_ns);
        self.values.push(val);
        if self.timestamps.len() >= self.max_rows {
            self.flush();
        }
    }

    /// Writes the current buffer as a Parquet row group.
    ///
    /// Does nothing when the buffer is empty. Otherwise writes the timestamp and value
    /// columns, closes the row group and clears the buffer. The data is not synced to
    /// disk here; that happens in [`Self::close`].
    ///
    /// # Panics
    ///
    /// Panics if the Parquet writer fails to write or close the row group or a column.
    fn flush(&mut self) {
        if self.timestamps.is_empty() {
            return;
        }
        let mut rg = self.writer.next_row_group().unwrap();

        // column 0: timestamps
        {
            let mut scw = rg.next_column().unwrap().expect("timestamp col");
            let mut cw = scw.untyped();
            if let ColumnWriter::Int64ColumnWriter(typed) = &mut cw {
                typed.write_batch(&self.timestamps, None, None).unwrap();
            }
            scw.close().unwrap();
        }

        // column 1: values
        {
            let mut scw = rg.next_column().unwrap().expect("value col");
            let mut cw = scw.untyped();
            if let ColumnWriter::DoubleColumnWriter(typed) = &mut cw {
                typed.write_batch(&self.values, None, None).unwrap();
            }
            scw.close().unwrap();
        }

        rg.close().unwrap();
        self.row_groups_written += 1;
        self.timestamps.clear();
        self.values.clear();
    }

    /// Returns whether the next source sample belongs in a new file.
    ///
    /// Files cover aligned windows of source time (see [`rotation_window`]).
    /// Using source time rather than wall-clock time keeps backlog replay
    /// correct: several hours of samples can be consumed in a few seconds.
    ///
    /// # Arguments
    ///
    /// * `timestamp_unix_ns` - Time of the next sample, in Unix nanoseconds.
    /// * `rotate_secs` - Rotation window length in seconds.
    ///
    /// # Returns
    ///
    /// `true` if the file already has [`MAX_ROW_GROUPS_PER_FILE`] row groups, if the
    /// sample is earlier than the file's first sample, or if it falls in a different
    /// rotation window. `false` for an empty file otherwise.
    fn should_rotate_before(&self, timestamp_unix_ns: i64, rotate_secs: u64) -> bool {
        if self.row_groups_written >= MAX_ROW_GROUPS_PER_FILE {
            return true;
        }

        let Some(first_timestamp_unix_ns) = self.first_timestamp_unix_ns else {
            return false;
        };
        if timestamp_unix_ns < first_timestamp_unix_ns {
            return true;
        }

        rotation_window(timestamp_unix_ns, rotate_secs)
            != rotation_window(first_timestamp_unix_ns, rotate_secs)
    }

    /// Flushes buffered rows, writes the footer, syncs, and publishes the file.
    ///
    /// The data and the rename are both flushed to disk before returning, so a
    /// caller that acks JetStream messages afterwards cannot lose them to a
    /// power cut. The steps are: write remaining rows, write the footer, fsync the
    /// file, rename `.parquet.inprogress` to `.parquet`, then fsync the directory.
    ///
    /// # Returns
    ///
    /// The final `.parquet` path.
    ///
    /// # Errors
    ///
    /// Returns an error if writing the footer, syncing the file, renaming it, or
    /// opening or syncing the directory fails. When only the directory sync fails, the
    /// file has already been renamed.
    ///
    /// # Panics
    ///
    /// Panics if writing the remaining buffered rows fails (see [`Self::flush`]).
    fn close(mut self) -> Result<PathBuf, DynError> {
        self.flush();
        let file = self.writer.into_inner()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&self.inprogress_path, &self.final_path)?;
        if let Some(dir) = self.final_path.parent() {
            fs::File::open(dir)?.sync_all()?;
        }
        Ok(self.final_path)
    }
}

/// Renames unfinished writer files under `root` so they are kept but never read.
///
/// A crash or power loss can leave a `.parquet.inprogress` file without a Parquet
/// footer. Every such file, in any subdirectory, is renamed to
/// `<name>.unfinished.quarantined-<unix_ms>-<n>`. The original bytes are preserved for
/// diagnosis. The quarantined name still starts with `part-<NNNN>`, so
/// [`next_file_index`] skips that index and later data goes to a new part. The samples
/// themselves come back through JetStream redelivery, because their messages were
/// never acked. Completed `.parquet` files are not opened or checked.
///
/// # Arguments
///
/// * `root` - Parquet output root. A missing directory is not an error.
///
/// # Returns
///
/// The new paths of the quarantined files.
///
/// # Errors
///
/// Returns an error if a directory cannot be read, a file type cannot be determined,
/// or a rename fails. Files renamed before the error stay renamed.
fn quarantine_incomplete_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    // Walks `dir` recursively, renaming unfinished files and appending their new paths.
    fn visit(dir: &Path, quarantined: &mut Vec<PathBuf>, stamp: u128) -> io::Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                visit(&path, quarantined, stamp)?;
                continue;
            }

            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.ends_with(".parquet.inprogress") {
                continue;
            }

            let quarantine_name = format!(
                "{name}.unfinished.quarantined-{stamp}-{}",
                quarantined.len()
            );
            let quarantine_path = path.with_file_name(quarantine_name);
            fs::rename(&path, &quarantine_path)?;
            quarantined.push(quarantine_path);
        }
        Ok(())
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut quarantined = Vec::new();
    visit(root, &mut quarantined, stamp)?;
    Ok(quarantined)
}

/// Scans a channel/day directory to find the next available Parquet file index.
///
/// Every entry whose name starts with `part-<number>` counts, including
/// `.inprogress` and quarantined files, so a new file never reuses their index. The
/// directory is created if it does not exist.
///
/// # Arguments
///
/// * `parquet_root` - Root output directory.
/// * `asset` - Asset number for the `asset<NNN>` directory.
/// * `channel` - LabJack channel for the `ch<NN>` directory.
/// * `date` - UTC date for the date directory.
///
/// # Returns
///
/// One more than the highest existing part index, or `1` for an empty directory.
///
/// # Panics
///
/// Panics if the directory cannot be created or read.
fn next_file_index(parquet_root: &Path, asset: u32, channel: u8, date: NaiveDate) -> usize {
    let dir = parquet_root
        .join(format!("asset{:03}", asset))
        .join(date.format("%Y-%m-%d").to_string())
        .join(format!("ch{:02}", channel));

    std::fs::create_dir_all(&dir).unwrap();
    let mut max_idx = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        if let Some(num) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix("part-"))
            .and_then(|name| name.split('.').next())
            .and_then(|value| value.parse::<usize>().ok())
        {
            max_idx = max_idx.max(num);
        }
    }
    max_idx + 1
}

/// Converts source identity text into a durable consumer-name token.
///
/// ASCII letters and digits are lowercased and kept, as are `-` and `_`. Whitespace,
/// `.` and `/` become `-`. Every other character is dropped. Leading and trailing `-`
/// are trimmed.
///
/// # Arguments
///
/// * `raw` - Identity text such as a box ID or source ID.
///
/// # Returns
///
/// The token, or `unknown` if nothing is left.
///
/// # Examples
///
/// ```text
/// sanitize_consumer_token("MU1 Box.A") == "mu1-box-a"
/// sanitize_consumer_token(" ..! ")     == "unknown"
/// ```
fn sanitize_consumer_token(raw: &str) -> String {
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

/// Builds the durable JetStream consumer name for one archived channel.
///
/// The name is `archiver-<box>-<source_type>-<source_id>-<channel>-current`, with each
/// part passed through [`sanitize_consumer_token`]. Missing values default to
/// `unknown-box`, `labjack` and the LabJack name. The channel number is not
/// zero-padded.
///
/// # Arguments
///
/// * `cfg` - Archiver config supplying the identity fields.
/// * `channel` - LabJack channel number.
///
/// # Examples
///
/// ```text
/// box_id = "box-01", source_type = None, source_id = "MU1", channel = 8
///   -> "archiver-box-01-labjack-mu1-8-current"
/// ```
fn archiver_consumer_name(cfg: &SampleConfig, channel: u8) -> String {
    format!(
        "archiver-{}-{}-{}-{}-current",
        sanitize_consumer_token(cfg.box_id.as_deref().unwrap_or("unknown-box")),
        sanitize_consumer_token(cfg.source_type.as_deref().unwrap_or("labjack")),
        sanitize_consumer_token(
            cfg.source_id
                .as_deref()
                .unwrap_or(cfg.labjack_name.as_str())
        ),
        channel
    )
}

/// Files closed while handling one payload.
///
/// The writer task uses it to decide whether pending messages can be acked (a file
/// closed) or must be left for redelivery (a close failed).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CloseOutcome {
    /// Files closed and published successfully.
    closed: usize,
    /// Files whose close failed.
    failed: usize,
}

impl CloseOutcome {
    /// Counts and logs the result of one [`ParquetLogger::close`] call.
    ///
    /// # Arguments
    ///
    /// * `result` - Result returned by the close.
    /// * `channel` - Channel of the file, for the log message.
    /// * `file_index` - Part index of the file, for the log message.
    fn record(&mut self, result: Result<PathBuf, DynError>, channel: u8, file_index: usize) {
        match result {
            Ok(path) => {
                self.closed += 1;
                println!("[logger] Closed {}", path.display());
            }
            Err(err) => {
                self.failed += 1;
                eprintln!(
                    "[logger] Failed to finalize channel {channel:02} part {file_index}: {err}"
                );
            }
        }
    }
}

/// Decodes one FlatBuffer scan payload and writes its samples to Parquet.
///
/// The payload contains the first sample timestamp plus a fixed interval, so
/// this function reconstructs each sample timestamp and rotates files at
/// aligned source-time windows and UTC date changes. The returned outcome tells
/// the caller which buffered JetStream messages are now safe to ack.
///
/// Before each sample, the open file is closed and a new one opened (with the next
/// free part index) when there is no open file, when the sample's UTC date differs
/// from the file's, or when [`ParquetLogger::should_rotate_before`] says so. A single
/// payload can therefore close a file part way through.
///
/// Sequence gaps and resets are logged but do not stop the write. A payload that is
/// not a valid FlatBuffer `Scan` is logged and writes nothing. If a sample timestamp
/// overflows, the rest of that payload is dropped.
///
/// # Arguments
///
/// * `payload` - FlatBuffer `Scan` bytes from one JetStream message.
/// * `channel` - LabJack channel the payload belongs to.
/// * `asset` - Asset number for output paths.
/// * `parquet_root` - Root output directory.
/// * `active_calibration` - Calibration stored in any file opened here.
/// * `rotate_secs` - Rotation window length in seconds.
/// * `logger` - Open file of the channel, if any; replaced when the file rotates.
/// * `file_index` - Part index of the open file; updated when a new file is opened.
/// * `last_sequence` - Sequence number of the previous scan, for gap detection;
///   updated to this scan's sequence.
///
/// # Returns
///
/// How many files were closed and how many closes failed while writing this payload.
///
/// # Panics
///
/// Panics if a new file cannot be created or a row group cannot be written (see
/// [`ParquetLogger::new`], [`ParquetLogger::flush`] and [`next_file_index`]).
fn process_scan_payload(
    payload: &[u8],
    channel: u8,
    asset: u32,
    parquet_root: &Path,
    active_calibration: &CalibrationSpec,
    rotate_secs: u64,
    logger: &mut Option<ParquetLogger>,
    file_index: &mut usize,
    last_sequence: &mut Option<u64>,
) -> CloseOutcome {
    let mut outcome = CloseOutcome::default();
    if let Ok(scan) = flatbuffers::root::<sampler::Scan>(payload) {
        let sequence = scan.sequence();
        match *last_sequence {
            Some(previous) if sequence == previous + 1 => {}
            Some(previous) if sequence > previous + 1 => {
                eprintln!(
                    "[logger] Channel {channel:02} sequence gap: expected {}, got {}",
                    previous + 1,
                    sequence
                );
            }
            Some(previous) if sequence <= previous => {
                println!(
                    "[logger] Channel {channel:02} sequence reset/new run: previous {}, current {}",
                    previous, sequence
                );
            }
            _ => {}
        }
        *last_sequence = Some(sequence);

        if let Some(vals) = scan.values() {
            let first_sample_unix_ns = scan.first_sample_unix_ns();
            let sample_interval_ns = scan.sample_interval_ns();

            for (index, v) in vals.iter().enumerate() {
                let timestamp_unix_ns = match sample_timestamp_ns(
                    first_sample_unix_ns,
                    sample_interval_ns,
                    index,
                ) {
                    Ok(ts) => ts,
                    Err(err) => {
                        eprintln!(
                            "[logger] Channel {channel:02} timestamp overflow at sequence {} sample {}: {}",
                            sequence, index, err
                        );
                        break;
                    }
                };

                let sample_date = timestamp_ns_to_utc_date(timestamp_unix_ns);
                if logger
                    .as_ref()
                    .map(|l| {
                        l.date != sample_date
                            || l.should_rotate_before(timestamp_unix_ns, rotate_secs)
                    })
                    .unwrap_or(true)
                {
                    if let Some(l) = logger.take() {
                        outcome.record(l.close(), channel, *file_index);
                    }
                    *file_index = next_file_index(parquet_root, asset, channel, sample_date);
                    *logger = Some(ParquetLogger::new(
                        asset,
                        channel,
                        *file_index,
                        sample_date,
                        active_calibration.clone(),
                        parquet_root,
                    ));
                }

                if let Some(log) = logger.as_mut() {
                    log.write_row(timestamp_unix_ns, v);
                }
            }
        }
    } else {
        eprintln!("[logger] Channel {channel:02} received invalid FlatBuffer payload");
    }
    outcome
}

/// Acks messages whose samples are all inside closed, synced Parquet files.
///
/// Drains `pending`. A failed ack is counted and logged, not retried; JetStream
/// redelivers that message after `ack_wait`, so its samples are written a second time
/// into a later file.
///
/// # Arguments
///
/// * `pending` - Messages to ack; empty on return.
/// * `channel` - Channel number, for the log message.
async fn ack_messages(pending: &mut Vec<jetstream::Message>, channel: u8) {
    let mut failures = 0usize;
    for msg in pending.drain(..) {
        if msg.ack().await.is_err() {
            failures += 1;
        }
    }
    if failures > 0 {
        eprintln!(
            "[logger] Failed to ack {failures} JetStream message(s) for channel {channel:02}; they will be redelivered"
        );
    }
}

/// Closes the open file (if any) and settles the messages it holds.
///
/// On success the pending messages are acked. On failure they are dropped
/// without an ack, so JetStream redelivers them after `ack_wait` and they are
/// written again into a new file. With no open file, the pending messages are acked.
///
/// # Arguments
///
/// * `logger` - Open file of the channel; `None` on return.
/// * `pending` - Messages whose samples are in the open file; empty on return.
/// * `channel` - Channel number, for log messages.
/// * `file_index` - Part index of the open file, for log messages.
async fn close_and_settle(
    logger: &mut Option<ParquetLogger>,
    pending: &mut Vec<jetstream::Message>,
    channel: u8,
    file_index: usize,
) {
    let mut outcome = CloseOutcome::default();
    if let Some(l) = logger.take() {
        outcome.record(l.close(), channel, file_index);
    }
    if outcome.failed > 0 {
        eprintln!(
            "[logger] Leaving {} message(s) for channel {channel:02} unacked for redelivery",
            pending.len()
        );
        pending.clear();
    } else {
        ack_messages(pending, channel).await;
    }
}

/// Consumer settings that let acks wait until a file is closed.
///
/// `ack_wait` must outlast one full rotation window plus the idle-close delay,
/// or JetStream would redeliver messages that are still waiting in an open file. It is
/// set to three rotation windows plus [`IDLE_CLOSE_AFTER`] plus 120 seconds of margin
/// (18 minutes for a 300 second window). The consumer uses explicit acks and
/// [`CONSUMER_MAX_ACK_PENDING`].
///
/// # Arguments
///
/// * `consumer_name` - Durable consumer name.
/// * `subject` - Subject the consumer filters on.
/// * `rotate_secs` - Rotation window length in seconds.
fn archiver_consumer_config(consumer_name: &str, subject: &str, rotate_secs: u64) -> pull::Config {
    pull::Config {
        durable_name: Some(consumer_name.to_string()),
        filter_subject: subject.to_string(),
        ack_policy: jetstream::consumer::AckPolicy::Explicit,
        ack_wait: Duration::from_secs(rotate_secs.saturating_mul(3))
            + IDLE_CLOSE_AFTER
            + Duration::from_secs(120),
        max_ack_pending: CONSUMER_MAX_ACK_PENDING,
        ..Default::default()
    }
}

/// Starts the durable pull consumer and writer task for one channel.
///
/// The returned [`ChannelLogger`] lets the config watcher update calibration
/// metadata or gracefully stop and respawn the task when identity changes.
///
/// The consumer is created with [`archiver_consumer_config`] if it does not exist. An
/// existing consumer whose `ack_wait` or `max_ack_pending` differs is updated in place.
///
/// The spawned task reads messages and writes them with [`process_scan_payload`],
/// holding each message unacked until its file is closed. It closes the open file and
/// settles its messages (see [`close_and_settle`]) when:
///
/// * [`MAX_PENDING_ACKS_PER_FILE`] messages are waiting,
/// * no data has arrived for [`IDLE_CLOSE_AFTER`] (checked every
///   [`IDLE_CHECK_INTERVAL`]),
/// * a different calibration arrives, so the next file carries the new metadata,
/// * the task ends because of a stop request, a consumer error, the end of the
///   message stream, or the calibration sender being dropped.
///
/// If attaching the message stream fails, the task logs the error and ends at once. A
/// panic inside the task (for example an output directory that cannot be created) also
/// ends it. In both cases `main` sees the task as finished and restarts the channel on
/// its next health check.
///
/// # Arguments
///
/// * `js` - Local JetStream context.
/// * `stream_name` - Stream that holds the channel subject.
/// * `consumer_name` - Durable consumer name, from [`archiver_consumer_name`].
/// * `subject` - Channel subject to filter on.
/// * `asset` - Asset number for output paths.
/// * `channel` - LabJack channel number.
/// * `rotate_secs` - Rotation window length in seconds.
/// * `calibration` - Initial calibration for the channel.
/// * `parquet_root` - Root output directory.
///
/// # Errors
///
/// Returns an error if the stream cannot be found, or the consumer cannot be created,
/// fetched or updated.
async fn spawn_channel_logger(
    js: jetstream::Context,
    stream_name: String,
    consumer_name: String,
    subject: String,
    asset: u32,
    channel: u8,
    rotate_secs: u64,
    calibration: CalibrationSpec,
    parquet_root: PathBuf,
) -> Result<ChannelLogger, DynError> {
    let stream = js.get_stream(stream_name.as_str()).await?;
    let desired = archiver_consumer_config(&consumer_name, &subject, rotate_secs);
    let mut consumer = stream
        .get_or_create_consumer(consumer_name.as_str(), desired.clone())
        .await?;
    // Durable consumers created by earlier versions keep their old ack_wait
    // and max_ack_pending. Both are editable in place, and the delivery
    // position is untouched, so no data is replayed or skipped.
    let existing = consumer.cached_info().config.clone();
    if existing.ack_wait != desired.ack_wait || existing.max_ack_pending != desired.max_ack_pending
    {
        consumer = stream.update_consumer(desired.clone()).await?;
        println!(
            "[logger] Updated consumer '{}': ack_wait {:?} -> {:?}, max_ack_pending {} -> {}",
            consumer_name,
            existing.ack_wait,
            desired.ack_wait,
            existing.max_ack_pending,
            desired.max_ack_pending
        );
    }

    let logger_subject = subject.clone();
    let logger_consumer_name = consumer_name.clone();
    let (calibration_tx, mut calibration_rx) = watch::channel(calibration.clone());
    let (stop_tx, mut stop_rx) = oneshot::channel();
    let calibration_for_task = calibration.clone();
    let handle = tokio::spawn(async move {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(err) => {
                eprintln!(
                    "[logger] Failed to attach JetStream consumer '{}' for {}: {}",
                    logger_consumer_name, logger_subject, err
                );
                return;
            }
        };
        println!(
            "[logger] Attached JetStream consumer '{}' to {}",
            logger_consumer_name, logger_subject
        );

        // Files are opened by data and rotated by source time. This ticker only
        // closes a file that has stopped receiving data; it never opens one,
        // so it cannot race the source-time rotation or leave empty parts.
        let mut idle_check = tokio::time::interval(IDLE_CHECK_INTERVAL);
        idle_check.tick().await;
        let mut logger: Option<ParquetLogger> = None;
        let mut file_index =
            next_file_index(&parquet_root, asset, channel, Utc::now().date_naive());
        let mut active_calibration = calibration_for_task;
        let mut last_sequence: Option<u64> = None;
        let mut pending: Vec<jetstream::Message> = Vec::new();
        let mut last_data = Instant::now();

        loop {
            tokio::select! {
                maybe = messages.next() => {
                    match maybe {
                        Some(Ok(msg)) => {
                            last_data = Instant::now();
                            let outcome = process_scan_payload(
                                &msg.payload,
                                channel,
                                asset,
                                &parquet_root,
                                &active_calibration,
                                rotate_secs,
                                &mut logger,
                                &mut file_index,
                                &mut last_sequence,
                            );
                            if outcome.failed > 0 {
                                // Let JetStream redeliver everything that was in
                                // the failed file, including this message.
                                eprintln!(
                                    "[logger] Leaving {} message(s) for channel {channel:02} unacked for redelivery",
                                    pending.len() + 1
                                );
                                pending.clear();
                            } else {
                                if outcome.closed > 0 {
                                    // Earlier messages are entirely inside the
                                    // closed file. This one may have samples in
                                    // the new file too, so it waits.
                                    ack_messages(&mut pending, channel).await;
                                }
                                pending.push(msg);
                            }
                            if pending.len() >= MAX_PENDING_ACKS_PER_FILE {
                                close_and_settle(&mut logger, &mut pending, channel, file_index).await;
                            }
                        }
                        Some(Err(err)) => {
                            eprintln!(
                                "[logger] JetStream consumer '{}' error on {}: {}",
                                logger_consumer_name, logger_subject, err
                            );
                            break;
                        }
                        None => {
                            eprintln!(
                                "[logger] JetStream consumer '{}' ended for {}",
                                logger_consumer_name, logger_subject
                            );
                            break;
                        }
                    }
                }
                _ = idle_check.tick() => {
                    if logger.is_some() && last_data.elapsed() >= IDLE_CLOSE_AFTER {
                        println!(
                            "[logger] No data on channel {channel:02} for {}s; closing the open file.",
                            IDLE_CLOSE_AFTER.as_secs()
                        );
                        close_and_settle(&mut logger, &mut pending, channel, file_index).await;
                    }
                }
                changed = calibration_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let updated = calibration_rx.borrow().clone();
                    if updated != active_calibration {
                        // The next sample opens a new file carrying the new
                        // calibration metadata.
                        close_and_settle(&mut logger, &mut pending, channel, file_index).await;
                        println!(
                            "[logger] Calibration updated for channel {channel:02}; rotating file."
                        );
                        active_calibration = updated;
                    }
                }
                _ = &mut stop_rx => {
                    println!("[logger] Graceful stop requested for channel {channel:02}");
                    break;
                }
            }
        }

        close_and_settle(&mut logger, &mut pending, channel, file_index).await;
    });

    Ok(ChannelLogger {
        handle,
        stop_tx: Some(stop_tx),
        calibration_tx,
        calibration,
        subject,
        stream_name,
        consumer_name,
        asset,
        rotate_secs,
    })
}

/// Computes the Unix timestamp for a sample index within a FlatBuffer scan.
///
/// # Arguments
///
/// * `first_sample_unix_ns` - Time of the scan's first sample, in Unix nanoseconds.
/// * `sample_interval_ns` - Time between samples, in nanoseconds.
/// * `index` - Zero-based sample index within the scan.
///
/// # Returns
///
/// `first_sample_unix_ns + sample_interval_ns * index`, in Unix nanoseconds.
///
/// # Errors
///
/// Returns an error if the result does not fit in an `i64`.
///
/// # Examples
///
/// ```text
/// sample_timestamp_ns(1_000, 500_000, 3) == Ok(1_501_000)
/// ```
fn sample_timestamp_ns(
    first_sample_unix_ns: u64,
    sample_interval_ns: u64,
    index: usize,
) -> Result<i64, String> {
    let timestamp = (first_sample_unix_ns as u128)
        .checked_add((sample_interval_ns as u128).saturating_mul(index as u128))
        .ok_or_else(|| "sample timestamp overflowed u128".to_string())?;
    i64::try_from(timestamp).map_err(|_| "sample timestamp exceeds i64 range".to_string())
}

/// Converts a Unix nanosecond timestamp to its UTC calendar date.
///
/// # Arguments
///
/// * `timestamp_unix_ns` - Time in Unix nanoseconds.
///
/// # Examples
///
/// ```text
/// timestamp_ns_to_utc_date(1_754_395_200_000_000_000) == 2025-08-05
/// ```
fn timestamp_ns_to_utc_date(timestamp_unix_ns: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp_nanos(timestamp_unix_ns).date_naive()
}

#[tokio::main]
/// Starts the archiver service and reacts to configuration updates.
///
/// Startup, in order:
///
/// 1. Quarantines unfinished files under `PARQUET_DIR` (see
///    [`quarantine_incomplete_files`]).
/// 2. Connects to local NATS with `NATS_CREDS_FILE` and opens (or creates) the config KV
///    bucket.
/// 3. If central mirroring is configured, copies the central config once (a failure is
///    logged, not fatal) and later starts [`run_central_kv_sync`] in the background.
/// 4. Loads the config from `CFG_KEY` and starts one writer per enabled channel with
///    [`spawn_channel_logger`].
///
/// It then loops until Ctrl-C or the end of the KV watch:
///
/// * Every 5 seconds it restarts any channel whose writer task has finished or is
///   missing (for example after a consumer error or a failed start).
/// * On a KV put it parses the new config (invalid values are logged and ignored),
///   stops writers for removed channels, restarts writers whose subject, stream,
///   consumer name, asset or `rotate_secs` changed, sends calibration-only changes to
///   the running writer, and starts writers for new channels. Delete and purge events
///   are ignored.
///
/// On exit it stops every writer, which closes its file and acks its messages.
///
/// # Errors
///
/// Returns an error if `NATS_SERVERS` is invalid, if quarantining fails, if the
/// credentials cannot be loaded or the NATS connection fails, if the KV bucket cannot
/// be opened or created, if the central server list is invalid, if the config key is
/// missing or does not parse, if the KV watch cannot be started, if any initial channel
/// writer fails to start, or if listening for Ctrl-C fails.
async fn main() -> Result<(), DynError> {
    let servers = nats_config::servers_from_env()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let parquet_root =
        PathBuf::from(std::env::var("PARQUET_DIR").unwrap_or_else(|_| "parquet".into()));
    for path in quarantine_incomplete_files(&parquet_root)? {
        eprintln!(
            "[logger] Quarantined unfinished Parquet writer file: {}",
            path.display()
        );
    }

    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
    let sample_opts = ConnectOptions::with_credentials_file(creds_path.clone())
        .await
        .map_err(|e| format!("Failed to load creds: {}", e))?;

    let nc = sample_opts
        .connect(servers)
        .await
        .map_err(|e| format!("NATS connect failed: {}", e))?;

    println!("Connected to sample NATS via creds!");
    let js = nats_config::jetstream_context(nc);

    // Load the config from local KV, mirroring it from central NATS first if configured.
    let bucket = std::env::var("CFG_BUCKET").unwrap_or_else(|_| "avenabox".into());
    let key = std::env::var("CFG_KEY").unwrap_or_else(|_| "labjackd.config.macbook".into());
    let store = ensure_kv_bucket(&js, bucket.as_str()).await?;
    let central_sync_cfg = central_kv_sync_config_from_env(&creds_path)?;
    if let Some(sync_cfg) = central_sync_cfg.as_ref() {
        if let Err(err) = mirror_central_kv_once(sync_cfg, &store, &key).await {
            eprintln!("[logger] Initial central-to-local KV mirror failed: {err}");
        }
    }
    let entry = store.entry(key.as_str()).await?.ok_or("KV key not found")?;

    let nested = serde_json::from_slice::<NestedConfig>(&entry.value)?;
    let mut cfg: SampleConfig = sample_config_from_nested(nested);

    println!("[logger] Loaded config: {:?}", cfg);

    // Create channel loggers and watch KV changes in the main task so
    // shutdown can wait for every active Parquet writer to close.
    let mut watch = store.watch(key.as_str()).await?;
    let mut active: HashMap<u8, ChannelLogger> = HashMap::new();
    if let Some(sync_cfg) = central_sync_cfg {
        tokio::spawn(run_central_kv_sync(sync_cfg, store.clone(), key.clone()));
    }

    // initial subscriptions
    for ch in &cfg.channels {
        let subject = subjects::live_labjack_channel_subject(
            &cfg.nats_subject,
            cfg.asset_number,
            *ch,
            cfg.site_id.as_deref(),
            cfg.box_id.as_deref(),
            Some(&cfg.labjack_name),
            cfg.source_type.as_deref(),
            cfg.source_id.as_deref(),
        );
        let calibration = cfg.calibrations.get(ch).cloned().unwrap_or_default();
        let consumer_name = archiver_consumer_name(&cfg, *ch);
        let h = spawn_channel_logger(
            js.clone(),
            cfg.nats_stream.clone(),
            consumer_name,
            subject,
            cfg.asset_number,
            *ch,
            cfg.rotate_secs,
            calibration,
            parquet_root.clone(),
        )
        .await?;
        active.insert(*ch, h);
    }

    println!("[logger] Watching KV for config changes...");
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let mut health_check = tokio::time::interval(Duration::from_secs(5));
    // Skip the immediate first tick; initial channel setup just completed.
    health_check.tick().await;

    loop {
        let entry = tokio::select! {
            signal = &mut shutdown => {
                signal?;
                println!("[logger] Shutdown signal received.");
                break;
            }
            event = watch.next() => {
                match event {
                    Some(Ok(entry)) => entry,
                    Some(Err(err)) => {
                        eprintln!("[logger] KV watch error: {err}");
                        continue;
                    }
                    None => {
                        eprintln!("[logger] KV watch ended; shutting down writers.");
                        break;
                    }
                }
            }
            _ = health_check.tick() => {
                let restart_channels: Vec<u8> = cfg
                    .channels
                    .iter()
                    .copied()
                    .filter(|channel| {
                        active
                            .get(channel)
                            .map(|logger| logger.handle.is_finished())
                            .unwrap_or(true)
                    })
                    .collect();

                for channel in restart_channels {
                    if let Some(logger) = active.remove(&channel) {
                        eprintln!(
                            "[logger] Channel {channel:02} writer ended; restarting it."
                        );
                        logger.stop().await;
                    } else {
                        eprintln!(
                            "[logger] Channel {channel:02} writer is missing; starting it."
                        );
                    }

                    let subject = subjects::live_labjack_channel_subject(
                        &cfg.nats_subject,
                        cfg.asset_number,
                        channel,
                        cfg.site_id.as_deref(),
                        cfg.box_id.as_deref(),
                        Some(&cfg.labjack_name),
                        cfg.source_type.as_deref(),
                        cfg.source_id.as_deref(),
                    );
                    let consumer_name = archiver_consumer_name(&cfg, channel);
                    let calibration = cfg
                        .calibrations
                        .get(&channel)
                        .cloned()
                        .unwrap_or_default();
                    match spawn_channel_logger(
                        js.clone(),
                        cfg.nats_stream.clone(),
                        consumer_name,
                        subject,
                        cfg.asset_number,
                        channel,
                        cfg.rotate_secs,
                        calibration,
                        parquet_root.clone(),
                    )
                    .await
                    {
                        Ok(logger) => {
                            active.insert(channel, logger);
                        }
                        Err(err) => {
                            eprintln!(
                                "[logger] Failed to restart channel {channel:02}: {err}"
                            );
                        }
                    }
                }
                continue;
            }
        };

        if entry.operation != Operation::Put {
            continue;
        }
        let new_cfg = match serde_json::from_slice::<NestedConfig>(&entry.value)
            .map(sample_config_from_nested)
        {
            Ok(config) => config,
            Err(err) => {
                eprintln!("[logger] Ignoring invalid KV config update: {err}");
                continue;
            }
        };
        println!("[logger] KV config update detected: {:?}", new_cfg);

        let removed_channels: Vec<u8> = active
            .keys()
            .copied()
            .filter(|channel| !new_cfg.channels.contains(channel))
            .collect();
        for channel in removed_channels {
            if let Some(logger) = active.remove(&channel) {
                println!("[logger] Gracefully removing channel {channel}");
                logger.stop().await;
            }
        }

        for channel in &new_cfg.channels {
            let subject = subjects::live_labjack_channel_subject(
                &new_cfg.nats_subject,
                new_cfg.asset_number,
                *channel,
                new_cfg.site_id.as_deref(),
                new_cfg.box_id.as_deref(),
                Some(&new_cfg.labjack_name),
                new_cfg.source_type.as_deref(),
                new_cfg.source_id.as_deref(),
            );
            let consumer_name = archiver_consumer_name(&new_cfg, *channel);
            let calibration = new_cfg
                .calibrations
                .get(channel)
                .cloned()
                .unwrap_or_default();

            let mut needs_respawn = !active.contains_key(channel);
            if let Some(logger) = active.get_mut(channel) {
                needs_respawn = logger.subject != subject
                    || logger.stream_name != new_cfg.nats_stream
                    || logger.consumer_name != consumer_name
                    || logger.asset != new_cfg.asset_number
                    || logger.rotate_secs != new_cfg.rotate_secs;
                if logger.calibration != calibration && !needs_respawn {
                    if logger.calibration_tx.send(calibration.clone()).is_ok() {
                        logger.calibration = calibration.clone();
                    } else {
                        needs_respawn = true;
                    }
                }
            }

            if !needs_respawn {
                continue;
            }
            if let Some(logger) = active.remove(channel) {
                println!("[logger] Gracefully restarting channel {channel}");
                logger.stop().await;
            } else {
                println!("[logger] Adding channel {channel}");
            }

            match spawn_channel_logger(
                js.clone(),
                new_cfg.nats_stream.clone(),
                consumer_name,
                subject,
                new_cfg.asset_number,
                *channel,
                new_cfg.rotate_secs,
                calibration,
                parquet_root.clone(),
            )
            .await
            {
                Ok(logger) => {
                    active.insert(*channel, logger);
                }
                Err(err) => {
                    eprintln!("[logger] Failed to start channel {channel}: {err}");
                }
            }
        }
        cfg = new_cfg;
    }

    println!("[logger] Closing {} channel writer(s)...", active.len());
    for (_, logger) in active {
        logger.stop().await;
    }
    println!("[logger] Shutdown complete.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use uuid::Uuid;

    /// Returns a unique, not yet created directory under the system temp directory.
    fn temporary_parquet_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rust-ljm-{name}-{}", Uuid::new_v4()))
    }

    /// A closed file is renamed to `.parquet` and reads back with its one row.
    #[test]
    fn completed_writer_is_atomically_published_and_readable() {
        let root = temporary_parquet_root("finalize");
        let date = NaiveDate::from_ymd_opt(2026, 8, 5).expect("valid date");
        let mut logger = ParquetLogger::new(1001, 11, 1, date, CalibrationSpec::default(), &root);
        logger.write_row(1_754_395_200_000_000_000, 1.25);

        let final_path = logger.close().expect("writer should finalize");
        assert!(final_path.exists());
        assert!(!final_path.with_extension("parquet.inprogress").exists());

        let file = fs::File::open(&final_path).expect("final file should open");
        let reader = SerializedFileReader::new(file).expect("final file should be readable");
        assert_eq!(reader.metadata().file_metadata().num_rows(), 1);
        fs::remove_dir_all(root).expect("temporary directory should be removable");
    }

    /// Rotation happens at aligned window edges, on backwards time and at the row group cap.
    #[test]
    fn writer_rotates_at_aligned_source_windows_and_before_row_group_limit() {
        let root = temporary_parquet_root("source-rotation");
        let date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("valid date");
        // 2026-08-18 00:05:00 UTC is a five-minute boundary.
        let window_start = 1_787_011_500_000_000_000_i64;
        assert_eq!(window_start % 300_000_000_000, 0);
        let start = window_start + 10_000_000_000;
        let mut logger = ParquetLogger::new(1001, 0, 1, date, CalibrationSpec::default(), &root);
        logger.write_row(start, 1.0);

        // Same window: no rotation, even 289.999 s after the first sample.
        assert!(!logger.should_rotate_before(window_start + 299_999_999_999, 300));
        // Next aligned window starts only 290 s after the first sample.
        assert!(logger.should_rotate_before(window_start + 300_000_000_000, 300));
        // Time going backwards always starts a new file.
        assert!(logger.should_rotate_before(start - 1, 300));

        logger.row_groups_written = MAX_ROW_GROUPS_PER_FILE;
        assert!(logger.should_rotate_before(start + 1, 300));

        let _ = logger.close().expect("writer should finalize");
        fs::remove_dir_all(root).expect("temporary directory should be removable");
    }

    /// Window indices are floor multiples of the period since the epoch.
    #[test]
    fn rotation_windows_align_to_epoch_multiples() {
        assert_eq!(rotation_window(0, 300), 0);
        assert_eq!(rotation_window(299_999_999_999, 300), 0);
        assert_eq!(rotation_window(300_000_000_000, 300), 1);
        assert_eq!(rotation_window(-1, 300), -1);
        // A zero period must not divide by zero.
        assert_eq!(rotation_window(5_000_000_000, 0), 5);
    }

    /// A zstd, delta-encoded file holds one row group and reads back exactly.
    #[test]
    fn compressed_file_round_trips_exactly_through_the_exporter_reader() {
        use parquet::basic::{Compression, Encoding};
        use parquet::record::RowAccessor;

        let root = temporary_parquet_root("round-trip");
        let date = NaiveDate::from_ymd_opt(2026, 9, 14).expect("valid date");
        let mut logger = ParquetLogger::new(1001, 8, 1, date, CalibrationSpec::default(), &root);
        let start = 1_789_401_600_000_000_000_i64;
        let rows = 60_000; // 30 s at 2 kHz
        let expected: Vec<(i64, f64)> = (0..rows)
            .map(|i| {
                // 16-bit ADC steps around 3.72 V, like the live pressure channels.
                let counts = (i * 7919 % 97) as f64;
                (start + i as i64 * 500_000, 3.70 + counts * 0.000_315_6)
            })
            .collect();
        for (ts, v) in &expected {
            logger.write_row(*ts, *v);
        }
        let path = logger.close().expect("writer should finalize");

        let reader = SerializedFileReader::new(fs::File::open(&path).expect("open"))
            .expect("compressed file should be readable");
        let meta = reader.metadata();
        assert_eq!(meta.num_row_groups(), 1);
        let rg = meta.row_group(0);
        assert!(matches!(rg.column(0).compression(), Compression::ZSTD(_)));
        assert!(
            rg.column(0)
                .encodings()
                .contains(&Encoding::DELTA_BINARY_PACKED)
        );
        assert!(rg.column(1).encodings().contains(&Encoding::RLE_DICTIONARY));

        // Read back exactly as the exporter does.
        let actual: Vec<(i64, f64)> = reader
            .get_row_iter(None)
            .expect("row iterator")
            .map(|row| {
                let row = row.expect("row");
                (
                    row.get_long(0).expect("ts"),
                    row.get_double(1).expect("value"),
                )
            })
            .collect();
        assert_eq!(actual, expected);
        fs::remove_dir_all(root).expect("temporary directory should be removable");
    }

    /// Builds a FlatBuffer scan payload like the streamer publishes.
    fn scan_payload(first_ns: u64, interval_ns: u64, sequence: u64, values: &[f64]) -> Vec<u8> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let values = builder.create_vector(values);
        let scan = sampler::Scan::create(
            &mut builder,
            &sampler::ScanArgs {
                first_sample_unix_ns: first_ns,
                sample_interval_ns: interval_ns,
                actual_scan_rate_hz: 1e9 / interval_ns as f64,
                sequence,
                values: Some(values),
            },
        );
        builder.finish(scan, None);
        builder.finished_data().to_vec()
    }

    /// End-to-end check against a real NATS server:
    /// an existing consumer with the old settings is updated in place, messages
    /// stay unacked until their file is closed, and files split at aligned
    /// windows. Run with `AVENA_TEST_NATS_URL=nats://127.0.0.1:4222`.
    #[tokio::test]
    #[ignore = "needs a JetStream-enabled nats-server; set AVENA_TEST_NATS_URL"]
    async fn archiver_defers_acks_until_file_close_on_real_nats() {
        let Ok(url) = std::env::var("AVENA_TEST_NATS_URL") else {
            return;
        };
        let client = async_nats::connect(url)
            .await
            .expect("connect to test NATS");
        let js = jetstream::new(client);
        let stream_name = format!("test-{}", Uuid::new_v4().simple());
        let subject = format!("{stream_name}.ch08");
        let stream = js
            .create_stream(jetstream::stream::Config {
                name: stream_name.clone(),
                subjects: vec![format!("{stream_name}.>")],
                ..Default::default()
            })
            .await
            .expect("create stream");

        // A durable consumer as created by the currently deployed archiver.
        let consumer_name = "archiver-test-current".to_string();
        stream
            .create_consumer(pull::Config {
                durable_name: Some(consumer_name.clone()),
                filter_subject: subject.clone(),
                ack_policy: jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                ..Default::default()
            })
            .await
            .expect("create old-style consumer");

        let root = temporary_parquet_root("nats-integration");
        let logger = spawn_channel_logger(
            js.clone(),
            stream_name.clone(),
            consumer_name.clone(),
            subject.clone(),
            1001,
            8,
            300,
            CalibrationSpec::default(),
            root.clone(),
        )
        .await
        .expect("spawn channel logger");

        let mut info_stream = js.get_stream(&stream_name).await.expect("stream");
        let info = info_stream
            .consumer_info(&consumer_name)
            .await
            .expect("consumer info");
        let expected = archiver_consumer_config(&consumer_name, &subject, 300);
        assert_eq!(
            info.config.ack_wait, expected.ack_wait,
            "ack_wait updated in place"
        );
        assert_eq!(info.config.max_ack_pending, CONSUMER_MAX_ACK_PENDING);

        // Five 1 s messages at 100 Hz, all inside one aligned window.
        let interval = 10_000_000_u64;
        let window = 1_790_000_100_u64 / 300 * 300; // an aligned five-minute boundary, in seconds
        let base = (window + 100) * 1_000_000_000;
        let values = vec![3.72; 100];
        for k in 0..5_u64 {
            js.publish(
                subject.clone(),
                scan_payload(base + k * 1_000_000_000, interval, k, &values).into(),
            )
            .await
            .expect("publish")
            .await
            .expect("stored");
        }

        async fn pending_of(s: &mut jetstream::stream::Stream, name: &str) -> (usize, u64) {
            let info = s.consumer_info(name).await.expect("consumer info");
            (info.num_ack_pending, info.num_pending)
        }
        let mut waited = 0;
        while pending_of(&mut info_stream, &consumer_name).await != (5, 0) && waited < 50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += 1;
        }
        assert_eq!(
            pending_of(&mut info_stream, &consumer_name).await,
            (5, 0),
            "open file holds its acks"
        );

        // A message in the next window closes the first file and acks its five
        // messages; the new message waits for its own file.
        let next = (window + 300) * 1_000_000_000;
        js.publish(
            subject.clone(),
            scan_payload(next, interval, 5, &values).into(),
        )
        .await
        .expect("publish")
        .await
        .expect("stored");
        waited = 0;
        while pending_of(&mut info_stream, &consumer_name).await != (1, 0) && waited < 50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += 1;
        }
        assert_eq!(
            pending_of(&mut info_stream, &consumer_name).await,
            (1, 0),
            "closed file's messages acked"
        );

        // Graceful stop closes the second file and acks the rest.
        logger.stop().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            pending_of(&mut info_stream, &consumer_name).await,
            (0, 0),
            "stop settles every message"
        );

        let mut files: Vec<PathBuf> = Vec::new();
        for day in fs::read_dir(root.join("asset1001")).expect("asset dir") {
            for f in fs::read_dir(day.expect("day").path().join("ch08")).expect("channel dir") {
                files.push(f.expect("file").path());
            }
        }
        files.sort();
        assert_eq!(files.len(), 2, "one file per aligned window: {files:?}");
        let rows: Vec<i64> = files
            .iter()
            .map(|p| {
                SerializedFileReader::new(fs::File::open(p).expect("open"))
                    .expect("readable")
                    .metadata()
                    .file_metadata()
                    .num_rows()
            })
            .collect();
        assert_eq!(rows.iter().sum::<i64>(), 600);

        js.delete_stream(&stream_name).await.expect("delete stream");
        fs::remove_dir_all(root).expect("temporary directory should be removable");
    }

    /// The consumer's `ack_wait` and `max_ack_pending` leave room for an open file.
    #[test]
    fn consumer_ack_wait_outlasts_a_file_and_pending_limit_has_headroom() {
        let cfg = archiver_consumer_config("archiver-test", "avenars.test.ch08", 300);
        assert_eq!(cfg.durable_name.as_deref(), Some("archiver-test"));
        assert_eq!(cfg.filter_subject, "avenars.test.ch08");
        assert!(cfg.ack_wait > Duration::from_secs(300) + IDLE_CLOSE_AFTER + IDLE_CHECK_INTERVAL);
        assert!(cfg.max_ack_pending > MAX_PENDING_ACKS_PER_FILE as i64);
    }

    /// Startup renames `.inprogress` files and leaves completed `.parquet` files alone.
    #[test]
    fn startup_quarantines_unfinished_without_scanning_completed_parquet() {
        let root = temporary_parquet_root("quarantine");
        let channel_dir = root.join("asset1001/2026-08-05/ch11");
        fs::create_dir_all(&channel_dir).expect("channel directory should be created");
        fs::write(channel_dir.join("part-0001.parquet.inprogress"), b"partial")
            .expect("unfinished file should be written");
        fs::write(channel_dir.join("part-0002.parquet"), b"not parquet")
            .expect("corrupt file should be written");

        let quarantined = quarantine_incomplete_files(&root).expect("quarantine should succeed");
        assert_eq!(quarantined.len(), 1);
        assert!(quarantined.iter().all(|path| path.exists()));
        assert!(!channel_dir.join("part-0001.parquet.inprogress").exists());
        assert!(channel_dir.join("part-0002.parquet").exists());
        fs::remove_dir_all(root).expect("temporary directory should be removable");
    }
}
