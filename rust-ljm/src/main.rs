use chrono::{TimeZone, Utc};
use chrono_tz::America::New_York;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tokio::time::Duration;

use ljmrs::handle::{ConnectionType, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

use async_nats::jetstream::kv::Operation;
use async_nats::jetstream::{self, kv, stream::Config as StreamConfig};
use async_nats::{ConnectOptions, ServerAddr};
use flatbuffers::FlatBufferBuilder;
use futures_util::StreamExt;

mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs");
}
use sample_data_generated::sampler::{self, ScanArgs};

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct NestedConfig {
    labjack_name: String,
    asset_number: u32,
    max_channels: u32,
    nats_subject: String,
    nats_stream: String,
    rotate_secs: u64,
    sensor_settings: SensorSettings,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct SensorSettings {
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

impl From<(NestedConfig, &SampleConfig)> for SampleConfig {
    fn from((nested, base): (NestedConfig, &SampleConfig)) -> Self {
        let raw = nested.sensor_settings;
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

struct LabJackGuard {
    handle: i32,
}

impl Drop for LabJackGuard {
    fn drop(&mut self) {
        let _ = LJMLibrary::stream_stop(self.handle);
        let _ = LJMLibrary::close_jack(self.handle);
    }
}

fn open_labjack_with_fallback(device_type: DeviceType) -> Result<i32, LJMError> {
    let lj_ip = std::env::var("LABJACK_IP").unwrap_or_else(|_| "10.165.77.233".to_string());
    let usb_id = std::env::var("LABJACK_USB_ID").unwrap_or_else(|_| "ANY".to_string());
    let order = std::env::var("LABJACK_OPEN_ORDER").unwrap_or_else(|_| "ethernet,usb".to_string());

    let mut modes: Vec<String> = order
        .split(',')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect();

    if modes.is_empty() {
        modes = vec!["ethernet".to_string(), "usb".to_string()];
    }

    let mut errors: Vec<String> = Vec::new();

    for mode in modes {
        match mode.as_str() {
            "ethernet" | "tcp" => {
                println!("[labjack] attempting ethernet open via {}", lj_ip);
                match LJMLibrary::open_jack(device_type, ConnectionType::ETHERNET, lj_ip.as_str()) {
                    Ok(handle) => return Ok(handle),
                    Err(e) => errors.push(format!("ethernet({}): {:?}", lj_ip, e)),
                }
            }
            "usb" => {
                println!("[labjack] attempting usb open via {}", usb_id);
                match LJMLibrary::open_jack(device_type, ConnectionType::USB, usb_id.as_str()) {
                    Ok(handle) => return Ok(handle),
                    Err(e) => errors.push(format!("usb({}): {:?}", usb_id, e)),
                }
            }
            "any" => {
                println!("[labjack] attempting any open");
                match LJMLibrary::open_jack(device_type, ConnectionType::ANY, "ANY") {
                    Ok(handle) => return Ok(handle),
                    Err(e) => errors.push(format!("any: {:?}", e)),
                }
            }
            other => {
                errors.push(format!("unsupported mode '{}'", other));
            }
        }
    }

    Err(LJMError::LibraryError(format!(
        "Could not open LabJack with order '{}': {}",
        order,
        errors.join(" | ")
    )))
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

    println!(
        "Creating JetStream stream '{}' for subject '{}'",
        stream_name, subject
    );

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
            let nested_cfg: NestedConfig = serde_json::from_slice(entry.value.as_ref())
                .map_err(|e| LJMError::LibraryError(format!("Config JSON parse error: {}", e)))?;

            let cfg = SampleConfig {
                scans_per_read: nested_cfg.sensor_settings.scan_rate,
                suggested_scan_rate: nested_cfg.sensor_settings.sampling_rate,
                channels: nested_cfg.sensor_settings.channels_enabled.clone(),
                asset_number: nested_cfg.asset_number,
                nats_subject: nested_cfg.nats_subject.clone(),
                nats_stream: nested_cfg.nats_stream.clone(),
                rotate_secs: nested_cfg.rotate_secs,
            };

            Ok(cfg)
        }
        Ok(None) => Err(LJMError::LibraryError(format!(
            "KV key '{}' not found",
            key
        ))),
        Err(e) => Err(LJMError::LibraryError(format!(
            "KV entry error for '{}': {}",
            key, e
        ))),
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
        Err(e) => {
            eprintln!("[watch_kv_config] watch error: {}", e);
            return;
        }
    };
    println!("[watch_kv_config] Watching KV key '{}'", key);

    loop {
        tokio::select! {
            maybe = watch.next() => {
                match maybe {
                    Some(Ok(entry)) => {
                        if entry.operation == Operation::Put {
                            // try nested format first
                            let parsed = serde_json::from_slice::<NestedConfig>(entry.value.as_ref())
                                .map(|nested| {
                                    let raw = nested.sensor_settings;
                                    let base = config_tx.borrow().clone();

                                    SampleConfig {
                                        scans_per_read: raw.scan_rate,
                                        suggested_scan_rate: raw.sampling_rate,
                                        channels: raw.channels_enabled,
                                        asset_number: base.asset_number,
                                        nats_subject: base.nats_subject,
                                        nats_stream: base.nats_stream,
                                        rotate_secs: base.rotate_secs,
                                    }

                                })
                                .or_else(|_| {
                                    serde_json::from_slice::<NestedConfig>(entry.value.as_ref())
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
                                });

                            match parsed {
                                Ok(new_cfg) => {
                                    if new_cfg != *config_tx.borrow() {
                                        println!(
                                            "[watch_kv_config] KV config updated (rev {}): {:?}",
                                            entry.revision, new_cfg
                                        );
                                        let _ = config_tx.send(new_cfg);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[watch_kv_config] Failed to parse JSON for key '{}': {}", entry.key, e);
                                }
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
    atomic::{AtomicBool, Ordering},
};

async fn sample_with_config(
    run_id: usize,
    cfg: SampleConfig,
    config_rx: &mut watch::Receiver<SampleConfig>,
    shutdown_rx: &mut watch::Receiver<bool>,
    js: &jetstream::Context,
) -> Result<(), LJMError> {
    ensure_stream_exists(
        &js,
        &cfg.nats_stream,
        &stream_subject_wildcard(&cfg.nats_subject, cfg.asset_number),
    )
    .await?;

    let handle = open_labjack_with_fallback(DeviceType::T7)?;
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

    let channel_addresses: Result<Vec<i32>, LJMError> = cfg
        .channels
        .iter()
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
    js: jetstream::Context,
) {
    let mut run_id = 0;
    loop {
        if *shutdown_rx.borrow() {
            println!("[run_sampler] Sampler shutting down...");
            break;
        }
        run_id += 1;
        let cfg = config_rx.borrow().clone();
        println!(
            "[run_sampler] Starting sampler run #{run_id} with {:?}",
            cfg
        );

        if let Err(e) = sample_with_config(run_id, cfg, &mut config_rx, &mut shutdown_rx, &js).await
        {
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
    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to load creds: {}", e)))?;

    let servers: Vec<ServerAddr> = vec![
        "nats://nats1.oats:4222"
            .parse()
            .map_err(|e| LJMError::LibraryError(format!("invalid server addr: {}", e)))?,
        "nats://nats2.oats:4222"
            .parse()
            .map_err(|e| LJMError::LibraryError(format!("invalid server addr: {}", e)))?,
        "nats://nats3.oats:4222"
            .parse()
            .map_err(|e| LJMError::LibraryError(format!("invalid server addr: {}", e)))?,
    ];

    let nc = opts
        .connect(servers)
        .await
        .map_err(|e| LJMError::LibraryError(format!("NATS connect failed: {}", e)))?;

    println!("Connected to NATS via creds!");
    let js = jetstream::new(nc);

    let bucket = std::env::var("CFG_BUCKET").unwrap_or_else(|_| "avenabox".into());
    let key = std::env::var("CFG_KEY").unwrap_or_else(|_| "labjackd.config.macbook".into());

    let store = ensure_kv_bucket(&js, &bucket).await?;
    let cfg = load_config_from_kv(&store, &key).await?;
    println!(
        "[bootstrap] Loaded initial config from KV '{}:{}': {:?}",
        bucket, key, cfg
    );

    let (config_tx, config_rx) = watch::channel(cfg);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    #[cfg(feature = "staticlib")]
    unsafe {
        LJMLibrary::init()?;
    }
    #[cfg(all(feature = "dynlink", not(feature = "staticlib")))]
    unsafe {
        let path = std::env::var("LJM_PATH").ok();
        LJMLibrary::init(path)?;
    }

    tokio::spawn(run_sampler(
        config_rx.clone(),
        shutdown_rx.clone(),
        js.clone(),
    ));
    tokio::spawn(watch_kv_config(
        store,
        key.clone(),
        config_tx.clone(),
        shutdown_rx.clone(),
    ));

    tokio::signal::ctrl_c()
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to listen for Ctrl+C: {}", e)))?;

    println!("Shutting down...");
    let _ = shutdown_tx.send(true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}
