use async_nats;
use async_nats::ConnectOptions;
use async_nats::jetstream::kv::Operation;
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::StreamExt;
use parquet::{
    column::writer::ColumnWriter,
    file::{metadata::KeyValue, properties::WriterProperties, writer::SerializedFileWriter},
    schema::parser::parse_message_type,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::watch;
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

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
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

#[allow(dead_code)]
struct ParquetLogger {
    writer: SerializedFileWriter<fs::File>,
    buffer: Vec<(i64, f64)>,
    max_rows: usize,
    date: NaiveDate,
    asset: u32,
    channel: u8,
    file_index: usize,
}

struct ChannelLogger {
    handle: tokio::task::JoinHandle<()>,
    calibration_tx: watch::Sender<CalibrationSpec>,
    calibration: CalibrationSpec,
}

impl ParquetLogger {
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
        let file_path = dir.join(format!("part-{:04}.parquet", file_index));

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
        let file = fs::File::create(file_path).unwrap();
        let writer = SerializedFileWriter::new(file, schema, props).unwrap();

        Self {
            writer,
            buffer: Vec::with_capacity(1000),
            max_rows: 1000,
            date,
            asset,
            channel,
            file_index,
        }
    }

    fn write_row(&mut self, timestamp_unix_ns: i64, val: f64) {
        self.buffer.push((timestamp_unix_ns, val));
        if self.buffer.len() >= self.max_rows {
            self.flush();
        }
    }

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

    fn close(mut self) {
        self.flush();
        if let Err(e) = self.writer.close() {
            eprintln!("Failed to close parquet file: {e}");
        }
    }
}

/// Scan channel/day directory to find the next available parquet file index
fn next_file_index(parquet_root: &Path, asset: u32, channel: u8, date: NaiveDate) -> usize {
    let dir = parquet_root
        .join(format!("asset{:03}", asset))
        .join(date.format("%Y-%m-%d").to_string())
        .join(format!("ch{:02}", channel));

    std::fs::create_dir_all(&dir).unwrap();
    let mut max_idx = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        if let Ok(e) = entry {
            if let Some(name) = e.file_name().to_str() {
                if let Some(num) = name
                    .strip_prefix("part-")
                    .and_then(|s| s.strip_suffix(".parquet"))
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    max_idx = max_idx.max(num);
                }
            }
        }
    }
    max_idx + 1
}

fn spawn_channel_logger(
    nc: async_nats::Client,
    subject: String,
    asset: u32,
    channel: u8,
    rotate_secs: u64,
    calibration: CalibrationSpec,
    parquet_root: PathBuf,
) -> ChannelLogger {
    let (calibration_tx, mut calibration_rx) = watch::channel(calibration.clone());
    let calibration_for_task = calibration.clone();
    let handle = tokio::spawn(async move {
        let mut sub = nc.subscribe(subject.clone()).await.unwrap();
        println!("[logger] Subscribed to {subject}");

        let mut ticker = tokio::time::interval(Duration::from_secs(rotate_secs));
        let mut logger: Option<ParquetLogger> = None;
        let mut file_index =
            next_file_index(&parquet_root, asset, channel, Utc::now().date_naive());
        let mut active_calibration = calibration_for_task;
        let mut last_sequence: Option<u64> = None;

        loop {
            tokio::select! {
                Some(msg) = sub.next() => {
                    if let Ok(scan) = flatbuffers::root::<sampler::Scan>(&msg.payload) {
                        let sequence = scan.sequence();
                        match last_sequence {
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
                                    previous,
                                    sequence
                                );
                            }
                            _ => {}
                        }
                        last_sequence = Some(sequence);

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
                                            sequence,
                                            index,
                                            err
                                        );
                                        break;
                                    }
                                };

                                let sample_date = timestamp_ns_to_utc_date(timestamp_unix_ns);
                                if logger.as_ref().map(|l| l.date != sample_date).unwrap_or(true) {
                                    if let Some(l) = logger.take() {
                                        l.close();
                                        println!("[logger] Closed file {}", file_index);
                                    }
                                    file_index = if logger.is_none() && sample_date != Utc::now().date_naive() {
                                        next_file_index(&parquet_root, asset, channel, sample_date)
                                    } else if sample_date == Utc::now().date_naive() {
                                        next_file_index(&parquet_root, asset, channel, sample_date)
                                    } else {
                                        next_file_index(&parquet_root, asset, channel, sample_date)
                                    };
                                    logger = Some(ParquetLogger::new(
                                        asset,
                                        channel,
                                        file_index,
                                        sample_date,
                                        active_calibration.clone(),
                                        &parquet_root,
                                    ));
                                }

                                if let Some(log) = logger.as_mut() {
                                    log.write_row(timestamp_unix_ns, v);
                                }
                            }
                        }
                    }
                }
                _ = ticker.tick() => {
                    let today = Utc::now().date_naive();
                    if let Some(l) = logger.take() {
                        l.close();
                        println!("[logger] Closed file {}", file_index);
                    }
                    file_index += 1;
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
                            l.close();
                            println!("[logger] Closed file {}", file_index);
                            file_index += 1;
                        } else {
                            file_index = next_file_index(&parquet_root, asset, channel, today);
                        }
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
            }
        }
    });

    ChannelLogger {
        handle,
        calibration_tx,
        calibration,
    }
}

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

