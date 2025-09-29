use std::{fs, path::Path, sync::Arc, collections::HashMap};
use parquet::{
    column::writer::ColumnWriter,
    data_type::ByteArray,
    file::{properties::WriterProperties, writer::SerializedFileWriter},
    schema::parser::parse_message_type,
};
use async_nats::{self, jetstream, ServerAddr};
use async_nats::ConnectOptions;
use async_nats::jetstream::kv::Operation;
use futures_util::StreamExt;
use tokio::time::Duration;
use chrono::{Utc, NaiveDate};

mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs");
}
use sample_data_generated::sampler;

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct NestedConfig {
    labjack_name: String,
    asset_number: u32,
    max_channels: u32,
    nats_subject: String,
    nats_stream: String,
    rotate_secs: u64,
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
    labjack_on_off: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SampleConfig {
    scans_per_read: i32,
    suggested_scan_rate: f64,
    channels: Vec<u8>,
    asset_number: u32,
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
    
    let servers: Vec<ServerAddr> = vec![
        "nats://nats1.oats:4222".parse()?,
        "nats://nats2.oats:4222".parse()?,
        "nats://nats3.oats:4222".parse()?,
    ];

    // Connect using creds
    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "/Users/anugunj/Downloads/apt.creds".into());
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| format!("Failed to load creds: {}", e))?;

    let nc = opts
        .connect(servers)
        .await
        .map_err(|e| format!("NATS connect failed: {}", e))?;

    println!("Connected to NATS via creds!");
    let js = jetstream::new(nc.clone());

    // Step 2: load config from KV
    let bucket = std::env::var("CFG_BUCKET").unwrap_or_else(|_| "avenabox".into());
    let key    = std::env::var("CFG_KEY").unwrap_or_else(|_| "labjackd.config.macbook".into());
    let store = js.get_key_value(bucket.as_str()).await?;
    let entry = store.entry(key.as_str()).await?.ok_or("KV key not found")?;

    let cfg: SampleConfig = match serde_json::from_slice::<NestedConfig>(&entry.value) {
        Ok(nested) => {
            let raw = nested.sensor_settings;
            SampleConfig {
                scans_per_read: raw.scan_rate,
                suggested_scan_rate: raw.sampling_rate,
                channels: raw.channels_enabled.clone(),
                asset_number: nested.asset_number,
                nats_subject: nested.nats_subject,
                nats_stream: nested.nats_stream,
                rotate_secs: nested.rotate_secs,
            }
        }
        Err(_) => {
            let nested = serde_json::from_slice::<NestedConfig>(&entry.value)?;
            let raw = nested.sensor_settings;
            SampleConfig {
                scans_per_read: raw.scan_rate,
                suggested_scan_rate: raw.sampling_rate,
                channels: raw.channels_enabled,
                asset_number: nested.asset_number,
                nats_subject: nested.nats_subject,
                nats_stream: nested.nats_stream,
                rotate_secs: nested.rotate_secs,
            }
        },
    };

    

    println!("[logger] Loaded config: {:?}", cfg);

    // Step 4: spawn dynamic watcher for KV config changes
    let mut watch = store.watch(key.as_str()).await?;
    let mut active: HashMap<u8, tokio::task::JoinHandle<()>> = HashMap::new();

    // initial subscriptions
    for ch in &cfg.channels {
        let subject = format!("{}.{:03}.data.ch{:02}", cfg.nats_subject, cfg.asset_number, ch);
        let h = spawn_channel_logger(nc.clone(), subject, cfg.asset_number, *ch, cfg.rotate_secs);
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
                            .map(|nested| {
                                let raw = nested.sensor_settings;
                                SampleConfig {
                                    scans_per_read: raw.scan_rate,
                                    suggested_scan_rate: raw.sampling_rate,
                                    channels: raw.channels_enabled.clone(),
                                    asset_number: nested.asset_number,
                                    nats_subject: nested.nats_subject,
                                    nats_stream: nested.nats_stream,
                                    rotate_secs: nested.rotate_secs,
                                }
                            })
                            .or_else(|_| {
                                serde_json::from_slice::<NestedConfig>(&entry.value)
                                    .map(|nested| {
                                        let raw = nested.sensor_settings;
                                        SampleConfig {
                                            scans_per_read: raw.scan_rate,
                                            suggested_scan_rate: raw.sampling_rate,
                                            channels: raw.channels_enabled,
                                            asset_number: nested.asset_number,
                                            nats_subject: nested.nats_subject,
                                            nats_stream: nested.nats_stream,
                                            rotate_secs: nested.rotate_secs,
                                        }
                                    })
                            })
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
                                    let subject = format!(
                                        "{}.{:03}.data.ch{:02}",
                                        new_cfg.nats_subject, new_cfg.asset_number, ch
                                    );
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
        }
    });
    

    tokio::signal::ctrl_c().await?;
    println!("Shutting down logger...");
    Ok(())
}
