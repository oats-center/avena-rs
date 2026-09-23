//! Diagnostic subscriber that writes live NATS LabJack samples to CSV files.
//!
//! The streamer publishes each channel's scans as FlatBuffer `sampler::Scan` messages
//! on NATS, and the archiver normally turns them into Parquet. This binary is for
//! inspecting that live stream without running the archiver. It makes a plain
//! (non-JetStream) core NATS subscription to the live subject wildcard from
//! [`subjects::live_labjack_stream_subject`], decodes each scan, and appends one CSV
//! file per channel. It only sees messages published while it is running.
//!
//! Each CSV file is named `labjack_<asset>_<channel>.csv` (for example
//! `labjack_001_ch03.csv`) and has the header `sequence,timestamp,raw_value`. Every
//! sample becomes one row: the scan's sequence number, the sample time as RFC 3339
//! UTC, and the value as published.
//!
//! # Configuration
//!
//! * `NATS_SUBJECT` - Subject root. Default: `avenabox`.
//! * `ASSET_NUMBER` - Asset number used in CSV file names. Unparseable values fall
//!   back to the default. Default: `1`.
//! * `SITE_ID` - Site ID for the structured subject layout.
//! * `BOX_ID` - Box ID for the structured subject layout.
//! * `LABJACK_NAME` - LabJack name, used as the source when `SOURCE_ID` is unset.
//! * `SOURCE_TYPE` - Read and passed on, but does not affect the subject.
//! * `SOURCE_ID` - Source ID for the structured subject layout.
//! * `OUTPUT_DIR` - Directory for the CSV files, created if missing. Default:
//!   `outputs`.
//! * `NATS_SERVERS` - Comma-separated NATS server URLs. Default:
//!   `nats://127.0.0.1:4222`.
//! * `NATS_CREDS_FILE` - NATS credentials file. Default: `apt.creds`.
//!
//! With the defaults the subscription is the legacy wildcard `avenabox.*.data.*`,
//! which matches every asset, while the file names always use `ASSET_NUMBER`.
//! Setting any of `SITE_ID`, `BOX_ID` or `SOURCE_ID`, or `NATS_SUBJECT=avenars`,
//! switches to `<root>.<site>.<box>.<source>.live.*`.

use async_nats::{self, ConnectOptions};
use flatbuffers::root;
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs");
}
mod nats_config;
mod subjects;
use sample_data_generated::sampler;

/// Extracts the final subject token, which is expected to be the channel name.
///
/// # Arguments
///
/// * `subject` - Subject of a received message.
///
/// # Returns
///
/// The text after the last `.` (the whole subject if it has no `.`). With
/// [`str::split`] this is always `Some`, so the `None` case in [`main`] is not reached
/// in practice.
///
/// # Examples
///
/// ```text
/// extract_channel_token("avenars.i69.i69-mu1.i69-lj2.live.ch11") -> Some("ch11")
/// extract_channel_token("avenabox.1456.data.ch03")              -> Some("ch03")
/// ```
fn extract_channel_token(subject: &str) -> Option<String> {
    subject.split('.').last().map(|s| s.to_string())
}

/// Opens or creates the per-channel CSV file and writes its header if needed.
///
/// The file is `<out_dir>/labjack_<asset>_<ch_token>.csv`, with the asset formatted by
/// [`subjects::pad_asset`], and is opened for appending. The header
/// `sequence,timestamp,raw_value` is written when the file did not exist or is empty.
///
/// # Arguments
///
/// * `out_dir` - Output directory; must already exist.
/// * `asset` - Asset number used in the file name.
/// * `ch_token` - Channel token from the subject, for example `ch03`.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or created, its metadata cannot be
/// read, or the header cannot be written.
fn open_csv_for_channel(out_dir: &Path, asset: u32, ch_token: &str) -> std::io::Result<File> {
    let fname = format!("labjack_{}_{}.csv", subjects::pad_asset(asset), ch_token);
    let path = out_dir.join(fname);
    let need_header = !path.exists();

    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    if need_header || file.metadata()?.len() == 0 {
        writeln!(file, "sequence,timestamp,raw_value")?;
    }
    Ok(file)
}

