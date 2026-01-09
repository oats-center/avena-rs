use async_nats::{self, ConnectOptions, ServerAddr};
use flatbuffers::root;
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

// Import your generated FlatBuffers schema
mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs"); // path relative to examples/
}
use sample_data_generated::sampler;

fn extract_channel_token(subject: &str) -> Option<String> {
    subject.split('.').last().map(|s| s.to_string())
}

fn pad_asset(n: u32) -> String {
    format!("{n:03}")
}

fn open_csv_for_channel(out_dir: &Path, asset: u32, ch_token: &str) -> std::io::Result<File> {
    let fname = format!("labjack_{}_{}.csv", pad_asset(asset), ch_token);
    let path = out_dir.join(fname);
    let need_header = !path.exists();

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    if need_header || file.metadata()?.len() == 0 {
        writeln!(file, "timestamp,values")?;
    }
    Ok(file)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // match JSON config keys
    let subject_prefix = std::env::var("NATS_SUBJECT").unwrap_or_else(|_| "avenabox".to_string());
    let asset_number: u32 = std::env::var("ASSET_NUMBER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    let out_dir_str = std::env::var("OUTPUT_DIR").unwrap_or_else(|_| "outputs".to_string());
    let out_dir = PathBuf::from(&out_dir_str);
    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir)?;
        println!("Created output directory: {}", out_dir.display());
    } else {
        println!("Using output directory: {}", out_dir.display());
    }

    // Subscribe to all per-channel subjects for this asset
    let wildcard = format!("{}.{}.data.*", subject_prefix, pad_asset(asset_number));
    println!("Subscribing to subject '{}'", wildcard);

    // Build server list
    let servers: Vec<ServerAddr> = vec![
        "nats://nats1.oats:4222".parse()?,
        "nats://nats2.oats:4222".parse()?,
        "nats://nats3.oats:4222".parse()?,
    ];

    // Connect using creds
    let creds_path = std::env::var("NATS_CREDS_FILE")
        .unwrap_or_else(|_| "apt.creds".into());
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| format!("Failed to load creds: {}", e))?;

    let nc = opts
        .connect(servers)
        .await
        .map_err(|e| format!("NATS connect failed: {}", e))?;

    println!("Connected to NATS with creds, subscribed at '{}'", wildcard);

    let mut sub = nc.subscribe(wildcard.clone()).await?;
    let mut files: HashMap<String, File> = HashMap::new();

    while let Some(msg) = sub.next().await {
        let ch_token = match extract_channel_token(&msg.subject) {
            Some(tok) => tok,
            None => {
                eprintln!("Subject '{}' missing channel token; skipping.", msg.subject);
                continue;
            }
        };

        match root::<sampler::Scan>(&msg.payload) {
            Ok(scan) => {
                let timestamp = scan.timestamp().unwrap_or("<no ts>");
                let values = scan.values().unwrap();
                let values_vec: Vec<f64> = values.iter().collect();

                let out_dir_clone = out_dir.clone();
                let file = files.entry(ch_token.clone()).or_insert_with(move || {
                    open_csv_for_channel(&out_dir_clone, asset_number, &ch_token)
                        .expect("failed to open per-channel csv")
                });

                let values_str = values_vec
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(";");

                writeln!(file, "{},{}", timestamp, values_str)?;
                file.flush()?;
            }
            Err(e) => {
                eprintln!(
                    "FlatBuffer decode error ({:?}) for subject '{}'",
                    e, msg.subject
                );
            }
        }
    }

    Ok(())
}
