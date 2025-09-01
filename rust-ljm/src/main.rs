use tokio::sync::{watch, mpsc};
use serde::{Deserialize, Serialize};
use chrono::{Utc, TimeZone};
use chrono_tz::America::New_York;
use tokio::time::Duration;

use ljmrs::{LJMLibrary, LJMError};
use ljmrs::handle::{DeviceType, ConnectionType};

use flatbuffers::FlatBufferBuilder;
use async_nats::jetstream::{self, kv, stream::Config as StreamConfig};
use async_nats::jetstream::kv::Operation;
use futures_util::StreamExt;

mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs");
}
use sample_data_generated::sampler::{self, ScanArgs};

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

struct LabJackGuard {
    handle: i32,
}

impl Drop for LabJackGuard {
    fn drop(&mut self) {
        let _ = LJMLibrary::stream_stop(self.handle);
        let _ = LJMLibrary::close_jack(self.handle);
    }
}

fn pad_channel(ch: u8) -> String {
    format!("ch{ch:02}")
}

fn pad_asset(n: u32) -> String {
    format!("{n:03}")
}

fn channel_subject(prefix: &str, asset: u32, ch: u8) -> String {
    format!("{}.{}.data.{}", prefix, pad_asset(asset), pad_channel(ch))
}

fn stream_subject_wildcard(prefix: &str, asset: u32) -> String {
    format!("{}.{}.data.*", prefix, pad_asset(asset))
}

async fn ensure_stream_exists(
    js: &jetstream::Context,
    stream_name: &str,
    subject: &str,
) -> Result<(), LJMError> {
    if js.get_stream(stream_name).await.is_ok() {
        println!("JetStream stream '{}' already exists.", stream_name);
        return Ok(());
    }

    println!("Creating JetStream stream '{}' for subject '{}'", stream_name, subject);

    let config = StreamConfig {
        name: stream_name.to_string(),
        subjects: vec![subject.to_string()],
        storage: jetstream::stream::StorageType::File,
        retention: jetstream::stream::RetentionPolicy::Limits,
        max_consumers: -1,
        max_messages: -1,
        max_bytes: -1,
        discard: jetstream::stream::DiscardPolicy::Old,
        ..Default::default()
    };

    js.create_stream(config)
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to create JetStream stream: {}", e)))?;

    Ok(())
}


async fn ensure_kv_bucket(js: &jetstream::Context, bucket: &str) -> Result<kv::Store, LJMError> {
    if let Ok(store) = js.get_key_value(bucket).await {
        println!("KV bucket '{}' already exists.", bucket);
        return Ok(store);
    }
    println!("Creating KV bucket '{}'", bucket);
    let cfg = kv::Config {
        bucket: bucket.to_string(),
        history: 5,
        ..Default::default()
    };
    js.create_key_value(cfg)
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to create KV bucket: {}", e)))
}

async fn load_config_from_kv(store: &kv::Store, key: &str) -> Result<SampleConfig, LJMError> {
    match store.entry(key).await {
        Ok(Some(entry)) => {
            serde_json::from_slice::<SampleConfig>(entry.value.as_ref())
                .map_err(|e| LJMError::LibraryError(format!("Config JSON parse error: {}", e)))
        }
        Ok(None) => Err(LJMError::LibraryError(format!("KV key '{}' not found", key))),
        Err(e) => Err(LJMError::LibraryError(format!("KV entry error for '{}': {}", key, e))),
    }
}


