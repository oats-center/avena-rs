//! Archives LabJack scan batches from NATS JetStream into Parquet files.
//!
//! The archiver reads the same dashboard configuration as the streamer,
//! subscribes to each configured channel with a durable pull consumer, decodes
//! FlatBuffer scan payloads, reconstructs per-sample timestamps, and writes
//! partitioned Parquet files with calibration metadata.

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
    column::writer::ColumnWriter,
    file::{
        metadata::KeyValue, properties::WriterProperties, reader::SerializedFileReader,
        writer::SerializedFileWriter,
    },
    schema::parser::parse_message_type,
};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{oneshot, watch};
use tokio::time::Duration;

mod calibration;
mod nats_config;
mod subjects;
mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs");
}
use sample_data_generated::sampler;

use calibration::CalibrationSpec;
use serde::{Deserialize, Serialize};

/// Error type used by the archiver for fallible async setup and IO paths.
type DynError = Box<dyn std::error::Error + Send + Sync>;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
/// Raw top-level dashboard configuration loaded from NATS KV.
struct NestedConfig {
    labjack_name: String,
    asset_number: u32,
    max_channels: u32,
    #[serde(default)]
    site_id: Option<String>,
    #[serde(default)]
    box_id: Option<String>,
    #[serde(default)]
    source_type: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    nats_subject: String,
    nats_stream: String,
    rotate_secs: u64,
    sensor_settings: SensorConfig,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
/// Raw sensor settings section from the dashboard configuration.
///
/// The archiver keeps calibration definitions from this section and otherwise
/// normalizes the same sampling fields used by the streamer.
struct SensorConfig {
    #[serde(rename = "scans_per_read", alias = "scan_rate")]
    scans_per_read: i32,
    #[serde(rename = "scan_rate_hz", alias = "sampling_rate")]
    scan_rate_hz: f64,
    channels_enabled: Vec<u8>,
    gains: i32,
    data_formats: Vec<String>,
    measurement_units: Vec<String>,
    labjack_on_off: bool,
    calibrations: Option<HashMap<String, CalibrationSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Normalized archiver configuration used to manage channel loggers.
///
/// This shape combines stream identity, channel list, rotation cadence, and
/// parsed per-channel calibrations.
struct SampleConfig {
    scans_per_read: i32,
    scan_rate_hz: f64,
    channels: Vec<u8>,
    asset_number: u32,
    labjack_name: String,
    site_id: Option<String>,
    box_id: Option<String>,
    source_type: Option<String>,
    source_id: Option<String>,
    nats_subject: String,
    nats_stream: String,
    rotate_secs: u64,
    calibrations: HashMap<u8, CalibrationSpec>,
}

impl From<(SensorConfig, &SampleConfig)> for SampleConfig {
    /// Replaces sensor-level settings while preserving source identity fields.
    ///
    /// This conversion is useful when a dashboard update changes only the
    /// nested sensor section but the archiver should keep the existing
    /// namespace and asset context.
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
fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone)]
/// Connection and key details for mirroring dashboard config from central NATS.
struct CentralKvSyncConfig {
    servers: Vec<async_nats::ServerAddr>,
    creds_path: String,
    bucket: String,
    key: String,
    domain: Option<String>,
}

/// Builds optional central KV mirroring settings from environment variables.
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
async fn connect_nats_with_creds(
    servers: Vec<async_nats::ServerAddr>,
    creds_path: String,
) -> Result<async_nats::Client, DynError> {
    let opts = ConnectOptions::with_credentials_file(creds_path).await?;
    Ok(opts.connect(servers).await?)
}

/// Opens or creates the KV bucket that contains dashboard configuration.
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
/// configuration does not replace the local copy.
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
/// that parse as a valid dashboard configuration.
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
/// date, channel, and monotonically increasing part index.
struct ParquetLogger {
    writer: SerializedFileWriter<fs::File>,
    inprogress_path: PathBuf,
    final_path: PathBuf,
    buffer: Vec<(i64, f64)>,
    max_rows: usize,
    date: NaiveDate,
    asset: u32,
    channel: u8,
    file_index: usize,
}

/// Runtime state for one active channel consumer and writer task.
///
/// The archiver keeps this state so KV updates can abort removed channels,
/// rotate calibration metadata, or respawn consumers when subject identity
/// changes.
struct ChannelLogger {
    handle: tokio::task::JoinHandle<()>,
    stop_tx: Option<oneshot::Sender<()>>,
    calibration_tx: watch::Sender<CalibrationSpec>,
    calibration: CalibrationSpec,
    subject: String,
    stream_name: String,
    consumer_name: String,
    asset: u32,
    rotate_secs: u64,
}

impl ChannelLogger {
    /// Requests an orderly writer shutdown and waits for the Parquet footer.
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
    /// The calibration spec is written into file-level key-value metadata so
    /// exported data can be interpreted without consulting the live config.
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