/// Converts a Unix nanosecond timestamp into RFC 3339 text in UTC.
///
/// # Arguments
///
/// * `timestamp_unix_ns` - Time since the Unix epoch, in nanoseconds.
///
/// # Returns
///
/// The formatted time, for example `2026-09-01T00:00:00.000500+00:00`. Every `i64`
/// value is representable, so this always returns `Some`.
fn timestamp_unix_ns_to_rfc3339(timestamp_unix_ns: i64) -> Option<String> {
    Some(chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(timestamp_unix_ns).to_rfc3339())
}

#[tokio::main]
/// Starts the live NATS subscriber and appends decoded samples to CSV.
///
/// Reads the configuration listed in the module docs, creates the output directory,
/// connects to NATS with the credentials file, and subscribes to the live wildcard.
/// For each message it decodes a `sampler::Scan` and writes one row per value, with
/// sample `i` timestamped `first_sample_unix_ns + i * sample_interval_ns`. CSV files are
/// opened on first use per channel token and kept open. The file is flushed after each
/// scan.
///
/// Messages that fail FlatBuffer decoding are logged to stderr and skipped. Scans
/// without a `values` vector are skipped silently. If a timestamp does not fit in
/// `i64`, the rest of that scan is dropped with a message on stderr.
///
/// Runs until the subscription ends.
///
/// # Errors
///
/// Returns an error if the output directory cannot be created, `NATS_SERVERS` is
/// invalid, the credentials file cannot be loaded, the connection or subscription
/// fails, or writing or flushing a CSV file fails.
///
/// # Panics
///
/// Panics if a per-channel CSV file cannot be opened or its header written.
async fn main() -> Result<(), Box<dyn Error>> {
    // match JSON config keys
    let subject_prefix = std::env::var("NATS_SUBJECT").unwrap_or_else(|_| "avenabox".to_string());
    let asset_number: u32 = std::env::var("ASSET_NUMBER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let site_id = std::env::var("SITE_ID").ok();
    let box_id = std::env::var("BOX_ID").ok();
    let labjack_name = std::env::var("LABJACK_NAME").ok();
    let source_type = std::env::var("SOURCE_TYPE").ok();
    let source_id = std::env::var("SOURCE_ID").ok();

    let out_dir_str = std::env::var("OUTPUT_DIR").unwrap_or_else(|_| "outputs".to_string());
    let out_dir = PathBuf::from(&out_dir_str);
    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir)?;
        println!("Created output directory: {}", out_dir.display());
    } else {
        println!("Using output directory: {}", out_dir.display());
    }

    // Subscribe to all per-channel subjects for this asset
    let wildcard = subjects::live_labjack_stream_subject(
        &subject_prefix,
        site_id.as_deref(),
        box_id.as_deref(),
        labjack_name.as_deref(),
        source_type.as_deref(),
        source_id.as_deref(),
    );
    println!("Subscribing to subject '{}'", wildcard);

    let servers = nats_config::servers_from_env()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    // Connect using creds
    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
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
                let values = match scan.values() {
                    Some(values) => values,
                    None => continue,
                };
                let sequence = scan.sequence();
                let first_sample_unix_ns = scan.first_sample_unix_ns();
                let sample_interval_ns = scan.sample_interval_ns();

                let out_dir_clone = out_dir.clone();
                let file = files.entry(ch_token.clone()).or_insert_with(move || {
                    open_csv_for_channel(&out_dir_clone, asset_number, &ch_token)
                        .expect("failed to open per-channel csv")
                });

                for (index, value) in values.iter().enumerate() {
                    let timestamp_unix_ns = (first_sample_unix_ns as u128)
                        .saturating_add((sample_interval_ns as u128).saturating_mul(index as u128));
                    let timestamp_unix_ns = match i64::try_from(timestamp_unix_ns) {
                        Ok(ts) => ts,
                        Err(_) => {
                            eprintln!(
                                "timestamp overflow for subject '{}' sequence {} sample {}",
                                msg.subject, sequence, index
                            );
                            break;
                        }
                    };
                    let Some(timestamp) = timestamp_unix_ns_to_rfc3339(timestamp_unix_ns) else {
                        continue;
                    };
                    writeln!(file, "{},{},{}", sequence, timestamp, value)?;
                }
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
