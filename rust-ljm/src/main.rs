use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tokio::time::Duration;

use ljmrs::handle::DeviceType;
use ljmrs::{LJMError, LJMLibrary};

use async_nats::ConnectOptions;
use async_nats::jetstream::kv::Operation;
use async_nats::jetstream::{self, kv, stream::Config as StreamConfig};
use flatbuffers::FlatBufferBuilder;
use futures_util::StreamExt;

mod labjack;
mod ljm_mode;
mod nats_config;
mod subjects;
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
    sensor_settings: SensorSettings,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct SensorSettings {
    #[serde(rename = "scans_per_read", alias = "scan_rate")]
    scans_per_read: i32,
    #[serde(rename = "scan_rate_hz", alias = "sampling_rate")]
    scan_rate_hz: f64,
    channels_enabled: Vec<u8>,
    gains: i32,
    data_formats: Vec<String>,
    measurement_units: Vec<String>,
    labjack_on_off: bool,
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
}

fn sample_config_from_nested(nested: NestedConfig) -> SampleConfig {
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
    }
}

fn sample_config_from_json(bytes: &[u8]) -> Result<SampleConfig, LJMError> {
    let nested_cfg: NestedConfig = serde_json::from_slice(bytes)
        .map_err(|e| LJMError::LibraryError(format!("Config JSON parse error: {}", e)))?;
    Ok(sample_config_from_nested(nested_cfg))
}

#[derive(Debug, Clone)]
struct CentralKvSyncConfig {
    servers: Vec<async_nats::ServerAddr>,
    creds_path: String,
    bucket: String,
    key: String,
    domain: Option<String>,
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

fn stream_max_bytes_from_env() -> Result<i64, LJMError> {
    let Some(raw) = env_nonempty("STREAM_MAX_BYTES") else {
        return Ok(-1);
    };

    let parsed = raw.parse::<i64>().map_err(|e| {
        LJMError::LibraryError(format!(
            "Invalid STREAM_MAX_BYTES value '{}': {}",
            raw, e
        ))
    })?;

    if parsed <= 0 {
        return Err(LJMError::LibraryError(format!(
            "Invalid STREAM_MAX_BYTES value '{}': must be greater than zero",
            raw
        )));
    }

    Ok(parsed)
}

async fn ensure_stream_exists(
    js: &jetstream::Context,
    stream_name: &str,
    subject: &str,
) -> Result<(), LJMError> {
    let max_bytes = stream_max_bytes_from_env()?;
    let desired_subjects = vec![subject.to_string()];

    if let Ok(stream) = js.get_stream(stream_name).await {
        let info = stream.cached_info();
        let already_configured = info.config.subjects == desired_subjects
            && info.config.storage == jetstream::stream::StorageType::File
            && info.config.retention == jetstream::stream::RetentionPolicy::Limits
            && info.config.max_bytes == max_bytes
            && info.config.discard == jetstream::stream::DiscardPolicy::Old;

        if already_configured {
            println!(
                "JetStream stream '{}' already matches subject(s) {:?}, storage {:?}, max_bytes {}, discard {:?}.",
                stream_name,
                info.config.subjects,
                info.config.storage,
                info.config.max_bytes,
                info.config.discard
            );
            return Ok(());
        }

        println!(
            "Reconciling JetStream stream '{}': subjects {:?} -> {:?}, max_bytes {} -> {}.",
            stream_name,
            info.config.subjects,
            desired_subjects,
            info.config.max_bytes,
            max_bytes
        );
    }

    println!(
        "Ensuring JetStream stream '{}' is configured for subject '{}'",
        stream_name, subject
    );

    let config = StreamConfig {
        name: stream_name.to_string(),
        subjects: desired_subjects,
        storage: jetstream::stream::StorageType::File,
        retention: jetstream::stream::RetentionPolicy::Limits,
        max_consumers: -1,
        max_messages: -1,
        max_bytes,
        discard: jetstream::stream::DiscardPolicy::Old,
        ..Default::default()
    };

    js.create_or_update_stream(config).await.map_err(|e| {
        LJMError::LibraryError(format!(
            "Failed to create or update JetStream stream '{}': {}",
            stream_name, e
        ))
    })?;

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
        Ok(Some(entry)) => sample_config_from_json(entry.value.as_ref()),
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

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn local_creds_path_from_env() -> String {
    std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into())
}

fn central_kv_sync_config_from_env() -> Result<Option<CentralKvSyncConfig>, LJMError> {
    let Some(raw_servers) = env_nonempty("CENTRAL_NATS_SERVERS") else {
        return Ok(None);
    };

    let servers =
        nats_config::servers_from_env_var("CENTRAL_NATS_SERVERS", Some(raw_servers.as_str()))
            .map_err(LJMError::LibraryError)?;
    let creds_path = env_nonempty("CENTRAL_NATS_CREDS_FILE").unwrap_or_else(local_creds_path_from_env);
    let bucket = env_nonempty("CENTRAL_CFG_BUCKET")
        .or_else(|| env_nonempty("CFG_BUCKET"))
        .unwrap_or_else(|| "avenabox".to_string());
    let key = env_nonempty("CENTRAL_CFG_KEY")
        .or_else(|| env_nonempty("CFG_KEY"))
        .unwrap_or_else(|| "v1.macbook.unknown-source.config".to_string());
    let domain = env_nonempty("CENTRAL_JS_DOMAIN");

    Ok(Some(CentralKvSyncConfig {
        servers,
        creds_path,
        bucket,
        key,
        domain,
    }))
}

async fn connect_nats_with_creds(
    servers: Vec<async_nats::ServerAddr>,
    creds_path: String,
) -> Result<async_nats::Client, LJMError> {
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to load creds: {}", e)))?;

    opts.connect(servers)
        .await
        .map_err(|e| LJMError::LibraryError(format!("NATS connect failed: {}", e)))
}

async fn mirror_remote_kv_entry_to_local(
    remote_store: &kv::Store,
    remote_bucket: &str,
    remote_key: &str,
    local_store: &kv::Store,
    local_key: &str,
) -> Result<bool, LJMError> {
    let remote_entry = match remote_store.entry(remote_key).await {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            return Err(LJMError::LibraryError(format!(
                "Remote KV key '{}' not found",
                remote_key
            )));
        }
        Err(e) => {
            return Err(LJMError::LibraryError(format!(
                "Remote KV entry error for '{}': {}",
                remote_key, e
            )));
        }
    };