        let message_type = "
            message schema {
                REQUIRED INT64 timestamp_unix_ns;
                REQUIRED DOUBLE value;
            }
        ";
        let schema = Arc::new(parse_message_type(message_type).unwrap());
        let calibration_json =
            serde_json::to_string(&calibration).unwrap_or_else(|_| "{}".to_string());
        let props = Arc::new(
            WriterProperties::builder()
                .set_key_value_metadata(Some(vec![KeyValue::new(
                    "calibration".to_string(),
                    calibration_json,
                )]))
                .build(),
        );
        let file = fs::File::create(&inprogress_path).unwrap();
        let writer = SerializedFileWriter::new(file, schema, props).unwrap();

        Self {
            writer,
            inprogress_path,
            final_path,
            buffer: Vec::with_capacity(1000),
            max_rows: 1000,
            date,
            asset,
            channel,
            file_index,
        }
    }

    /// Buffers one timestamped value and flushes when the row group is full.
    fn write_row(&mut self, timestamp_unix_ns: i64, val: f64) {
        self.buffer.push((timestamp_unix_ns, val));
        if self.buffer.len() >= self.max_rows {
            self.flush();
        }
    }

    /// Writes the current buffer as a Parquet row group.
    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let mut rg = self.writer.next_row_group().unwrap();

        // column 0: timestamps
        {
            let mut scw = rg.next_column().unwrap().expect("timestamp col");
            let mut cw = scw.untyped();
            if let ColumnWriter::Int64ColumnWriter(typed) = &mut cw {
                let values: Vec<i64> = self.buffer.iter().map(|(ts, _)| *ts).collect();
                typed.write_batch(&values, None, None).unwrap();
            }
            scw.close().unwrap();
        }

        // column 1: values
        {
            let mut scw = rg.next_column().unwrap().expect("value col");
            let mut cw = scw.untyped();
            if let ColumnWriter::DoubleColumnWriter(typed) = &mut cw {
                let values: Vec<f64> = self.buffer.iter().map(|(_, v)| *v).collect();
                typed.write_batch(&values, None, None).unwrap();
            }
            scw.close().unwrap();
        }

        rg.close().unwrap();
        self.buffer.clear();
    }

    /// Flushes buffered rows and closes the Parquet writer.
    fn close(mut self) -> Result<PathBuf, DynError> {
        self.flush();
        self.writer.close()?;
        fs::rename(&self.inprogress_path, &self.final_path)?;
        Ok(self.final_path)
    }
}

