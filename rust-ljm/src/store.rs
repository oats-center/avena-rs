use std::{fs, path::Path, sync::Arc};
use parquet::{
    column::writer::ColumnWriter,
    data_type::ByteArray,
    file::{
        properties::WriterProperties,
        writer::SerializedFileWriter,
    },
    schema::parser::parse_message_type,
};
use async_nats::{self, jetstream};
use futures_util::StreamExt;
use tokio::time::Duration;
use chrono::{Utc, NaiveDate};

mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs"); // adjust path if needed
}
use sample_data_generated::sampler;

use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
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
            let cw = scw.untyped();
            if let ColumnWriter::ByteArrayColumnWriter(typed) = cw {
                let values: Vec<ByteArray> =
                    self.buffer.iter().map(|(ts, _)| ByteArray::from(ts.as_str())).collect();
                typed.write_batch(&values, None, None).unwrap();
            }
            scw.close().unwrap();
        }

        // column 1: values
        {
            let mut scw = rg.next_column().unwrap().expect("value col");
            let cw = scw.untyped();
            if let ColumnWriter::DoubleColumnWriter(typed) = cw {
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


fn spawn_channel_logger(
    nc: async_nats::Client,
    subject: String,
    asset: u32,
    channel: u8,
    rotate_secs: u64,
) {
    tokio::spawn(async move {
        let mut sub = nc.subscribe(subject.clone()).await.unwrap();
        println!("[logger] Subscribed to {subject}");

        let mut ticker = tokio::time::interval(Duration::from_secs(rotate_secs));
        let mut logger: Option<ParquetLogger> = None;
        let mut file_index = 0;

        loop {
            tokio::select! {
                Some(msg) = sub.next() => {
                    if let Ok(scan) = flatbuffers::root::<sampler::Scan>(&msg.payload) {
                        if let Some(ts) = scan.timestamp() {
                            if let Some(vals) = scan.values() {
                                let today = Utc::now().date_naive();

                                // rollover if date changed
                                if logger.as_ref().map(|l| l.date != today).unwrap_or(true) {
                                    if let Some(l) = logger.take() {
                                        l.close();
                                        println!("[logger] Closed file {}", file_index);
                                    }
                                    file_index = 1;
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
    });
}



#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //
    // Step 1: bootstrap connection (local NATS by default)
    //
    let bootstrap_url =
        std::env::var("NATS_BOOTSTRAP_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let nc = async_nats::connect(&bootstrap_url).await?;
    let js = jetstream::new(nc.clone());

    //
    // Step 2: load config from well-known KV key
    //
    let store = js.get_key_value("sampler_cfg").await?;
    let entry = store.entry("active").await?.ok_or("KV key not found")?;
    let cfg: SampleConfig = serde_json::from_slice(&entry.value)?;
    println!("[logger] Loaded config: {:?}", cfg);

    //
    // Step 3: reconnect if config points to different NATS cluster
    //
    let nc = if cfg.nats_url != bootstrap_url {
        println!("[logger] Reconnecting to {}", cfg.nats_url);
        async_nats::connect(&cfg.nats_url).await?
    } else {
        nc
    };

    
    for ch in &cfg.channels {
        let subject = format!(
            "{}.{:03}.data.ch{:02}",
            cfg.nats_subject, cfg.asset_number, ch
        );
        spawn_channel_logger(
            nc.clone(),
            subject,
            cfg.asset_number,
            *ch,
            cfg.rotate_secs,
        ); 
    }


    tokio::signal::ctrl_c().await?;
    println!("Shutting down logger...");
    Ok(())
}