    sample_config_from_json(remote_entry.value.as_ref())?;

    let local_matches = match local_store.entry(local_key).await {
        Ok(Some(local_entry)) => local_entry.value.as_ref() == remote_entry.value.as_ref(),
        Ok(None) => false,
        Err(e) => {
            return Err(LJMError::LibraryError(format!(
                "Local KV entry error for '{}': {}",
                local_key, e
            )));
        }
    };

    if local_matches {
        return Ok(false);
    }

    local_store
        .put(local_key, remote_entry.value.clone())
        .await
        .map_err(|e| {
            LJMError::LibraryError(format!(
                "Failed to mirror remote KV '{}' into local key '{}': {}",
                remote_key, local_key, e
            ))
        })?;

    println!(
        "[central_kv_sync] mirrored '{}:{}' into local '{}'",
        remote_bucket,
        remote_key,
        local_key
    );
    Ok(true)
}

async fn run_central_kv_sync(
    sync_cfg: CentralKvSyncConfig,
    local_store: kv::Store,
    local_key: String,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        if *shutdown_rx.borrow() {
            println!("[central_kv_sync] shutdown");
            break;
        }

        let connect_result =
            connect_nats_with_creds(sync_cfg.servers.clone(), sync_cfg.creds_path.clone()).await;
        let client = match connect_result {
            Ok(client) => client,
            Err(err) => {
                eprintln!("[central_kv_sync] connect failed: {:?}", err);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            println!("[central_kv_sync] shutdown");
                            break;
                        }
                    }
                }
                continue;
            }
        };

        let remote_js =
            nats_config::jetstream_context_for_domain(client, sync_cfg.domain.as_deref());
        let remote_store = match ensure_kv_bucket(&remote_js, &sync_cfg.bucket).await {
            Ok(store) => store,
            Err(err) => {
                eprintln!("[central_kv_sync] remote bucket setup failed: {:?}", err);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            println!("[central_kv_sync] shutdown");
                            break;
                        }
                    }
                }
                continue;
            }
        };

        if let Err(err) =
            mirror_remote_kv_entry_to_local(
                &remote_store,
                &sync_cfg.bucket,
                &sync_cfg.key,
                &local_store,
                &local_key,
            )
                .await
        {
            eprintln!("[central_kv_sync] initial mirror failed: {:?}", err);
        }

        let mut watch = match remote_store.watch(&sync_cfg.key).await {
            Ok(watch) => watch,
            Err(err) => {
                eprintln!("[central_kv_sync] watch setup failed: {}", err);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            println!("[central_kv_sync] shutdown");
                            break;
                        }
                    }
                }
                continue;
            }
        };

        println!(
            "[central_kv_sync] watching remote '{}:{}' for local key '{}'",
            sync_cfg.bucket, sync_cfg.key, local_key
        );

        loop {
            tokio::select! {
                maybe = watch.next() => {
                    match maybe {
                        Some(Ok(entry)) => {
                            if entry.operation == Operation::Put {
                                if let Err(err) = sample_config_from_json(entry.value.as_ref()) {
                                    eprintln!(
                                        "[central_kv_sync] ignoring invalid remote config for key '{}': {:?}",
                                        entry.key, err
                                    );
                                    continue;
                                }

                                match local_store.entry(local_key.as_str()).await {
                                    Ok(Some(local_entry)) if local_entry.value.as_ref() == entry.value.as_ref() => {}
                                    Ok(_) => {
                                        if let Err(err) = local_store.put(local_key.as_str(), entry.value.clone()).await {
                                            eprintln!(
                                                "[central_kv_sync] failed to mirror remote update for key '{}': {}",
                                                entry.key, err
                                            );
                                        } else {
                                            println!(
                                                "[central_kv_sync] mirrored remote update rev {} for '{}'",
                                                entry.revision, entry.key
                                            );
                                        }
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "[central_kv_sync] failed to inspect local key '{}': {}",
                                            local_key, err
                                        );
                                    }
                                }
                            } else {
                                eprintln!(
                                    "[central_kv_sync] {:?} for remote key '{}', ignoring.",
                                    entry.operation, entry.key
                                );
                            }
                        }
                        Some(Err(err)) => {
                            eprintln!("[central_kv_sync] watch stream error: {}", err);
                            break;
                        }
                        None => {
                            eprintln!("[central_kv_sync] watch ended");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        println!("[central_kv_sync] shutdown");
                        return;
                    }
                }
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    println!("[central_kv_sync] shutdown");
                    break;
                }
            }
        }
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
                            match sample_config_from_json(entry.value.as_ref()) {
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
                                    eprintln!("[watch_kv_config] Failed to parse JSON for key '{}': {:?}", entry.key, e);
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

#[derive(Debug, Clone, Copy)]
struct StreamClock {
    sample_interval_ns: u64,
    next_first_sample_unix_ns: u64,
    sequence: u64,
    last_batch_samples: usize,
    run_started: bool,
}

impl StreamClock {
    fn new(sample_interval_ns: u64) -> Self {
        Self {
            sample_interval_ns,
            next_first_sample_unix_ns: 0,
            sequence: 0,
            last_batch_samples: 0,
            run_started: false,
        }
    }

    fn next_batch(&mut self, batch_samples: usize) -> Result<(u64, u64), LJMError> {
        if batch_samples == 0 {
            return Err(LJMError::LibraryError(
                "Received empty batch; refusing to guess timestamps".to_string(),
            ));
        }

        let first_sample_unix_ns = if !self.run_started {
            let now_ns = unix_time_now_ns()?;
            let offset_ns = (batch_samples.saturating_sub(1) as u128)
                .saturating_mul(self.sample_interval_ns as u128);
            let first = (now_ns as u128).saturating_sub(offset_ns);
            u64::try_from(first).map_err(|_| {
                LJMError::LibraryError("Initial stream timestamp overflowed u64".to_string())
            })?
        } else {
            self.next_first_sample_unix_ns
        };

        let batch_span_ns = (batch_samples as u128).saturating_mul(self.sample_interval_ns as u128);
        let next = (first_sample_unix_ns as u128).saturating_add(batch_span_ns);
        self.next_first_sample_unix_ns = u64::try_from(next).map_err(|_| {
            LJMError::LibraryError("Next stream timestamp overflowed u64".to_string())
        })?;

        let sequence = self.sequence;
        self.sequence = self.sequence.saturating_add(1);
        self.last_batch_samples = batch_samples;
        self.run_started = true;

        Ok((first_sample_unix_ns, sequence))
    }
}

fn unix_time_now_ns() -> Result<u64, LJMError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| LJMError::LibraryError(format!("system clock before Unix epoch: {e}")))?;
    u64::try_from(duration.as_nanos())
        .map_err(|_| LJMError::LibraryError("system time nanoseconds overflowed u64".to_string()))
}

