use std::{fs, path::Path, sync::Arc, collections::HashMap};
use parquet::{
    column::writer::ColumnWriter,
    data_type::ByteArray,
    file::{properties::WriterProperties, writer::SerializedFileWriter},
    schema::parser::parse_message_type,
};
use async_nats::{self, jetstream};
use async_nats::jetstream::kv::Operation;
use futures_util::StreamExt;
use tokio::time::Duration;
use chrono::{Utc, NaiveDate};

mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs"); // adjust path if needed
}
use sample_data_generated::sampler;

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct NestedConfig {
    cabinet_id: String,
    labjack_name: String,
    serial: String,
    sensor_settings: SensorConfig,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct SensorConfig {
    scan_rate: i32,
    sampling_rate: f64,
    channels_enabled: Vec<u8>,
    gains: i32,
    data_formats: Vec<String>,
    measurement_units: Vec<String>,
    labjack_reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SampleConfig {
    scans_per_read: i32,
    suggested_scan_rate: f64,
    channels: Vec<u8>,
    asset_number: u32,
    nats_url: String,
    nats_subject: String,
    nats_stream: String,
    rotate_secs: u64,
}

impl From<(SensorConfig, &SampleConfig)> for SampleConfig {
    fn from((raw, base): (SensorConfig, &SampleConfig)) -> Self {
        SampleConfig {
            scans_per_read: raw.scan_rate,
            suggested_scan_rate: raw.sampling_rate,
            channels: raw.channels_enabled,
            asset_number: base.asset_number,
            nats_url: base.nats_url.clone(),
            nats_subject: base.nats_subject.clone(),
            nats_stream: base.nats_stream.clone(),
            rotate_secs: base.rotate_secs,
        }
    }
}

#[allow(dead_code)]
struct ParquetLogger {
    writer: SerializedFileWriter<fs::File>,
    buffer: Vec<(String, f64)>,
    max_rows: usize,
    date: NaiveDate,
    asset: u32,
    channel: u8,
    file_index: usize,
}

impl ParquetLogger {
    fn new(asset: u32, channel: u8, file_index: usize, date: NaiveDate) -> Self {
        let dir = Path::new("parquet")
            .join(format!("asset{:03}", asset))
            .join(date.format("%Y-%m-%d").to_string())
            .join(format!("ch{:02}", channel));

        fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join(format!("part-{:04}.parquet", file_index));

        let message_type = "
            message schema {
                REQUIRED BINARY timestamp (UTF8);
                REQUIRED DOUBLE value;
            }
        ";
        let schema = Arc::new(parse_message_type(message_type).unwrap());
        let props = Arc::new(WriterProperties::builder().build());
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

    fn write_row(&mut self, ts: &str, val: f64) {
        self.buffer.push((ts.to_string(), val));
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
            if let ColumnWriter::ByteArrayColumnWriter(typed) = &mut cw {
                let values: Vec<ByteArray> =
                    self.buffer.iter().map(|(ts, _)| ByteArray::from(ts.as_str())).collect();
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
fn next_file_index(asset: u32, channel: u8, date: NaiveDate) -> usize {
    let dir = Path::new("parquet")
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
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut sub = nc.subscribe(subject.clone()).await.unwrap();
        println!("[logger] Subscribed to {subject}");

        let mut ticker = tokio::time::interval(Duration::from_secs(rotate_secs));
        let mut logger: Option<ParquetLogger> = None;
        let mut file_index = next_file_index(asset, channel, Utc::now().date_naive());

        loop {
            tokio::select! {
                Some(msg) = sub.next() => {
                    if let Ok(scan) = flatbuffers::root::<sampler::Scan>(&msg.payload) {
                        if let Some(ts) = scan.timestamp() {
                            if let Some(vals) = scan.values() {
                                let today = Utc::now().date_naive();

                                // rollover if day changes
                                if logger.as_ref().map(|l| l.date != today).unwrap_or(true) {
                                    if let Some(l) = logger.take() {
                                        l.close();
                                        println!("[logger] Closed file {}", file_index);
                                    }
                                    file_index = 1; // reset numbering for new day
                                    logger = Some(ParquetLogger::new(asset, channel, file_index, today));
                                }

                                if let Some(log) = logger.as_mut() {
                                    let ts = ts.to_string();
                                    for v in vals {
                                        log.write_row(&ts, v);
                                    }
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
                    logger = Some(ParquetLogger::new(asset, channel, file_index, today));
                }
            }
        }
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Step 1: bootstrap NATS
    let bootstrap_url =
        std::env::var("NATS_BOOTSTRAP_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let nc = async_nats::connect(&bootstrap_url).await?;
    let js = jetstream::new(nc.clone());

    // Step 2: load config from KV
    let store = js.get_key_value("avenabox_001").await?;
    let entry = store.entry("labjackd.config.TEST001").await?.ok_or("KV key not found")?;
    let cfg: SampleConfig = match serde_json::from_slice::<NestedConfig>(&entry.value) {
        Ok(nested) => {
            let raw_cfg = nested.sensor_settings;
            let base = SampleConfig {
                scans_per_read: raw_cfg.scan_rate,
                suggested_scan_rate: raw_cfg.sampling_rate,
                channels: raw_cfg.channels_enabled.clone(),
                asset_number: 0,
                nats_url: std::env::var("NATS_URL").unwrap_or_else(|_| bootstrap_url.clone()),
                nats_subject: "avenabox".into(),
                nats_stream: "stream".into(),
                rotate_secs: 60,
            };
            SampleConfig::from((raw_cfg, &base))
        }
        Err(_) => serde_json::from_slice::<SampleConfig>(&entry.value)?,
    };

    println!("[logger] Loaded config: {:?}", cfg);

    // Step 3: reconnect if needed
    let nc = if cfg.nats_url != bootstrap_url {
        println!("[logger] Reconnecting to {}", cfg.nats_url);
        async_nats::connect(&cfg.nats_url).await?
    } else {
        nc
    };

    // Step 4: spawn dynamic watcher for KV config changes
    let mut watch = store.watch("labjackd.config.TEST001").await?;
    let mut active: HashMap<u8, tokio::task::JoinHandle<()>> = HashMap::new();

    // initial subscriptions
    for ch in &cfg.channels {
        let subject = format!("{}.{:03}.data.ch{:02}", cfg.nats_subject, cfg.asset_number, ch);
        let h = spawn_channel_logger(nc.clone(), subject, cfg.asset_number, *ch, cfg.rotate_secs);
        active.insert(*ch, h);
    }

    tokio::spawn(async move {
        println!("[logger] Watching KV for config changes...");
        while let Some(ev) = watch.next().await {
            if let Ok(entry) = ev {
                if entry.operation == Operation::Put {
                    if let Ok(new_cfg) = serde_json::from_slice::<NestedConfig>(&entry.value)
                        .map(|nested| SampleConfig::from((nested.sensor_settings, &cfg)))
                        .or_else(|_| serde_json::from_slice::<SampleConfig>(&entry.value))
                    {
                        println!("[logger] KV config update detected: {:?}", new_cfg);

                        // remove old channels
                        active.retain(|ch, handle| {
                            if new_cfg.channels.contains(ch) {
                                true
                            } else {
                                println!("[logger] Removing channel {ch}");
                                handle.abort();
                                false
                            }
                        });

                        // add new channels
                        for ch in &new_cfg.channels {
                            if !active.contains_key(ch) {
                                println!("[logger] Adding channel {ch}");
                                let subject = format!("{}.{:03}.data.ch{:02}",
                                    new_cfg.nats_subject, new_cfg.asset_number, ch);
                                let h = spawn_channel_logger(
                                    nc.clone(),
                                    subject,
                                    new_cfg.asset_number,
                                    *ch,
                                    new_cfg.rotate_secs,
                                );
                                active.insert(*ch, h);
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