async fn watch_kv_config(
    store: kv::Store,
    key: String,
    config_tx: tokio::sync::watch::Sender<SampleConfig>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut watch = match store.watch(&key).await {
        Ok(w) => w,
        Err(e) => { eprintln!("[watch_kv_config] watch error: {}", e); return; }
    };
    println!("[watch_kv_config] Watching KV key '{}'", key);

    loop {
        tokio::select! {
            maybe = watch.next() => {
                match maybe {
                    Some(Ok(entry)) => {
                        if entry.operation == Operation::Put {
                            if let Ok(new_cfg) = serde_json::from_slice::<SampleConfig>(entry.value.as_ref()) {
                                if new_cfg != *config_tx.borrow() {
                                    println!("[watch_kv_config] KV config updated (rev {}): {:?}", entry.revision, new_cfg);
                                    let _ = config_tx.send(new_cfg);
                                }
                            } else {
                                eprintln!("[watch_kv_config] Invalid JSON in KV for key '{}'", entry.key);
                            }
                        } else {
                            eprintln!("[watch_kv_config] {:?} for key '{}', ignoring.", entry.operation, entry.key);
                        }
                    }
                    Some(Err(e)) => eprintln!("[watch_kv_config] stream err: {}", e),
                    None => { eprintln!("[watch_kv_config] watch ended"); break; }
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { println!("[watch_kv_config] shutdown"); break; }
            }
        }
    }
}


use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering}
};

async fn sample_with_config(
    run_id: usize,
    cfg: SampleConfig,
    config_rx: &mut watch::Receiver<SampleConfig>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<(), LJMError> {
    println!("[run #{run_id}] Connecting to NATS at {}", cfg.nats_url);
    let nc = async_nats::connect(cfg.nats_url.clone())
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to connect to NATS: {}", e)))?;
    let js = async_nats::jetstream::new(nc);

    ensure_stream_exists(&js, &cfg.nats_stream, &stream_subject_wildcard(&cfg.nats_subject, cfg.asset_number)).await?;

    let handle = LJMLibrary::open_jack(DeviceType::ANY, ConnectionType::ANY, "ANY")?;
    let _guard = LabJackGuard { handle };

    let info = LJMLibrary::get_handle_info(handle)?;
    println!(
        "[run #{run_id}] Connected to {:?} (serial {})",
        info.device_type, info.serial_number
    );

    if matches!(info.device_type, DeviceType::T7) {
        LJMLibrary::write_name(handle, "AIN_ALL_NEGATIVE_CH", 199_u32)?;
    }
    LJMLibrary::write_name(handle, "AIN_ALL_RANGE", 10.0_f64)?;
    LJMLibrary::write_name(handle, "AIN_ALL_RESOLUTION_INDEX", 0_u32)?;
    LJMLibrary::write_name(handle, "STREAM_SETTLING_US", 0_u32)?;

    let channel_addresses: Result<Vec<i32>, LJMError> = cfg.channels.iter()
        .map(|ch| {
            LJMLibrary::name_to_address(&format!("AIN{}", ch))
                .map(|(addr, _)| addr)
                .map_err(|e| LJMError::LibraryError(format!("Invalid channel {}: {:?}", ch, e)))
        })
        .collect();
    let channel_addresses = channel_addresses?;
    let num_channels = channel_addresses.len();

    let actual_rate = LJMLibrary::stream_start(
        handle,
        cfg.scans_per_read,
        cfg.suggested_scan_rate,
        channel_addresses.clone(),
    )?;
    println!(
        "[run #{run_id}] Streaming started: {} scans/read @ {} Hz",
        cfg.scans_per_read, actual_rate
    );

    let (scan_tx, mut scan_rx) = mpsc::channel::<Vec<f64>>(32);

    // Shared running flag for stopping the blocking loop
    let running = Arc::new(AtomicBool::new(true));
    let running_reader = running.clone();
    let scan_tx_reader = scan_tx.clone();

    // Single long-lived blocking task for reading
    let read_handle = tokio::task::spawn_blocking(move || {
        while running_reader.load(Ordering::Relaxed) {
            match LJMLibrary::stream_read(handle) {
                Ok(batch) => {
                    if scan_tx_reader.blocking_send(batch).is_err() {
                        break; // receiver gone
                    }
                }
                Err(e) => {
                    eprintln!("[run #{run_id}] Error reading stream: {:?}", e);
                    break;
                }
            }
        }
    });

    let mut builder = FlatBufferBuilder::new();

    loop {
        tokio::select! {
            Some(batch) = scan_rx.recv() => {
                let batch_timestamp = New_York.from_utc_datetime(&Utc::now().naive_utc()).to_rfc3339();

                let scans = batch.chunks(num_channels);
                let mut per_channel: Vec<Vec<f64>> = (0..num_channels)
                    .map(|_| Vec::with_capacity(scans.len()))
                    .collect();

                for scan in batch.chunks(num_channels) {
                    for (i, v) in scan.iter().enumerate() {
                        per_channel[i].push(*v);
                    }
                }

                for (i, values) in per_channel.into_iter().enumerate() {
                    builder.reset();
                    let ts_fb = builder.create_string(&batch_timestamp);
                    let values_fb = builder.create_vector(&values);
                    let scan_args = ScanArgs { timestamp: Some(ts_fb), values: Some(values_fb)};
                    let scan_offset = sampler::Scan::create(&mut builder, &scan_args);
                    builder.finish(scan_offset, None);

                    let data = builder.finished_data().to_vec();

                    let ch_num: u8 = cfg.channels[i];
                    let subject = channel_subject(&cfg.nats_subject, cfg.asset_number, ch_num);

                    if let Err(e) = js.publish(subject, data.into()).await {
                        eprintln!("[run #{run_id}] Failed to publish to NATS: {}", e);
                    }
                }
            }
            _ = config_rx.changed() => {
                println!("[run #{run_id}] Config change detected. Stopping stream...");
                running.store(false, Ordering::Relaxed);
                let _ = LJMLibrary::stream_stop(handle);
                drop(scan_tx);
                let _ = read_handle.await;
                return Ok(());
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    println!("[run #{run_id}] Shutdown signal received. Stopping stream...");
                    running.store(false, Ordering::Relaxed);
                    let _ = LJMLibrary::stream_stop(handle);
                    drop(scan_tx);
                    let _ = read_handle.await;
                    return Ok(());
                }
            }
        }
    }
}