fn derive_sample_interval_ns(actual_rate: f64) -> Result<u64, LJMError> {
    if !actual_rate.is_finite() || actual_rate <= 0.0 {
        return Err(LJMError::LibraryError(format!(
            "Invalid actual scan rate returned by stream_start: {actual_rate}"
        )));
    }

    let interval = (1_000_000_000.0 / actual_rate).round();
    if !interval.is_finite() || interval <= 0.0 {
        return Err(LJMError::LibraryError(format!(
            "Failed to derive sample interval from actual scan rate: {actual_rate}"
        )));
    }

    Ok(interval as u64)
}

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
        &subjects::live_labjack_stream_subject(
            &cfg.nats_subject,
            cfg.site_id.as_deref(),
            cfg.box_id.as_deref(),
            Some(&cfg.labjack_name),
            cfg.source_type.as_deref(),
            cfg.source_id.as_deref(),
        ),
    )
    .await?;

    let handle = labjack::open_labjack_from_env()?;
    let info = labjack::handle_info(handle)?;
    let ip = labjack::handle_ip_address(&info)?.unwrap_or_else(|| "N/A".to_string());
    println!(
        "[labjack] connected via {:?}, serial {}, ip {}",
        info.connection_type, info.serial_number, ip
    );
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
        cfg.scan_rate_hz,
        channel_addresses.clone(),
    )?;
    println!(
        "[run #{run_id}] Streaming started: {} scans/read @ {} Hz",
        cfg.scans_per_read, actual_rate
    );
    let sample_interval_ns = derive_sample_interval_ns(actual_rate)?;
    println!(
        "[run #{run_id}] Derived sample interval: {} ns from actual scan rate {} Hz",
        sample_interval_ns, actual_rate
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
    drop(scan_tx);

    let mut builder = FlatBufferBuilder::new();
    let mut clock = StreamClock::new(sample_interval_ns);

    loop {
        tokio::select! {
            maybe_batch = scan_rx.recv() => {
                let Some(batch) = maybe_batch else {
                    eprintln!(
                        "[run #{run_id}] Stream reader ended unexpectedly at sequence {}. Next expected first sample ns {}",
                        clock.sequence,
                        clock.next_first_sample_unix_ns
                    );
                    running.store(false, Ordering::Relaxed);
                    let _ = LJMLibrary::stream_stop(handle);
                    let _ = read_handle.await;
                    return Err(LJMError::LibraryError(
                        "Stream reader terminated unexpectedly".to_string(),
                    ));
                };
                if batch.is_empty() {
                    eprintln!("[run #{run_id}] Received empty batch; stopping run.");
                    running.store(false, Ordering::Relaxed);
                    let _ = LJMLibrary::stream_stop(handle);
                    let _ = read_handle.await;
                    return Err(LJMError::LibraryError(
                        "Received empty batch from stream_read".to_string(),
                    ));
                }
                if batch.len() % num_channels != 0 {
                    eprintln!(
                        "[run #{run_id}] Batch length {} is not divisible by channel count {}; stopping run as discontinuity.",
                        batch.len(),
                        num_channels
                    );
                    running.store(false, Ordering::Relaxed);
                    let _ = LJMLibrary::stream_stop(handle);
                    let _ = read_handle.await;
                    return Err(LJMError::LibraryError(format!(
                        "Malformed stream batch: {} values for {} channels",
                        batch.len(),
                        num_channels
                    )));
                }

                let batch_samples = batch.len() / num_channels;
                let (first_sample_unix_ns, sequence) = clock.next_batch(batch_samples)?;
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
                    let values_fb = builder.create_vector(&values);
                    let scan_args = ScanArgs {
                        first_sample_unix_ns,
                        sample_interval_ns,
                        actual_scan_rate_hz: actual_rate,
                        sequence,
                        values: Some(values_fb),
                    };
                    let scan_offset = sampler::Scan::create(&mut builder, &scan_args);
                    builder.finish(scan_offset, None);

                    let data = builder.finished_data().to_vec();

                    let ch_num: u8 = cfg.channels[i];
                    let subject = subjects::live_labjack_channel_subject(
                        &cfg.nats_subject,
                        cfg.asset_number,
                        ch_num,
                        cfg.site_id.as_deref(),
                        cfg.box_id.as_deref(),
                        Some(&cfg.labjack_name),
                        cfg.source_type.as_deref(),
                        cfg.source_id.as_deref(),
                    );

                    if let Err(e) = js.publish(subject, data.into()).await {
                        eprintln!("[run #{run_id}] Failed to publish to NATS: {}", e);
                    }
                }
            }
            _ = config_rx.changed() => {
                println!(
                    "[run #{run_id}] Config change detected. Stopping stream at sequence {}. Next expected first sample ns {}",
                    clock.sequence,
                    clock.next_first_sample_unix_ns
                );
                running.store(false, Ordering::Relaxed);
                let _ = LJMLibrary::stream_stop(handle);
                let _ = read_handle.await;
                return Ok(());
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    println!(
                        "[run #{run_id}] Shutdown signal received. Stopping stream at sequence {}. Next expected first sample ns {}",
                        clock.sequence,
                        clock.next_first_sample_unix_ns
                    );
                    running.store(false, Ordering::Relaxed);
                    let _ = LJMLibrary::stream_stop(handle);
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
    let servers = nats_config::servers_from_env().map_err(LJMError::LibraryError)?;
    let nc = connect_nats_with_creds(servers, local_creds_path_from_env()).await?;

    println!("Connected to NATS via creds!");
    let js = nats_config::jetstream_context(nc);

    let bucket = std::env::var("CFG_BUCKET").unwrap_or_else(|_| "avenabox".into());
    let key = std::env::var("CFG_KEY").unwrap_or_else(|_| "v1.macbook.unknown-source.config".into());

    let store = ensure_kv_bucket(&js, &bucket).await?;
    let central_sync_cfg = central_kv_sync_config_from_env()?;
    if let Some(sync_cfg) = central_sync_cfg.as_ref() {
        match connect_nats_with_creds(sync_cfg.servers.clone(), sync_cfg.creds_path.clone()).await {
            Ok(remote_client) => {
                let remote_js = nats_config::jetstream_context_for_domain(
                    remote_client,
                    sync_cfg.domain.as_deref(),
                );
                match ensure_kv_bucket(&remote_js, &sync_cfg.bucket).await {
                    Ok(remote_store) => {
                        if let Err(err) = mirror_remote_kv_entry_to_local(
                            &remote_store,
                            &sync_cfg.bucket,
                            &sync_cfg.key,
                            &store,
                            &key,
                        )
                        .await
                        {
                            eprintln!("[central_kv_sync] bootstrap mirror failed: {:?}", err);
                        }
                    }
                    Err(err) => {
                        eprintln!("[central_kv_sync] bootstrap remote bucket failed: {:?}", err);
                    }
                }
            }
            Err(err) => {
                eprintln!("[central_kv_sync] bootstrap connect failed: {:?}", err);
            }
        }
    }

    let cfg = load_config_from_kv(&store, &key).await?;
    println!(
        "[bootstrap] Loaded initial config from KV '{}:{}': {:?}",
        bucket, key, cfg
    );

    let (config_tx, config_rx) = watch::channel(cfg);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    unsafe {
        ljm_mode::init_ljm()?;
    }

    tokio::spawn(run_sampler(
        config_rx.clone(),
        shutdown_rx.clone(),
        js.clone(),
    ));
    tokio::spawn(watch_kv_config(
        store.clone(),
        key.clone(),
        config_tx.clone(),
        shutdown_rx.clone(),
    ));
    if let Some(sync_cfg) = central_sync_cfg {
        tokio::spawn(run_central_kv_sync(
            sync_cfg,
            store.clone(),
            key.clone(),
            shutdown_rx.clone(),
        ));
    }

    tokio::signal::ctrl_c()
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to listen for Ctrl+C: {}", e)))?;

    println!("Shutting down...");
    let _ = shutdown_tx.send(true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_kv_json(
        scans_field: &str,
        scans_value: &str,
        rate_field: &str,
        rate_value: &str,
    ) -> String {
        format!(
            r#"{{
  "labjack_name": "Unit A",
  "asset_number": 1456,
  "max_channels": 14,
  "nats_subject": "avenabox",
  "nats_stream": "labjacks",
  "rotate_secs": 300,
  "sensor_settings": {{
    "{scans_field}": {scans_value},
    "{rate_field}": {rate_value},
    "channels_enabled": [7, 11],
    "gains": 1,
    "data_formats": ["voltage", "voltage"],
    "measurement_units": ["V", "V"],
    "labjack_on_off": true
  }}
}}"#
        )
    }

    #[test]
    fn derive_interval_from_actual_rate() {
        let interval = derive_sample_interval_ns(5_000.0).expect("valid rate");
        assert_eq!(interval, 200_000);
    }

    #[test]
    fn clock_advances_monotonically_after_first_batch() {
        let mut clock = StreamClock::new(200_000);
        let (first, seq0) = clock.next_batch(10).expect("first batch");
        let (second, seq1) = clock.next_batch(10).expect("second batch");
        assert_eq!(seq0, 0);
        assert_eq!(seq1, 1);
        assert_eq!(second, first + (10 * 200_000));
    }

    #[test]
    fn clock_resets_sequence_on_new_run() {
        let mut clock = StreamClock::new(1_000);
        let _ = clock.next_batch(3).expect("first batch");
        let _ = clock.next_batch(3).expect("second batch");

        let reset_clock = StreamClock::new(1_000);
        assert_eq!(reset_clock.sequence, 0);
    }

    #[test]
    fn clock_rejects_empty_batch() {
        let mut clock = StreamClock::new(1_000);
        assert!(clock.next_batch(0).is_err());
    }

    #[test]
    fn kv_config_parses_canonical_field_names() {
        let config = sample_config_from_json(
            sample_kv_json("scans_per_read", "200", "scan_rate_hz", "5000").as_bytes(),
        )
        .expect("canonical config should parse");

        assert_eq!(config.scans_per_read, 200);
        assert_eq!(config.scan_rate_hz, 5000.0);
        assert_eq!(config.channels, vec![7, 11]);
        assert_eq!(config.asset_number, 1456);
        assert_eq!(config.nats_subject, "avenabox");
    }

    #[test]
    fn kv_config_parses_legacy_field_names() {
        let config = sample_config_from_json(
            sample_kv_json("scan_rate", "200", "sampling_rate", "1000").as_bytes(),
        )
        .expect("legacy config should parse");

        assert_eq!(config.scans_per_read, 200);
        assert_eq!(config.scan_rate_hz, 1000.0);
        assert_eq!(config.rotate_secs, 300);
    }
}
