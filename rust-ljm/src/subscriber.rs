use async_nats;
use flatbuffers::root;
use futures_util::stream::StreamExt; // for .next()
// use serde_json::json; // uncomment if doing verbose print
use std::collections::HashMap;
use std::env;
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

fn pad_asset(n: u32) -> String { format!("{n:03}") }

fn extract_channel_token(subject: &str) -> Option<String> {
    subject.split('.').last().map(|s| s.to_string())
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
    let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://0.0.0.0:4222".to_string());
    let subject_prefix = env::var("SUBJECT_PREFIX").unwrap_or_else(|_| "labjack".to_string());
    let asset_number: u32 = env::var("ASSET_NUMBER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    let out_dir_str = env::var("OUTPUT_DIR").unwrap_or_else(|_| "outputs".to_string());
    let out_dir = PathBuf::from(&out_dir_str);
    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir)?;
        println!("Created output directory: {}", out_dir.display());
    } else {
        println!("Using output directory: {}", out_dir.display());
    }

    // Subscribe to all per-channel subjects for this asset
    let wildcard = format!("{}.{}.data.*", subject_prefix, pad_asset(asset_number));

    let nc = async_nats::connect(&nats_url).await?;
    println!("Connected to NATS at {}", nats_url);

    let mut sub = nc.subscribe(wildcard.clone()).await?;
    println!("Subscribed to '{}'", wildcard);


    let mut files: HashMap<String, File> = HashMap::new();

    while let Some(msg) = sub.next().await {
        // Determine channel token from subject (e.g., "ch01")
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

                // For quick sanity/logging
                // println!(
                //     "[{}] {}  count={}  first={:?}",
                //     ch_token,
                //     timestamp,
                //     values_vec.len(),
                //     values_vec.first().copied()
                // );

                // Optional JSON debug
                // let json_obj = json!({
                //     "subject": msg.subject,
                //     "channel": ch_token,
                //     "timestamp": timestamp,
                //     "values": values_vec
                // });
                // println!("As JSON: {}", json_obj);

                // Open (or reuse) CSV for this channel inside output dir
                let out_dir_clone = out_dir.clone();
                let file = files
                    .entry(ch_token.clone())
                    .or_insert_with(move || {
                        open_csv_for_channel(&out_dir_clone, asset_number, &ch_token)
                            .expect("failed to open per-channel csv")
                    });

                // Append one line per batch: timestamp, and all values joined by ';'
                let values_str = values_vec
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(";");

                writeln!(file, "{},{}", timestamp, values_str)?;
                file.flush()?;
            }
            Err(_) => {
                eprintln!(
                    "Failed to decode FlatBuffer payload for subject '{}'",
                    msg.subject
                );
            }
        }
    }

    Ok(())
}