/// Moves unfinished writer files out of the exporter's `.parquet` namespace.
///
/// A power loss can leave a file without a Parquet footer. The original bytes
/// are preserved for diagnosis, while a new part index is used for subsequent
/// acquisition.
fn quarantine_incomplete_files(root: &Path) -> io::Result<Vec<PathBuf>> {
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
            let unfinished = name.ends_with(".parquet.inprogress");
            let unreadable_parquet = name.ends_with(".parquet")
                && match fs::File::open(&path) {
                    Ok(file) => SerializedFileReader::new(file).is_err(),
                    Err(err) => {
                        eprintln!(
                            "[logger] Could not inspect existing Parquet file {}: {err}",
                            path.display()
                        );
                        false
                    }
                };
            if !unfinished && !unreadable_parquet {
                continue;
            }

            let reason = if unfinished { "unfinished" } else { "corrupt" };
            let quarantine_name =
                format!("{name}.{reason}.quarantined-{stamp}-{}", quarantined.len());
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

/// Decodes one FlatBuffer scan payload and writes its samples to Parquet.
///
/// The payload contains the first sample timestamp plus a fixed interval, so
/// this function reconstructs each sample timestamp and rotates files when the
/// UTC date changes.
fn process_scan_payload(
    payload: &[u8],
    channel: u8,
    asset: u32,
    parquet_root: &Path,
    active_calibration: &CalibrationSpec,
    logger: &mut Option<ParquetLogger>,
    file_index: &mut usize,
    last_sequence: &mut Option<u64>,
) {
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
                    .map(|l| l.date != sample_date)
                    .unwrap_or(true)
                {
                    if let Some(l) = logger.take() {
                        match l.close() {
                            Ok(path) => println!("[logger] Closed {}", path.display()),
                            Err(err) => eprintln!(
                                "[logger] Failed to finalize channel {channel:02} part {}: {err}",
                                *file_index
                            ),
                        }
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
}

/// Starts the durable pull consumer and writer task for one channel.
///
/// The returned [`ChannelLogger`] lets the config watcher update calibration
/// metadata or gracefully stop and respawn the task when identity changes.
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
    let consumer = stream
        .get_or_create_consumer(
            consumer_name.as_str(),
            pull::Config {
                durable_name: Some(consumer_name.clone()),
                filter_subject: subject.clone(),
                ack_policy: jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                ..Default::default()
            },
        )
        .await?;

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

        let mut ticker = tokio::time::interval(Duration::from_secs(rotate_secs));
        // `interval` ticks immediately; consume that tick so the first part is
        // created only by data or after a full rotation interval.
        ticker.tick().await;
        let mut logger: Option<ParquetLogger> = None;
        let mut file_index =
            next_file_index(&parquet_root, asset, channel, Utc::now().date_naive());
        let mut active_calibration = calibration_for_task;
        let mut last_sequence: Option<u64> = None;

        loop {
            tokio::select! {
                maybe = messages.next() => {
                    match maybe {
                        Some(Ok(msg)) => {
                            process_scan_payload(
                                &msg.payload,
                                channel,
                                asset,
                                &parquet_root,
                                &active_calibration,
                                &mut logger,
                                &mut file_index,
                                &mut last_sequence,
                            );
                            if let Err(err) = msg.ack().await {
                                eprintln!(
                                    "[logger] Failed to ack JetStream message for channel {channel:02}: {}",
                                    err
                                );
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
                _ = ticker.tick() => {
                    let today = Utc::now().date_naive();
                    if let Some(l) = logger.take() {
                        match l.close() {
                            Ok(path) => println!("[logger] Closed {}", path.display()),
                            Err(err) => eprintln!(
                                "[logger] Failed to finalize channel {channel:02} part {file_index}: {err}"
                            ),
                        }
                    }
                    file_index = next_file_index(&parquet_root, asset, channel, today);
                    logger = Some(ParquetLogger::new(
                        asset,
                        channel,
                        file_index,
                        today,
                        active_calibration.clone(),
                        &parquet_root,
                    ));
                }
                changed = calibration_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let updated = calibration_rx.borrow().clone();
                    if updated != active_calibration {
                        let today = Utc::now().date_naive();
                        if let Some(l) = logger.take() {
                            match l.close() {
                                Ok(path) => println!("[logger] Closed {}", path.display()),
                                Err(err) => eprintln!(
                                    "[logger] Failed to finalize channel {channel:02} part {file_index}: {err}"
                                ),
                            }
                        }
                        file_index = next_file_index(&parquet_root, asset, channel, today);
                        println!(
                            "[logger] Calibration updated for channel {channel:02}; rotating file."
                        );
                        logger = Some(ParquetLogger::new(
                            asset,
                            channel,
                            file_index,
                            today,
                            updated.clone(),
                            &parquet_root,
                        ));
                        active_calibration = updated;
                    }
                }
                _ = &mut stop_rx => {
                    println!("[logger] Graceful stop requested for channel {channel:02}");
                    break;
                }
            }
        }

        if let Some(logger) = logger.take() {
            match logger.close() {
                Ok(path) => println!("[logger] Closed {}", path.display()),
                Err(err) => eprintln!(
                    "[logger] Failed to finalize channel {channel:02} during shutdown: {err}"
                ),
            }
        }
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
fn timestamp_ns_to_utc_date(timestamp_unix_ns: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp_nanos(timestamp_unix_ns).date_naive()
}

#[tokio::main]
/// Starts the archiver service and reacts to configuration updates.
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

    // Step 2: load config from KV
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
    let cfg: SampleConfig = sample_config_from_nested(nested);

    println!("[logger] Loaded config: {:?}", cfg);

    // Step 4: create channel loggers and watch KV changes in the main task so
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
    use parquet::file::reader::FileReader;
    use uuid::Uuid;

    fn temporary_parquet_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rust-ljm-{name}-{}", Uuid::new_v4()))
    }

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

    #[test]
    fn startup_quarantines_unfinished_and_corrupt_files() {
        let root = temporary_parquet_root("quarantine");
        let channel_dir = root.join("asset1001/2026-08-05/ch11");
        fs::create_dir_all(&channel_dir).expect("channel directory should be created");
        fs::write(channel_dir.join("part-0001.parquet.inprogress"), b"partial")
            .expect("unfinished file should be written");
        fs::write(channel_dir.join("part-0002.parquet"), b"not parquet")
            .expect("corrupt file should be written");

        let quarantined = quarantine_incomplete_files(&root).expect("quarantine should succeed");
        assert_eq!(quarantined.len(), 2);
        assert!(quarantined.iter().all(|path| path.exists()));
        assert!(!channel_dir.join("part-0001.parquet.inprogress").exists());
        assert!(!channel_dir.join("part-0002.parquet").exists());
        fs::remove_dir_all(root).expect("temporary directory should be removable");
    }
}
