use chrono::{TimeZone, Utc};
use chrono_tz::America::New_York;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{mpsc, watch};
use tokio::time::Duration;

use ljmrs::handle::{ConnectionType, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

use async_nats::jetstream::kv::Operation;
use async_nats::jetstream::{self, kv, stream::Config as StreamConfig};
use async_nats::{ConnectOptions, ServerAddr};
use flatbuffers::FlatBufferBuilder;
use futures_util::StreamExt;

mod calibration;
mod sample_data_generated {
    #![allow(dead_code, unused_imports)]
    include!("data_generated.rs");
}
use calibration::CalibrationSpec;
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
    calibrations: Option<HashMap<String, CalibrationSpec>>,
    trigger_settings: Option<HashMap<String, TriggerSettings>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum TriggerType {
    Rising,
    Falling,
}

fn default_trigger_enabled() -> bool {
    true
}

fn default_trigger_type() -> TriggerType {
    TriggerType::Rising
}

fn default_trigger_holdoff_ms() -> u64 {
    500
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TriggerSettings {
    #[serde(default = "default_trigger_enabled")]
    enabled: bool,
    #[serde(default = "default_trigger_type", rename = "type")]
    trigger_type: TriggerType,
    #[serde(default)]
    threshold: f64,
    #[serde(default = "default_trigger_holdoff_ms", alias = "holdoffMs")]
    holdoff_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TriggerEvent {
    asset: u32,
    channel: u8,
    trigger_time: String,
    trigger_time_unix_ms: i64,
    raw_value: f64,
    calibrated_value: f64,
    threshold: f64,
    trigger_type: TriggerType,
    holdoff_ms: u64,
    calibration_id: String,
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
    trigger_stream: String,
    calibrations: HashMap<u8, CalibrationSpec>,
    trigger_settings: HashMap<u8, TriggerSettings>,
}

#[derive(Debug, Clone, Default)]
struct ChannelTriggerState {
    last_calibrated: Option<f64>,
    next_eligible_at_ms: i64,
}

fn parse_calibrations(raw: &SensorSettings) -> HashMap<u8, CalibrationSpec> {
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
                eprintln!("[streamer] Invalid calibration channel key '{key}', expected u8.");
            }
        }
    }

    out
}

fn parse_trigger_settings(raw: &SensorSettings) -> HashMap<u8, TriggerSettings> {
    let mut out = HashMap::new();
    let Some(trigger_settings) = raw.trigger_settings.as_ref() else {
        return out;
    };

    for (key, spec) in trigger_settings {
        match key.parse::<u8>() {
            Ok(ch) => {
                out.insert(ch, spec.clone());
            }
            Err(_) => {
                eprintln!("[streamer] Invalid trigger channel key '{key}', expected u8.");
            }
        }
    }

    out
}

fn default_trigger_stream() -> String {
    std::env::var("TRIGGER_STREAM").unwrap_or_else(|_| "labjack_triggers".to_string())
}

fn sample_config_from_nested(nested: NestedConfig, base: Option<&SampleConfig>) -> SampleConfig {
    let raw = nested.sensor_settings;
    let calibrations = parse_calibrations(&raw);
    let trigger_settings = parse_trigger_settings(&raw);

    SampleConfig {
        scans_per_read: raw.scan_rate,
        suggested_scan_rate: raw.sampling_rate,
        channels: raw.channels_enabled,
        asset_number: nested.asset_number,
        nats_subject: nested.nats_subject,
        nats_stream: nested.nats_stream,
        rotate_secs: nested.rotate_secs,
        trigger_stream: base
            .map(|cfg| cfg.trigger_stream.clone())
            .unwrap_or_else(default_trigger_stream),
        calibrations,
        trigger_settings,
    }
}