async fn run_sampler(
    mut config_rx: tokio::sync::watch::Receiver<SampleConfig>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut run_id = 0;
    loop {
        if *shutdown_rx.borrow() {
            println!("[run_sampler] Sampler shutting down...");
            break;
        }
        run_id += 1;
        let cfg = config_rx.borrow().clone();
        println!("[run_sampler] Starting sampler run #{run_id} with {:?}", cfg);

        if let Err(e) = sample_with_config(run_id, cfg, &mut config_rx, &mut shutdown_rx).await {
            eprintln!("[run_sampler] Sampler error: {:?}", e);
        }

        if *shutdown_rx.borrow() {
            println!("[run_sampler] Shutdown detected after sampler error/config change");
            break;
        }

        println!("[run_sampler] Restarting sampler after config change...");
    }
}



#[tokio::main]
async fn main() -> Result<(), LJMError> {
    
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://0.0.0.0:4222".into());
    let nc = async_nats::connect(&nats_url).await
        .map_err(|e| LJMError::LibraryError(format!("NATS connect failed: {}", e)))?;
    let js = jetstream::new(nc);

    let bucket = std::env::var("CFG_BUCKET").unwrap_or_else(|_| "sampler_cfg".into());
    let key    = std::env::var("CFG_KEY").unwrap_or_else(|_| "active".into());

    let store = ensure_kv_bucket(&js, &bucket).await?;
    let cfg = load_config_from_kv(&store, &key).await?;
    println!("[bootstrap] Loaded initial config from KV '{}:{}': {:?}", bucket, key, cfg);

    let (config_tx, config_rx) = watch::channel(cfg);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    #[cfg(feature = "staticlib")]
    unsafe { LJMLibrary::init()?; }
    #[cfg(all(feature = "dynlink", not(feature = "staticlib")))]
    unsafe {
        let path = std::env::var("LJM_PATH").ok();
        LJMLibrary::init(path)?;
    }

    tokio::spawn(run_sampler(config_rx.clone(), shutdown_rx.clone()));
    tokio::spawn(watch_kv_config(store, key.clone(), config_tx.clone(), shutdown_rx.clone()));

    tokio::signal::ctrl_c()
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to listen for Ctrl+C: {}", e)))?;

    println!("Shutting down...");
    let _ = shutdown_tx.send(true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}