fn timestamp_ns_to_utc_date(timestamp_unix_ns: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp_nanos(timestamp_unix_ns).date_naive()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let servers = nats_config::servers_from_env()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let parquet_root =
        PathBuf::from(std::env::var("PARQUET_DIR").unwrap_or_else(|_| "parquet".into()));

    // Connect using creds
    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| format!("Failed to load creds: {}", e))?;

    let nc = opts
        .connect(servers)
        .await
        .map_err(|e| format!("NATS connect failed: {}", e))?;

    println!("Connected to NATS via creds!");
    let js = nats_config::jetstream_context(nc.clone());

    // Step 2: load config from KV
    let bucket = std::env::var("CFG_BUCKET").unwrap_or_else(|_| "avenabox".into());
    let key = std::env::var("CFG_KEY").unwrap_or_else(|_| "labjackd.config.macbook".into());
    let store = js.get_key_value(bucket.as_str()).await?;
    let entry = store.entry(key.as_str()).await?.ok_or("KV key not found")?;

    let nested = serde_json::from_slice::<NestedConfig>(&entry.value)?;
    let cfg: SampleConfig = sample_config_from_nested(nested);

    println!("[logger] Loaded config: {:?}", cfg);

    // Step 4: spawn dynamic watcher for KV config changes
    let mut watch = store.watch(key.as_str()).await?;
    let mut active: HashMap<u8, ChannelLogger> = HashMap::new();

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
        let h = spawn_channel_logger(
            nc.clone(),
            subject,
            cfg.asset_number,
            *ch,
            cfg.rotate_secs,
            calibration,
            parquet_root.clone(),
        );
        active.insert(*ch, h);
    }

    tokio::spawn({
        let nc = nc.clone();
        async move {
            println!("[logger] Watching KV for config changes...");
            while let Some(ev) = watch.next().await {
                if let Ok(entry) = ev {
                    if entry.operation == Operation::Put {
                        if let Ok(new_cfg) = serde_json::from_slice::<NestedConfig>(&entry.value)
                            .map(sample_config_from_nested)
                        {
                            println!("[logger] KV config update detected: {:?}", new_cfg);

                            // remove old channels
                            active.retain(|ch, entry| {
                                if new_cfg.channels.contains(ch) {
                                    true
                                } else {
                                    println!("[logger] Removing channel {ch}");
                                    entry.handle.abort();
                                    false
                                }
                            });

                            // add new channels
                            for ch in &new_cfg.channels {
                                if !active.contains_key(ch) {
                                    println!("[logger] Adding channel {ch}");
                                    let subject = subjects::live_labjack_channel_subject(
                                        &new_cfg.nats_subject,
                                        new_cfg.asset_number,
                                        *ch,
                                        new_cfg.site_id.as_deref(),
                                        new_cfg.box_id.as_deref(),
                                        Some(&new_cfg.labjack_name),
                                        new_cfg.source_type.as_deref(),
                                        new_cfg.source_id.as_deref(),
                                    );
                                    let calibration =
                                        new_cfg.calibrations.get(ch).cloned().unwrap_or_default();
                                    let h = spawn_channel_logger(
                                        nc.clone(),
                                        subject,
                                        new_cfg.asset_number,
                                        *ch,
                                        new_cfg.rotate_secs,
                                        calibration,
                                        parquet_root.clone(),
                                    );
                                    active.insert(*ch, h);
                                } else {
                                    let calibration =
                                        new_cfg.calibrations.get(ch).cloned().unwrap_or_default();
                                    let mut needs_respawn = false;
                                    if let Some(entry) = active.get_mut(ch) {
                                        if entry.calibration != calibration {
                                            if entry
                                                .calibration_tx
                                                .send(calibration.clone())
                                                .is_ok()
                                            {
                                                entry.calibration = calibration.clone();
                                            } else {
                                                needs_respawn = true;
                                            }
                                        }
                                    }
                                    if needs_respawn {
                                        if let Some(entry) = active.remove(ch) {
                                            entry.handle.abort();
                                        }
                                        let subject = subjects::live_labjack_channel_subject(
                                            &new_cfg.nats_subject,
                                            new_cfg.asset_number,
                                            *ch,
                                            new_cfg.site_id.as_deref(),
                                            new_cfg.box_id.as_deref(),
                                            Some(&new_cfg.labjack_name),
                                            new_cfg.source_type.as_deref(),
                                            new_cfg.source_id.as_deref(),
                                        );
                                        let h = spawn_channel_logger(
                                            nc.clone(),
                                            subject,
                                            new_cfg.asset_number,
                                            *ch,
                                            new_cfg.rotate_secs,
                                            calibration,
                                            parquet_root.clone(),
                                        );
                                        active.insert(*ch, h);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    println!("Shutting down logger...");
    Ok(())
}