fn parse_nested_config(
    bytes: &[u8],
    base: Option<&SampleConfig>,
) -> Result<SampleConfig, LJMError> {
    let nested_cfg: NestedConfig = serde_json::from_slice(bytes)
        .map_err(|e| LJMError::LibraryError(format!("Config JSON parse error: {}", e)))?;
    Ok(sample_config_from_nested(nested_cfg, base))
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
    let lj_ip = std::env::var("LABJACK_IP").ok();
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
                let Some(lj_ip) = lj_ip.as_deref() else {
                    errors.push("ethernet: LABJACK_IP not set".to_string());
                    continue;
                };
                println!("[labjack] attempting ethernet open via {}", lj_ip);
                match LJMLibrary::open_jack(device_type, ConnectionType::ETHERNET, lj_ip) {
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

fn trigger_subject(prefix: &str, asset: u32, ch: u8) -> String {
    format!(
        "{}.{}.trigger.{}",
        prefix,
        pad_asset(asset),
        pad_channel(ch)
    )
}

fn trigger_stream_subject_wildcard(prefix: &str) -> String {
    format!("{}.*.trigger.*", prefix)
}

fn parse_nats_servers_from_env() -> Result<Vec<ServerAddr>, LJMError> {
    let raw = std::env::var("NATS_SERVERS")
        .map_err(|_| LJMError::LibraryError("NATS_SERVERS must be set".to_string()))?;

    let servers = raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<ServerAddr>().map_err(|e| {
                LJMError::LibraryError(format!("invalid server addr '{}': {}", part, e))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if servers.is_empty() {
        return Err(LJMError::LibraryError(
            "NATS_SERVERS resolved to an empty list".to_string(),
        ));
    }

    Ok(servers)
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
        Ok(Some(entry)) => parse_nested_config(entry.value.as_ref(), None),
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
                            let current_cfg = config_tx.borrow().clone();
                            let parsed = parse_nested_config(entry.value.as_ref(), Some(&current_cfg));

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
                                    eprintln!(
                                        "[watch_kv_config] Failed to parse JSON for key '{}': {:?}",
                                        entry.key, e
                                    );
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
    ensure_stream_exists(
        &js,
        &cfg.trigger_stream,
        &trigger_stream_subject_wildcard(&cfg.nats_subject),
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
    let mut trigger_states: HashMap<u8, ChannelTriggerState> = cfg
        .channels
        .iter()
        .map(|ch| (*ch, ChannelTriggerState::default()))
        .collect();

    loop {
        tokio::select! {
            Some(batch) = scan_rx.recv() => {
                let batch_end_utc = Utc::now();
                let batch_timestamp = New_York
                    .from_utc_datetime(&batch_end_utc.naive_utc())
                    .to_rfc3339();
                let sample_period_ms = if cfg.suggested_scan_rate > 0.0 {
                    1000.0 / cfg.suggested_scan_rate
                } else {
                    0.0
                };

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

                    if let Some(trigger_state) = trigger_states.get_mut(&ch_num) {
                        let calibration = cfg.calibrations.get(&ch_num).cloned().unwrap_or_default();
                        let trigger_settings = cfg.trigger_settings.get(&ch_num);
                        let mut emitted = 0_u32;

                        for (sample_idx, raw_value) in values.iter().enumerate() {
                            let calibrated = calibration.apply(*raw_value);
                            let sample_offset_ms = if sample_period_ms > 0.0 {
                                let remaining = values.len().saturating_sub(sample_idx + 1);
                                (remaining as f64 * sample_period_ms).round() as i64
                            } else {
                                0
                            };
                            let sample_time = batch_end_utc - chrono::Duration::milliseconds(sample_offset_ms);
                            let sample_time_unix_ms = sample_time.timestamp_millis();

                            if let (Some(prev), Some(trigger)) =
                                (trigger_state.last_calibrated, trigger_settings)
                            {
                                if trigger.enabled && sample_time_unix_ms >= trigger_state.next_eligible_at_ms {
                                    let crossed = match trigger.trigger_type {
                                        TriggerType::Rising => {
                                            prev <= trigger.threshold && calibrated > trigger.threshold
                                        }
                                        TriggerType::Falling => {
                                            prev >= trigger.threshold && calibrated < trigger.threshold
                                        }
                                    };

                                    if crossed {
                                        let event = TriggerEvent {
                                            asset: cfg.asset_number,
                                            channel: ch_num,
                                            trigger_time: sample_time.to_rfc3339(),
                                            trigger_time_unix_ms: sample_time_unix_ms,
                                            raw_value: *raw_value,
                                            calibrated_value: calibrated,
                                            threshold: trigger.threshold,
                                            trigger_type: trigger.trigger_type.clone(),
                                            holdoff_ms: trigger.holdoff_ms,
                                            calibration_id: calibration.id_or_default().to_string(),
                                        };
                                        let trigger_payload = match serde_json::to_vec(&event) {
                                            Ok(payload) => payload,
                                            Err(err) => {
                                                eprintln!("[run #{run_id}] Failed to encode trigger event: {}", err);
                                                trigger_state.last_calibrated = Some(calibrated);
                                                continue;
                                            }
                                        };

                                        let trigger_subject = trigger_subject(
                                            &cfg.nats_subject,
                                            cfg.asset_number,
                                            ch_num,
                                        );
                                        if let Err(err) = js
                                            .publish(trigger_subject, trigger_payload.into())
                                            .await
                                        {
                                            eprintln!(
                                                "[run #{run_id}] Failed to publish trigger event for channel {}: {}",
                                                ch_num, err
                                            );
                                        } else {
                                            emitted += 1;
                                        }

                                        trigger_state.next_eligible_at_ms = sample_time_unix_ms
                                            + trigger.holdoff_ms as i64;
                                    }
                                }
                            }

                            trigger_state.last_calibrated = Some(calibrated);
                        }

                        if emitted > 0 {
                            println!(
                                "[run #{run_id}] emitted {} trigger event(s) for ch{}",
                                emitted, ch_num
                            );
                        }
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

    let servers = parse_nats_servers_from_env()?;

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
