//! The `streamer` binary: streams LabJack T7 analog input samples into NATS JetStream.
//!
//! The streamer is the first stage of the data path: streamer -> local NATS
//! JetStream -> archiver -> Parquet, with the exporter serving the Parquet files as
//! CSV over NATS. It reads its active configuration from a local KV bucket
//! (`avenabox` by default), optionally mirrors that entry from a central NATS
//! deployment, opens the LabJack over Ethernet, and publishes one FlatBuffer
//! `Scan` message per channel per LJM read. Structured configurations publish to
//! `<root>.<site>.<box>.<source>.live.chNN` (for example
//! `avenars.<site>.<box>.<source>.live.chNN`); legacy configurations publish to
//! `<root>.<asset>.data.chNN` (see the `subjects` module).
//!
//! A configuration change in KV stops the active LabJack stream and restarts it with
//! the new channel list, scan rate, and subject namespace. Setting
//! `labjack_on_off` to `false` stops sampling until the configuration changes again.
//!
//! # Configuration
//!
//! * `NATS_SERVERS` - Comma-separated local NATS server URLs. Default:
//!   `nats://127.0.0.1:4222`.
//! * `NATS_CREDS_FILE` - Credentials file for the local NATS connection. Default:
//!   `apt.creds`.
//! * `JS_DOMAIN` - JetStream domain for the local connection. Unset or empty uses
//!   the default domain.
//! * `CFG_BUCKET` - Local KV bucket that holds the streamer configuration. Also the
//!   fallback for `CENTRAL_CFG_BUCKET`. Default: `avenabox`.
//! * `CFG_KEY` - Key of the configuration entry in that bucket. Also the fallback
//!   for `CENTRAL_CFG_KEY`. Default: `unknown-site.macbook.unknown-source.config`.
//! * `CENTRAL_NATS_SERVERS` - Comma-separated central NATS server URLs. When neither
//!   this nor `CFG_NATS_SERVERS` is set, central mirroring is off.
//! * `CFG_NATS_SERVERS` - Fallback for `CENTRAL_NATS_SERVERS`.
//! * `CENTRAL_NATS_CREDS_FILE` - Credentials file for the central connection.
//!   Default: the value of `NATS_CREDS_FILE`, or `apt.creds`.
//! * `CENTRAL_CFG_BUCKET` - Central KV bucket to mirror from. Default: `CFG_BUCKET`,
//!   or `avenabox`.
//! * `CENTRAL_CFG_KEY` - Central KV key to mirror from. Default: `CFG_KEY`, or
//!   `unknown-site.macbook.unknown-source.config`.
//! * `CENTRAL_JS_DOMAIN` - JetStream domain on the central server. Falls back to
//!   `CFG_JS_DOMAIN`; unset uses the default domain.
//! * `STREAM_MAX_BYTES` - Byte limit for the JetStream stream, greater than zero.
//!   Unset means unlimited (`-1`).
//! * `STREAMER_MAX_LABJACK_FAILURES` - Consecutive sampler failures before the
//!   process exits. Values that do not parse or are zero use the default. Default:
//!   `5`.
//! * `STREAMER_LABJACK_RETRY_DELAY_SECS` - Delay before restarting the sampler after
//!   a failure, in seconds. Default: `5`.
//! * `LABJACK_IP` - IPv4 address of the LabJack T7. Required unless
//!   `LABJACK_IDENTIFIER` holds an IPv4 address.
//! * `LABJACK_IDENTIFIER` - Fallback for `LABJACK_IP`, used only when it parses as
//!   an IPv4 address.
//! * `LABJACK_SERIAL` - Expected serial number. When set, a device with another
//!   serial is rejected. `ANY` or a non-numeric value disables the check.
//! * `LABJACK_NAME` - Logical device name. Only logged.
//! * `LJM_PATH` - Path to the LJM shared library (`dynlink` builds only). When unset,
//!   no path is passed to `ljmrs`.
//!
//! # Design
//!
//! * Timestamps are derived, not read. The LJM stream API returns values without
//!   timestamps, so [`StreamClock`] numbers the batches and computes each batch's
//!   first-sample time from the actual scan rate. It slews the timeline toward the
//!   system clock by at most 1 ms a minute, so drift between the LabJack crystal
//!   and the host clock does not grow over runs that last weeks, without chasing
//!   read-latency noise (see [`StreamClock`]).
//! * Publishing does not wait for JetStream acknowledgements. The acks for a batch
//!   are checked on a spawned task so a slow ack does not delay the next LabJack
//!   read, which would risk a device buffer overflow.
//! * Scans that LJM fills with `-9999.0` after a buffer overflow are published as
//!   NaN so they are not archived as real readings.
//! * After `STREAMER_MAX_LABJACK_FAILURES` consecutive failures the process exits
//!   with success, so a systemd unit that restarts only on failure leaves it down
//!   instead of looping on a missing device.
//! * Central configuration is parsed before it is written to local KV, so an invalid
//!   central value cannot replace the last working local one.

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
/// Raw top-level configuration document stored in NATS KV.
///
/// The web dashboard writes this nested shape. The streamer normalizes it into
/// [`SampleConfig`] with [`sample_config_from_nested`] before applying it to LabJack
/// and NATS runtime state. Fields the streamer does not use are still parsed, so a
/// document missing them is rejected.
struct NestedConfig {
    /// Logical LabJack name. Used as the source token in structured subjects when
    /// `source_id` is not set.
    labjack_name: String,
    /// Asset number used in legacy subjects (`<root>.<asset>.data.chNN`).
    asset_number: u32,
    /// Maximum channel count shown by the dashboard. Not used by the streamer.
    max_channels: u32,
    /// Site identifier for the structured subject namespace.
    #[serde(default)]
    site_id: Option<String>,
    /// Box identifier for the structured subject namespace.
    #[serde(default)]
    box_id: Option<String>,
    /// Source type. Carried through to [`SampleConfig`] but not used in subjects.
    #[serde(default)]
    source_type: Option<String>,
    /// Source identifier for the structured subject namespace.
    #[serde(default)]
    source_id: Option<String>,
    /// Subject root, for example `avenars`.
    nats_subject: String,
    /// Name of the JetStream stream that stores the live samples.
    nats_stream: String,
    /// File rotation period in seconds. Used by the archiver, not by the streamer.
    rotate_secs: u64,
    /// LabJack acquisition settings.
    sensor_settings: SensorSettings,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
/// Raw sensor settings section from the dashboard configuration document.
///
/// Field aliases preserve compatibility with older configuration names while
/// letting the runtime use the clearer `scans_per_read` and `scan_rate_hz`
/// names internally.
struct SensorSettings {
    /// Scans returned by each LJM stream read. Legacy name: `scan_rate`.
    #[serde(rename = "scans_per_read", alias = "scan_rate")]
    scans_per_read: i32,
    /// Requested scan rate in Hz (scans per second across all channels). Legacy
    /// name: `sampling_rate`.
    #[serde(rename = "scan_rate_hz", alias = "sampling_rate")]
    scan_rate_hz: f64,
    /// Analog input numbers to stream (`7` means `AIN7`), in scan order.
    channels_enabled: Vec<u8>,
    /// Gain setting from the dashboard. Not used by the streamer.
    gains: i32,
    /// Per-channel data format labels. Not used by the streamer.
    data_formats: Vec<String>,
    /// Per-channel unit labels. Not used by the streamer.
    measurement_units: Vec<String>,
    /// Whether sampling is enabled. `false` keeps the LabJack stream stopped.
    labjack_on_off: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Normalized streamer configuration used by the sampling loop.
///
/// This is the compact runtime shape derived from the dashboard's nested JSON.
/// Equality is used by the KV watcher ([`watch_kv_config`]) to decide whether a new
/// value should restart the LabJack stream, so a change to any field, including
/// ones the streamer does not use such as `rotate_secs`, restarts it.
struct SampleConfig {
    /// Whether sampling is enabled.
    labjack_on_off: bool,
    /// Scans returned by each LJM stream read.
    scans_per_read: i32,
    /// Requested scan rate in Hz. The device may run at a slightly different rate.
    scan_rate_hz: f64,
    /// Analog input numbers to stream, in scan order.
    channels: Vec<u8>,
    /// Asset number used in legacy subjects.
    asset_number: u32,
    /// Logical LabJack name, used as the fallback source token in subjects.
    labjack_name: String,
    /// Site identifier for structured subjects.
    site_id: Option<String>,
    /// Box identifier for structured subjects.
    box_id: Option<String>,
    /// Source type. Not used in subjects.
    source_type: Option<String>,
    /// Source identifier for structured subjects.
    source_id: Option<String>,
    /// Subject root.
    nats_subject: String,
    /// JetStream stream name.
    nats_stream: String,
    /// Archiver file rotation period in seconds. Not used by the streamer.
    rotate_secs: u64,
}

/// Converts the dashboard JSON shape into the runtime sampling configuration.
///
/// Copies the used fields and drops `max_channels`, `gains`, `data_formats` and
/// `measurement_units`.
///
/// # Arguments
///
/// * `nested` - Parsed KV document.
fn sample_config_from_nested(nested: NestedConfig) -> SampleConfig {
    let raw = nested.sensor_settings;
    SampleConfig {
        labjack_on_off: raw.labjack_on_off,
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

/// Parses a NATS KV JSON value into a runtime sampling config.
///
/// This is the deserialization boundary for streamer configuration. It accepts
/// the dashboard's nested schema ([`NestedConfig`]) and reports JSON errors as
/// `LJMError` values so the rest of the streamer can use one error type. No checks
/// beyond deserialization are done (for example, an empty channel list is accepted).
///
/// # Arguments
///
/// * `bytes` - Raw KV value, UTF-8 JSON.
///
/// # Errors
///
/// Returns `LJMError::LibraryError` if the bytes are not valid JSON, a required
/// field is missing, or a field has the wrong type.
fn sample_config_from_json(bytes: &[u8]) -> Result<SampleConfig, LJMError> {
    let nested_cfg: NestedConfig = serde_json::from_slice(bytes)
        .map_err(|e| LJMError::LibraryError(format!("Config JSON parse error: {}", e)))?;
    Ok(sample_config_from_nested(nested_cfg))
}

#[derive(Debug, Clone)]
/// Connection and key details for mirroring configuration from central NATS.
///
/// Built by [`central_kv_sync_config_from_env`].
struct CentralKvSyncConfig {
    /// Central NATS server addresses.
    servers: Vec<async_nats::ServerAddr>,
    /// Credentials file for the central connection.
    creds_path: String,
    /// Central KV bucket name.
    bucket: String,
    /// Central KV key to mirror.
    key: String,
    /// JetStream domain on the central server, if any.
    domain: Option<String>,
}

/// RAII guard that stops streaming and closes the LabJack handle on drop.
///
/// The sampling loop exits through several error and shutdown paths, so cleanup
/// is tied to ownership of the handle instead of each branch remembering to
/// close the device.
struct LabJackGuard {
    /// LJM device handle to stop and close.
    handle: i32,
}

impl Drop for LabJackGuard {
    /// Stops any active LabJack stream and closes the device handle.
    ///
    /// Errors from LJM are ignored, since there is nothing useful to do with them
    /// during cleanup.
    fn drop(&mut self) {
        let _ = LJMLibrary::stream_stop(self.handle);
        let _ = LJMLibrary::close_jack(self.handle);
    }
}

/// Reads the optional JetStream stream byte limit from `STREAM_MAX_BYTES`.
///
/// A missing or empty value returns `-1`, JetStream's value for no limit.
///
/// # Returns
///
/// The byte limit, or `-1` when unset.
///
/// # Errors
///
/// Returns an error if the value is not an integer or is not greater than zero.
fn stream_max_bytes_from_env() -> Result<i64, LJMError> {
    let Some(raw) = env_nonempty("STREAM_MAX_BYTES") else {
        return Ok(-1);
    };

    let parsed = raw.parse::<i64>().map_err(|e| {
        LJMError::LibraryError(format!("Invalid STREAM_MAX_BYTES value '{}': {}", raw, e))
    })?;

    if parsed <= 0 {
        return Err(LJMError::LibraryError(format!(
            "Invalid STREAM_MAX_BYTES value '{}': must be greater than zero",
            raw
        )));
    }

    Ok(parsed)
}

/// Reads the number of consecutive sampler failures after which the streamer exits.
///
/// Reads `STREAMER_MAX_LABJACK_FAILURES`. A missing, unparsable or zero value gives
/// the default of 5.
fn max_labjack_failures_from_env() -> usize {
    std::env::var("STREAMER_MAX_LABJACK_FAILURES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(5)
}

/// Reads the delay between sampler restarts after a failed run.
///
/// Reads `STREAMER_LABJACK_RETRY_DELAY_SECS` in whole seconds. A missing or
/// unparsable value gives the default of 5 s. `0` is accepted and means no delay.
fn labjack_retry_delay_from_env() -> Duration {
    let secs = std::env::var("STREAMER_LABJACK_RETRY_DELAY_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(5);
    Duration::from_secs(secs)
}

/// Creates or reconciles the JetStream stream used for live LabJack samples.
///
/// The stream is configured with file storage, limit retention, old-message
/// discard, no message or consumer limits, the byte limit from `STREAM_MAX_BYTES`,
/// and the subject wildcard needed for the current source. Existing subjects are
/// kept during namespace migrations so historical consumers remain valid until they
/// are deliberately retired. If the stream already has these settings, nothing is
/// written.
///
/// # Arguments
///
/// * `js` - JetStream context of the local NATS server.
/// * `stream_name` - Stream name from the configuration (`nats_stream`).
/// * `subject` - Subject wildcard for this source, from
///   `subjects::live_labjack_stream_subject`.
///
/// # Errors
///
/// Returns an error if `STREAM_MAX_BYTES` is invalid or if JetStream rejects the
/// create or update request.
async fn ensure_stream_exists(
    js: &jetstream::Context,
    stream_name: &str,
    subject: &str,
) -> Result<(), LJMError> {
    let max_bytes = stream_max_bytes_from_env()?;
    let mut desired_subjects = vec![subject.to_string()];

    if let Ok(stream) = js.get_stream(stream_name).await {
        let info = stream.cached_info();
        desired_subjects = info.config.subjects.clone();
        if !desired_subjects.iter().any(|existing| existing == subject) {
            desired_subjects.push(subject.to_string());
        }
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
            stream_name, info.config.subjects, desired_subjects, info.config.max_bytes, max_bytes
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

/// Opens a KV bucket, creating it if it does not exist.
///
/// A new bucket keeps 5 revisions per key; other settings use the `async_nats`
/// defaults. Used for both the local bucket and the central one, so the streamer
/// also creates the central bucket if it is missing.
///
/// # Arguments
///
/// * `js` - JetStream context of the server that holds the bucket.
/// * `bucket` - Bucket name.
///
/// # Errors
///
/// Returns an error if the bucket does not exist and cannot be created.
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

/// Loads and parses the initial streamer configuration from local NATS KV.
///
/// # Arguments
///
/// * `store` - Local KV bucket.
/// * `key` - Configuration key.
///
/// # Errors
///
/// Returns an error if the key does not exist, the KV read fails, or the value does
/// not parse (see [`sample_config_from_json`]).
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

/// Returns a trimmed environment variable value when it is set and non-empty.
fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Returns the credentials file used for local NATS connections.
///
/// Reads `NATS_CREDS_FILE`, defaulting to `apt.creds`.
fn local_creds_path_from_env() -> String {
    std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into())
}

/// Builds central-to-local KV mirroring settings from environment variables.
///
/// If neither `CENTRAL_NATS_SERVERS` nor `CFG_NATS_SERVERS` is set, the streamer
/// runs against local KV only and this returns `Ok(None)`. Each `CENTRAL_*`
/// variable falls back to its `CFG_*` counterpart; see the module docs for the full
/// list and defaults.
///
/// # Errors
///
/// Returns an error if the server list contains an invalid URL or no usable URL.
fn central_kv_sync_config_from_env() -> Result<Option<CentralKvSyncConfig>, LJMError> {
    let Some(raw_servers) =
        env_nonempty("CENTRAL_NATS_SERVERS").or_else(|| env_nonempty("CFG_NATS_SERVERS"))
    else {
        return Ok(None);
    };

    let servers = nats_config::servers_from_env_var("CENTRAL_NATS_SERVERS", &raw_servers)
        .map_err(LJMError::LibraryError)?;
    let creds_path =
        env_nonempty("CENTRAL_NATS_CREDS_FILE").unwrap_or_else(local_creds_path_from_env);
    let bucket = env_nonempty("CENTRAL_CFG_BUCKET")
        .or_else(|| env_nonempty("CFG_BUCKET"))
        .unwrap_or_else(|| "avenabox".to_string());
    let key = env_nonempty("CENTRAL_CFG_KEY")
        .or_else(|| env_nonempty("CFG_KEY"))
        .unwrap_or_else(|| "unknown-site.macbook.unknown-source.config".to_string());
    let domain = env_nonempty("CENTRAL_JS_DOMAIN").or_else(|| env_nonempty("CFG_JS_DOMAIN"));

    Ok(Some(CentralKvSyncConfig {
        servers,
        creds_path,
        bucket,
        key,
        domain,
    }))
}

/// Performs one central-to-local KV mirror pass during streamer bootstrap.
///
/// Connects to central NATS, opens (or creates) the central bucket, and copies the
/// configured key into local KV with [`mirror_remote_kv_entry_to_local`]. The
/// connection is dropped afterward; [`run_central_kv_sync`] keeps its own.
///
/// # Arguments
///
/// * `sync_cfg` - Central connection and key details.
/// * `local_store` - Local KV bucket to write into.
/// * `local_key` - Local configuration key.
///
/// # Returns
///
/// `true` if the local entry was written, `false` if it already matched.
///
/// # Errors
///
/// Returns an error if the connection, bucket setup, or mirror step fails.
async fn mirror_central_kv_once(
    sync_cfg: &CentralKvSyncConfig,
    local_store: &kv::Store,
    local_key: &str,
) -> Result<bool, LJMError> {
    let client =
        connect_nats_with_creds(sync_cfg.servers.clone(), sync_cfg.creds_path.clone()).await?;
    let remote_js = nats_config::jetstream_context_for_domain(client, sync_cfg.domain.as_deref());
    let remote_store = ensure_kv_bucket(&remote_js, &sync_cfg.bucket).await?;
    mirror_remote_kv_entry_to_local(
        &remote_store,
        &sync_cfg.bucket,
        &sync_cfg.key,
        local_store,
        local_key,
    )
    .await
}

/// Connects to NATS using a credentials file and explicit server list.
///
/// # Arguments
///
/// * `servers` - Server addresses to try.
/// * `creds_path` - Path to the NATS credentials file.
///
/// # Errors
///
/// Returns an error if the credentials file cannot be loaded or the connection
/// fails.
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

/// Copies one remote KV configuration entry into the local KV bucket if needed.
///
/// The remote value is parsed before it is written locally so an invalid central
/// configuration cannot replace the last known local configuration. The write is
/// skipped when the local bytes already equal the remote bytes, so an unchanged
/// value does not add a KV revision.
///
/// # Arguments
///
/// * `remote_store` - Central KV bucket.
/// * `remote_bucket` - Central bucket name, used only in the log line.
/// * `remote_key` - Central key to read.
/// * `local_store` - Local KV bucket.
/// * `local_key` - Local key to write.
///
/// # Returns
///
/// `true` if the local entry was written, `false` if it already matched.
///
/// # Errors
///
/// Returns an error if the remote key is missing or cannot be read, the remote
/// value does not parse, the local key cannot be read, or the local write fails.
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
        remote_bucket, remote_key, local_key
    );
    Ok(true)
}

/// Watches central NATS KV and mirrors valid updates into the local bucket.
///
/// Each pass connects, opens the central bucket, mirrors the current value once,
/// and then watches the key. Only `Put` operations are mirrored; deletes and purges
/// are logged and ignored, and invalid values are skipped. After a connection,
/// bucket, or watch failure, or when the watch ends, the task waits 5 s and starts a
/// new pass. It returns when the shutdown channel becomes `true`.
///
/// Writes to local KV are then picked up by [`watch_kv_config`] like any other
/// update.
///
/// # Arguments
///
/// * `sync_cfg` - Central connection and key details.
/// * `local_store` - Local KV bucket to write into.
/// * `local_key` - Local configuration key.
/// * `shutdown_rx` - Shutdown flag shared with the rest of the process.
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

        if let Err(err) = mirror_remote_kv_entry_to_local(
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

/// Watches local NATS KV for streamer configuration updates.
///
/// Parsed changes are sent through a watch channel so the active sampler can
/// stop cleanly and restart with the new settings. A value is sent only when it
/// differs from the last one in the channel. Non-`Put` operations, values that do
/// not parse, and watch stream errors are logged and skipped. The task returns when
/// the watch cannot be set up, the watch ends, or shutdown is signaled.
///
/// # Arguments
///
/// * `store` - Local KV bucket.
/// * `key` - Configuration key to watch.
/// * `config_tx` - Sender read by [`run_sampler`] and [`sample_with_config`].
/// * `shutdown_rx` - Shutdown flag shared with the rest of the process.
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

/// Length of each window over which clock drift is measured, in nanoseconds (60 s).
///
/// Long enough that at least one read in the window usually arrives with low
/// latency, so the window minimum approximates drift rather than scheduling delay.
const CLOCK_CHECK_WINDOW_NS: u128 = 60_000_000_000;
/// Smallest window offset that is acted on, in nanoseconds (5 ms).
const CLOCK_SLEW_THRESHOLD_NS: i128 = 5_000_000;
/// Largest adjustment made in one window when slewing, in nanoseconds (1 ms).
///
/// The actual limit is the smaller of this and half the sample interval (see
/// [`StreamClock::max_slew_ns`]), so an adjustment never makes timestamps go
/// backwards or collide. One millisecond per minute follows crystal drift of up
/// to about 16 ppm at 100 Hz, and a quarter millisecond (the limit at 2 kHz) up to
/// about 4 ppm; MU1 measured about 2 ppm.
const CLOCK_MAX_SLEW_NS: i128 = 1_000_000;
/// Windows over which the slewing offset is taken as the smallest window offset
/// (15, about a quarter of an hour).
///
/// A single minute is not enough on a busy host: on MU1 the read latency stayed
/// high for several minutes at a time before dropping back, so each minute's
/// minimum overstated the offset. Over 15 minutes at least one fast read almost
/// always occurs, while 2 ppm of drift moves the offset by under 2 ms.
const CLOCK_OFFSET_HISTORY_WINDOWS: usize = 15;
/// Offset treated as a system clock step rather than drift, in nanoseconds (2 s).
const CLOCK_STEP_THRESHOLD_NS: i128 = 2_000_000_000;
/// Consecutive windows a step-sized offset must persist before the timeline jumps.
const CLOCK_STEP_WINDOWS: u32 = 3;
/// Windows between summary log lines about slewing (60, about one hour).
const CLOCK_LOG_EVERY_WINDOWS: u32 = 60;

#[derive(Debug, Clone)]
/// Tracks timestamp continuity for successive LabJack stream batches.
///
/// The LabJack stream API returns value batches without per-sample timestamps,
/// so the streamer derives the first timestamp and sequence number for each
/// published batch from the actual stream rate.
///
/// The LabJack crystal and the system clock disagree by a few ppm, which added
/// up to about 4-5 s over a 26-day run on MU1. To stop that accumulating, the
/// clock compares each batch's last-sample time with the system clock and keeps
/// the smallest difference seen in each 60 s window.
///
/// That minimum is not a clean drift measurement on a busy host. On MU1, with the
/// camera software loading the CPU, read latency wandered by tens to hundreds of
/// milliseconds from one minute to the next. An earlier version jumped the
/// timeline by the full window minimum and so chased that noise, stepping the
/// timestamps by up to 430 ms every minute. The clock therefore never jumps for
/// ordinary offsets. It slews:
///
/// * The offset used for slewing is the smallest window minimum over the last 15
///   windows, because on a busy host a whole minute can pass without a fast read.
/// * An offset of at least 5 ms that points the same way in two consecutive
///   windows moves the timeline by at most 1 ms (less at high sample rates, see
///   [`StreamClock::max_slew_ns`]). Genuine drift is steady and gets followed;
///   latency noise changes sign or size and mostly does not.
/// * Only an offset of 2 s or more that persists for three windows, which is what
///   a system clock step looks like (for example chronyd correcting a badly set
///   clock), moves the timeline in one jump. That jump is logged.
///
/// The first batch of a run is anchored to the system clock at the moment it
/// arrives, so whatever delay that one read had is built into the timeline. At the
/// end of the first window the clock therefore corrects the anchor once, in full,
/// by the smallest offset seen in that window, and logs it. After that only the
/// rules above apply.
///
/// Slewing is summarized in the log about once an hour.
///
/// A new clock is created for every sampling run, so sequence numbers restart at 0
/// and the first batch of each run is anchored to the system clock again.
struct StreamClock {
    /// Time between consecutive scans, in nanoseconds, from the actual scan rate.
    sample_interval_ns: u64,
    /// Expected Unix timestamp (ns) of the first sample of the next batch.
    next_first_sample_unix_ns: u64,
    /// Sequence number that the next batch will get.
    sequence: u64,
    /// Samples per channel in the most recent batch. Recorded but not read.
    last_batch_samples: usize,
    /// Whether the first batch has been timestamped.
    run_started: bool,
    /// System time (Unix ns) at which the current drift window started.
    window_start_unix_ns: u128,
    /// Smallest receive-time minus last-sample-time seen in the current window, in
    /// nanoseconds. `None` until the window has a measurement.
    window_min_error_ns: Option<i128>,
    /// Window minimum of the previous window, in nanoseconds.
    previous_window_error_ns: Option<i128>,
    /// Consecutive windows whose offset was at least [`CLOCK_STEP_THRESHOLD_NS`]
    /// in the same direction.
    step_windows: u32,
    /// This window's own minimum from the previous window, for step detection.
    previous_raw_offset_ns: Option<i128>,
    /// Total slew applied since the last summary log line, in nanoseconds.
    slewed_since_log_ns: i128,
    /// Whether the first window has ended and the initial anchor was checked.
    anchor_checked: bool,
    /// Window minima of the most recent windows, oldest first, at most
    /// [`CLOCK_OFFSET_HISTORY_WINDOWS`].
    recent_offsets_ns: std::collections::VecDeque<i128>,
    /// Windows completed since the last summary log line.
    windows_since_log: u32,
}

/// What the clock decided to do at the end of one drift window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClockAdjustment {
    /// Leave the timeline alone.
    None,
    /// Move the timeline by this many nanoseconds, at most [`CLOCK_MAX_SLEW_NS`].
    Slew(i128),
    /// Move the timeline by this many nanoseconds in one jump (a clock step).
    Step(i128),
    /// Correct the initial anchor by this many nanoseconds, once per run.
    Anchor(i128),
}

/// Decides how to adjust the timeline from two consecutive window offsets.
///
/// # Arguments
///
/// * `current_ns` - Minimum receive-time minus sample-time in the window that
///   just ended, in nanoseconds. Positive means the timeline is behind the system
///   clock.
/// * `previous_ns` - The same measure for the window before, if there was one.
/// * `step_windows` - Consecutive windows, including this one, whose offset was at
///   least [`CLOCK_STEP_THRESHOLD_NS`] in this direction.
/// * `max_slew_ns` - Largest slew allowed, in nanoseconds (positive).
///
/// # Returns
///
/// [`ClockAdjustment::Step`] when a step-sized offset has lasted
/// [`CLOCK_STEP_WINDOWS`] windows, [`ClockAdjustment::Slew`] when an offset of at
/// least [`CLOCK_SLEW_THRESHOLD_NS`] points the same way as in the previous window,
/// and [`ClockAdjustment::None`] otherwise.
///
/// # Examples
///
/// ```text
/// decide_adjustment(64 ms, Some(60 ms), 0, 1 ms)   -> Slew(+1 ms)
/// decide_adjustment(64 ms, Some(-400 ms), 0, 1 ms) -> None     (direction changed)
/// decide_adjustment(3 s, Some(3 s), 3, 1 ms)       -> Step(+3 s)
/// ```
fn decide_adjustment(
    current_ns: i128,
    previous_ns: Option<i128>,
    step_windows: u32,
    max_slew_ns: i128,
) -> ClockAdjustment {
    if current_ns.abs() >= CLOCK_STEP_THRESHOLD_NS {
        return if step_windows >= CLOCK_STEP_WINDOWS {
            ClockAdjustment::Step(current_ns)
        } else {
            ClockAdjustment::None
        };
    }
    let persistent = previous_ns.is_some_and(|previous| {
        previous.abs() >= CLOCK_SLEW_THRESHOLD_NS && previous.signum() == current_ns.signum()
    });
    if current_ns.abs() >= CLOCK_SLEW_THRESHOLD_NS && persistent {
        ClockAdjustment::Slew(current_ns.clamp(-max_slew_ns, max_slew_ns))
    } else {
        ClockAdjustment::None
    }
}

impl StreamClock {
    /// Creates a stream clock that has not yet seen a batch.
    ///
    /// # Arguments
    ///
    /// * `sample_interval_ns` - Time between scans in nanoseconds, from
    ///   [`derive_sample_interval_ns`].
    fn new(sample_interval_ns: u64) -> Self {
        Self {
            sample_interval_ns,
            next_first_sample_unix_ns: 0,
            sequence: 0,
            last_batch_samples: 0,
            run_started: false,
            window_start_unix_ns: 0,
            window_min_error_ns: None,
            previous_window_error_ns: None,
            step_windows: 0,
            previous_raw_offset_ns: None,
            slewed_since_log_ns: 0,
            windows_since_log: 0,
            anchor_checked: false,
            recent_offsets_ns: std::collections::VecDeque::new(),
        }
    }

    /// Returns the largest slew allowed per window, in nanoseconds.
    ///
    /// The smaller of [`CLOCK_MAX_SLEW_NS`] and half the sample interval, so that
    /// even a backward slew leaves consecutive timestamps strictly increasing.
    fn max_slew_ns(&self) -> i128 {
        CLOCK_MAX_SLEW_NS
            .min(self.sample_interval_ns as i128 / 2)
            .max(1)
    }

    /// Returns the first sample timestamp and sequence for the next batch.
    ///
    /// Reads the system clock and calls [`Self::next_batch_at`]. The first batch is
    /// anchored so that its last sample falls at the current wall-clock time. Each
    /// later batch starts where the previous one ended, with the drift handling
    /// described in the type docs.
    ///
    /// # Arguments
    ///
    /// * `batch_samples` - Samples per channel in the batch (scans in the batch).
    ///
    /// # Returns
    ///
    /// The Unix timestamp of the batch's first sample in nanoseconds, and the
    /// batch sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error if `batch_samples` is zero, if the system clock is before
    /// the Unix epoch, or if a timestamp overflows `u64`.
    fn next_batch(&mut self, batch_samples: usize) -> Result<(u64, u64), LJMError> {
        let now_ns = unix_time_now_ns()?;
        self.next_batch_at(batch_samples, now_ns)
    }

    /// [`Self::next_batch`] with an explicit receive time, for testing.
    ///
    /// For every batch after the first, it measures the receive time minus the
    /// expected time of the batch's last sample and keeps the window minimum. When
    /// the window has lasted [`CLOCK_CHECK_WINDOW_NS`], [`decide_adjustment`]
    /// chooses whether to slew or step, the adjustment is added to this batch's
    /// first-sample time, and later batches continue from there. The window then
    /// restarts at `now_ns`.
    ///
    /// # Arguments
    ///
    /// * `batch_samples` - Samples per channel in the batch.
    /// * `now_ns` - Receive time of the batch, Unix nanoseconds.
    ///
    /// # Returns
    ///
    /// The Unix timestamp of the batch's first sample in nanoseconds, and the
    /// batch sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error if `batch_samples` is zero or if the initial, corrected, or
    /// next timestamp does not fit in `u64`.
    fn next_batch_at(&mut self, batch_samples: usize, now_ns: u64) -> Result<(u64, u64), LJMError> {
        if batch_samples == 0 {
            return Err(LJMError::LibraryError(
                "Received empty batch; refusing to guess timestamps".to_string(),
            ));
        }

        let last_offset_ns = (batch_samples.saturating_sub(1) as u128)
            .saturating_mul(self.sample_interval_ns as u128);
        let mut first_sample_unix_ns = if !self.run_started {
            let first = (now_ns as u128).saturating_sub(last_offset_ns);
            self.window_start_unix_ns = now_ns as u128;
            u64::try_from(first).map_err(|_| {
                LJMError::LibraryError("Initial stream timestamp overflowed u64".to_string())
            })?
        } else {
            self.next_first_sample_unix_ns
        };

        if self.run_started {
            let last_sample_ns = first_sample_unix_ns as i128 + last_offset_ns as i128;
            let error_ns = now_ns as i128 - last_sample_ns;
            self.window_min_error_ns = Some(match self.window_min_error_ns {
                Some(min) => min.min(error_ns),
                None => error_ns,
            });

            if (now_ns as u128).saturating_sub(self.window_start_unix_ns) >= CLOCK_CHECK_WINDOW_NS {
                let offset_ns = self.window_min_error_ns.take().unwrap_or(0);
                self.window_start_unix_ns = now_ns as u128;

                let same_direction = self
                    .previous_raw_offset_ns
                    .is_some_and(|previous| previous.signum() == offset_ns.signum());
                self.step_windows = if offset_ns.abs() >= CLOCK_STEP_THRESHOLD_NS {
                    if same_direction {
                        self.step_windows + 1
                    } else {
                        1
                    }
                } else {
                    0
                };

                self.previous_raw_offset_ns = Some(offset_ns);
                let adjustment = if !self.anchor_checked {
                    // First window: fix the latency built into the initial anchor.
                    self.anchor_checked = true;
                    if offset_ns.abs() >= CLOCK_SLEW_THRESHOLD_NS {
                        println!(
                            "[clock] Corrected the initial anchor by {:+.3} ms after the first minute",
                            offset_ns as f64 / 1e6
                        );
                        ClockAdjustment::Anchor(offset_ns)
                    } else {
                        ClockAdjustment::None
                    }
                } else {
                    self.recent_offsets_ns.push_back(offset_ns);
                    if self.recent_offsets_ns.len() > CLOCK_OFFSET_HISTORY_WINDOWS {
                        self.recent_offsets_ns.pop_front();
                    }
                    let floor_ns = self
                        .recent_offsets_ns
                        .iter()
                        .copied()
                        .min()
                        .unwrap_or(offset_ns);
                    // A clock step shows in every recent window at once, so step detection
                    // uses this window; slewing uses the floor over the recent history.
                    let decision_ns = if offset_ns.abs() >= CLOCK_STEP_THRESHOLD_NS {
                        offset_ns
                    } else {
                        floor_ns
                    };
                    decide_adjustment(
                        decision_ns,
                        self.previous_window_error_ns,
                        self.step_windows,
                        self.max_slew_ns(),
                    )
                };
                self.previous_window_error_ns = Some(
                    self.recent_offsets_ns
                        .iter()
                        .copied()
                        .min()
                        .unwrap_or(offset_ns),
                );

                let applied_ns = match adjustment {
                    ClockAdjustment::None => 0,
                    ClockAdjustment::Slew(ns) => {
                        self.slewed_since_log_ns += ns;
                        ns
                    }
                    ClockAdjustment::Anchor(ns) => {
                        // The anchor is now corrected; start the persistence rule afresh.
                        self.previous_window_error_ns = None;
                        ns
                    }
                    ClockAdjustment::Step(ns) => {
                        println!(
                            "[clock] Offset of {:+.3} s persisted for {} minutes; stepped stream timestamps at sequence {} (system clock step?)",
                            ns as f64 / 1e9,
                            CLOCK_STEP_WINDOWS,
                            self.sequence
                        );
                        self.step_windows = 0;
                        self.previous_window_error_ns = None;
                        ns
                    }
                };
                if applied_ns != 0 {
                    // Offsets measured before the adjustment are now off by applied_ns.
                    for past in self.recent_offsets_ns.iter_mut() {
                        *past -= applied_ns;
                    }
                    if let Some(previous) = self.previous_window_error_ns.as_mut() {
                        *previous -= applied_ns;
                    }
                    let corrected = first_sample_unix_ns as i128 + applied_ns;
                    first_sample_unix_ns = u64::try_from(corrected).map_err(|_| {
                        LJMError::LibraryError(
                            "Corrected stream timestamp overflowed u64".to_string(),
                        )
                    })?;
                }

                self.windows_since_log += 1;
                if self.windows_since_log >= CLOCK_LOG_EVERY_WINDOWS {
                    if self.slewed_since_log_ns != 0 {
                        println!(
                            "[clock] Slewed stream timestamps by {:+.3} ms over the last {} minutes; latest window offset {:+.3} ms",
                            self.slewed_since_log_ns as f64 / 1e6,
                            self.windows_since_log,
                            offset_ns as f64 / 1e6
                        );
                    }
                    self.slewed_since_log_ns = 0;
                    self.windows_since_log = 0;
                }
            }
        }

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

/// Value LJM inserts for scans lost to a stream buffer overflow when it
/// auto-recovers (LJM's `LJM_DUMMY_VALUE` constant).
const LJM_DUMMY_VALUE: f64 = -9999.0;

/// Replaces LJM auto-recovery placeholders with NaN and returns how many.
///
/// LJM keeps the scan count intact when it recovers, so timestamps stay
/// correct, but the placeholder would otherwise be archived as a real
/// -9999 V reading.
///
/// # Arguments
///
/// * `batch` - Interleaved samples from one LJM read, changed in place.
///
/// # Returns
///
/// The number of values replaced, across all channels.
///
/// # Examples
///
/// ```text
/// [3.72, -9999.0, 3.71]  ->  [3.72, NaN, 3.71], returns 1
/// ```
fn replace_dummy_samples(batch: &mut [f64]) -> usize {
    let mut replaced = 0;
    for v in batch.iter_mut() {
        if *v == LJM_DUMMY_VALUE {
            *v = f64::NAN;
            replaced += 1;
        }
    }
    replaced
}

/// Returns the current Unix timestamp in nanoseconds.
///
/// # Errors
///
/// Returns an error if the system clock is before the Unix epoch or the value does
/// not fit in `u64`.
fn unix_time_now_ns() -> Result<u64, LJMError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| LJMError::LibraryError(format!("system clock before Unix epoch: {e}")))?;
    u64::try_from(duration.as_nanos())
        .map_err(|_| LJMError::LibraryError("system time nanoseconds overflowed u64".to_string()))
}

/// Converts the actual LabJack scan rate into a sample interval in nanoseconds.
///
/// The interval is `1e9 / actual_rate`, rounded to the nearest nanosecond.
///
/// # Arguments
///
/// * `actual_rate` - Scan rate in Hz returned by `stream_start`.
///
/// # Errors
///
/// Returns an error if the rate is not finite or not positive, or if the rounded
/// interval is zero.
///
/// # Examples
///
/// ```text
/// 5000.0 Hz  ->  200_000 ns
/// ```
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

/// Runs one LabJack streaming session with a fixed configuration.
///
/// In order, the session:
///
/// 1. Creates or updates the JetStream stream ([`ensure_stream_exists`]).
/// 2. Opens and checks the LabJack (`labjack::open_labjack_from_env`) and wraps
///    the handle in a [`LabJackGuard`].
/// 3. Configures all analog inputs as single-ended (T7 only), +/-10 V range,
///    resolution index 0 and `STREAM_SETTLING_US` 0.
/// 4. Starts the hardware stream and derives the sample interval from the rate the
///    device actually runs at.
/// 5. Reads batches on a blocking thread, replaces LJM dummy values with NaN,
///    splits each batch by channel, encodes one FlatBuffer `Scan` per channel, and
///    publishes it to the channel's subject.
///
/// It runs until the configuration changes, shutdown is signaled, or an error
/// occurs. A publish that fails is logged and that channel's message for the batch
/// is dropped; it does not end the session.
///
/// # Arguments
///
/// * `run_id` - Run counter from [`run_sampler`], used in log lines.
/// * `cfg` - Configuration for this session.
/// * `config_rx` - Configuration channel; any change ends the session.
/// * `shutdown_rx` - Shutdown flag; `true` ends the session.
/// * `js` - JetStream context of the local NATS server.
///
/// # Returns
///
/// `Ok(())` when the session stopped because of a configuration change or
/// shutdown.
///
/// # Errors
///
/// Returns an error if stream setup fails, the LabJack cannot be opened or
/// configured, a channel number has no `AIN` register, `stream_start` fails or
/// reports an invalid rate, the reader stops (for example on an LJM read error),
/// a batch is empty or not a multiple of the channel count, or a timestamp cannot
/// be computed.
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
            cfg.asset_number,
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
    // Stops the stream and closes the handle on every return path below.
    let _guard = LabJackGuard { handle };

    let info = LJMLibrary::get_handle_info(handle)?;
    println!(
        "[run #{run_id}] Connected to {:?} (serial {})",
        info.device_type, info.serial_number
    );

    // 199 selects GND as the negative input, i.e. single-ended measurements.
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

    // Up to 32 raw batches can wait between the reader thread and this task.
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
    let channel_subjects: Vec<String> = cfg
        .channels
        .iter()
        .map(|ch| {
            subjects::live_labjack_channel_subject(
                &cfg.nats_subject,
                cfg.asset_number,
                *ch,
                cfg.site_id.as_deref(),
                cfg.box_id.as_deref(),
                Some(&cfg.labjack_name),
                cfg.source_type.as_deref(),
                cfg.source_id.as_deref(),
            )
        })
        .collect();

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

                // LJM returns scans interleaved: ch0, ch1, ..., ch0, ch1, ...
                let batch_samples = batch.len() / num_channels;
                let (first_sample_unix_ns, sequence) = clock.next_batch(batch_samples)?;
                let mut batch = batch;
                let skipped = replace_dummy_samples(&mut batch);
                if skipped > 0 {
                    eprintln!(
                        "[run #{run_id}] LabJack skipped {} sample(s) in sequence {}; stored as NaN",
                        skipped, sequence
                    );
                }
                let scans = batch.chunks(num_channels);
                let mut per_channel: Vec<Vec<f64>> = (0..num_channels)
                    .map(|_| Vec::with_capacity(scans.len()))
                    .collect();

                for scan in batch.chunks(num_channels) {
                    for (i, v) in scan.iter().enumerate() {
                        per_channel[i].push(*v);
                    }
                }

                let mut acks = Vec::with_capacity(num_channels);
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

                    match js.publish(channel_subjects[i].clone(), data.into()).await {
                        Ok(ack) => acks.push(ack),
                        Err(e) => eprintln!("[run #{run_id}] Failed to publish to NATS: {}", e),
                    }
                }
                // Confirm JetStream stored every channel's message without
                // holding up the next LabJack read.
                tokio::spawn(async move {
                    let mut failed = 0usize;
                    let mut last_error = None;
                    for ack in acks {
                        if let Err(e) = ack.await {
                            failed += 1;
                            last_error = Some(e);
                        }
                    }
                    if let Some(e) = last_error {
                        eprintln!(
                            "[run #{run_id}] JetStream did not confirm {failed} message(s) of sequence {sequence}: {e}"
                        );
                    }
                });
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

/// Supervises LabJack sampling across config changes and transient failures.
///
/// Runs [`sample_with_config`] in a loop with the latest configuration. After a
/// configuration change it restarts at once; after an error it waits for
/// `STREAMER_LABJACK_RETRY_DELAY_SECS` first. While `labjack_on_off` is `false` it
/// waits for the next configuration change instead of opening the device. Any
/// session error counts as a failure, including NATS stream setup errors. The
/// failure count resets after a session that ends without error and while sampling
/// is disabled.
///
/// # Arguments
///
/// * `config_rx` - Configuration channel fed by [`watch_kv_config`].
/// * `shutdown_rx` - Shutdown flag shared with the rest of the process.
/// * `js` - JetStream context of the local NATS server.
///
/// # Returns
///
/// `true` when consecutive failures reached `STREAMER_MAX_LABJACK_FAILURES`, which
/// lets the process exit cleanly instead of relying on service restarts. `false`
/// on shutdown, or when the configuration channel closes while sampling is
/// disabled.
async fn run_sampler(
    mut config_rx: tokio::sync::watch::Receiver<SampleConfig>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    js: jetstream::Context,
) -> bool {
    let mut run_id = 0;
    let mut consecutive_failures = 0usize;
    let max_failures = max_labjack_failures_from_env();
    let retry_delay = labjack_retry_delay_from_env();
    loop {
        if *shutdown_rx.borrow() {
            println!("[run_sampler] Sampler shutting down...");
            return false;
        }
        run_id += 1;
        let cfg = config_rx.borrow().clone();
        if !cfg.labjack_on_off {
            consecutive_failures = 0;
            println!("[run_sampler] LabJack stream disabled; waiting for config update.");
            tokio::select! {
                changed = config_rx.changed() => {
                    if changed.is_err() {
                        eprintln!("[run_sampler] Config channel closed while disabled.");
                        return false;
                    }
                    continue;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        println!("[run_sampler] Sampler shutting down while disabled...");
                        return false;
                    }
                }
            }
            continue;
        }
        println!(
            "[run_sampler] Starting sampler run #{run_id} with {:?}",
            cfg
        );

        let mut had_error = false;
        match sample_with_config(run_id, cfg, &mut config_rx, &mut shutdown_rx, &js).await {
            Ok(()) => {
                consecutive_failures = 0;
            }
            Err(e) => {
                had_error = true;
                consecutive_failures = consecutive_failures.saturating_add(1);
                eprintln!(
                    "[run_sampler] Sampler error {}/{}: {:?}",
                    consecutive_failures, max_failures, e
                );
                if consecutive_failures >= max_failures {
                    eprintln!(
                        "[run_sampler] Reached {} consecutive LabJack sampler failures; exiting cleanly so systemd does not auto-restart the streamer.",
                        max_failures
                    );
                    return true;
                }
            }
        }

        if *shutdown_rx.borrow() {
            println!("[run_sampler] Shutdown detected after sampler error/config change");
            return false;
        }

        if had_error {
            println!(
                "[run_sampler] Restarting sampler after error in {}s...",
                retry_delay.as_secs()
            );
            tokio::select! {
                _ = tokio::time::sleep(retry_delay) => {}
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        println!("[run_sampler] Sampler shutting down during retry delay...");
                        return false;
                    }
                }
            }
        } else {
            println!("[run_sampler] Restarting sampler after config change...");
        }
    }
}

#[tokio::main]
/// Starts the streamer service, configuration watchers, and sampler supervisor.
///
/// Connects to local NATS, opens the configuration bucket, mirrors the central
/// configuration once if central sync is configured (a failure is logged and the
/// local value is used), loads the configuration, initializes LJM, and then spawns
/// [`run_sampler`], [`watch_kv_config`] and, if configured, [`run_central_kv_sync`].
/// It waits for the sampler to stop or for Ctrl+C, sets the shutdown flag, and gives
/// the tasks 300 ms to stop before returning.
///
/// # Errors
///
/// Returns an error if the local credentials cannot be loaded, `NATS_SERVERS` is
/// invalid, the local connection fails, the KV bucket cannot be opened or created,
/// the central server list is invalid, the configuration key is missing or invalid,
/// LJM initialization fails, the sampler task panics, or the Ctrl+C handler cannot
/// be installed. Stopping after repeated sampler failures is not an error.
async fn main() -> Result<(), LJMError> {
    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
    let sample_opts = ConnectOptions::with_credentials_file(creds_path.clone())
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to load creds: {}", e)))?;

    let sample_servers = nats_config::servers_from_env().map_err(LJMError::LibraryError)?;

    let sample_nc = sample_opts
        .connect(sample_servers)
        .await
        .map_err(|e| LJMError::LibraryError(format!("NATS connect failed: {}", e)))?;

    println!("Connected to sample NATS via creds!");
    let sample_js = nats_config::jetstream_context(sample_nc);

    let bucket = std::env::var("CFG_BUCKET").unwrap_or_else(|_| "avenabox".into());
    let key = std::env::var("CFG_KEY")
        .unwrap_or_else(|_| "unknown-site.macbook.unknown-source.config".into());

    let store = ensure_kv_bucket(&sample_js, &bucket).await?;
    let central_sync_cfg = central_kv_sync_config_from_env()?;
    if let Some(sync_cfg) = central_sync_cfg.as_ref() {
        if let Err(err) = mirror_central_kv_once(sync_cfg, &store, &key).await {
            eprintln!("[bootstrap] Central-to-local KV mirror failed: {:?}", err);
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

    let sampler_handle = tokio::spawn(run_sampler(
        config_rx.clone(),
        shutdown_rx.clone(),
        sample_js.clone(),
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

    let sampler_stopped_after_failures = tokio::select! {
        result = sampler_handle => {
            match result {
                Ok(stopped_after_failures) => stopped_after_failures,
                Err(err) => {
                    return Err(LJMError::LibraryError(format!(
                        "Sampler task failed to join: {err}"
                    )));
                }
            }
        }
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|e| LJMError::LibraryError(format!("Failed to listen for Ctrl+C: {}", e)))?;
            false
        }
    };

    if sampler_stopped_after_failures {
        eprintln!("Streamer stopped after repeated LabJack sampler failures.");
    } else {
        println!("Shutting down...");
    }
    let _ = shutdown_tx.send(true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a KV configuration document with the given scan field names and values.
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

    /// Checks that 5 kHz gives a 200,000 ns interval.
    #[test]
    fn derive_interval_from_actual_rate() {
        let interval = derive_sample_interval_ns(5_000.0).expect("valid rate");
        assert_eq!(interval, 200_000);
    }

    /// Checks that consecutive batches are contiguous and numbered 0, 1.
    #[test]
    fn clock_advances_monotonically_after_first_batch() {
        let mut clock = StreamClock::new(200_000);
        let (first, seq0) = clock.next_batch(10).expect("first batch");
        let (second, seq1) = clock.next_batch(10).expect("second batch");
        assert_eq!(seq0, 0);
        assert_eq!(seq1, 1);
        assert_eq!(second, first + (10 * 200_000));
    }

    /// Checks that a new clock starts at sequence 0.
    #[test]
    fn clock_resets_sequence_on_new_run() {
        let mut clock = StreamClock::new(1_000);
        let _ = clock.next_batch(3).expect("first batch");
        let _ = clock.next_batch(3).expect("second batch");

        let reset_clock = StreamClock::new(1_000);
        assert_eq!(reset_clock.sequence, 0);
    }

    /// Checks that read-latency jitter under 5 ms never moves timestamps.
    #[test]
    fn clock_ignores_latency_jitter_below_threshold() {
        // 100 Hz, 100 scans per read: one batch per second.
        let mut clock = StreamClock::new(10_000_000);
        let t0 = 1_790_000_000_000_000_000_u64;
        let (first0, _) = clock.next_batch_at(100, t0).expect("first batch");
        let mut expected_first = first0;
        for k in 1..=120_u64 {
            // Each batch arrives 0-3 ms after its last sample.
            let jitter = (k % 4) * 1_000_000;
            let now = t0 + k * 1_000_000_000 + jitter;
            let (first, _) = clock.next_batch_at(100, now).expect("batch");
            expected_first += 1_000_000_000;
            assert_eq!(first, expected_first, "no correction expected at batch {k}");
        }
    }

    /// One simulated batch: the timestamp the clock gave its first sample, and the
    /// system-clock time at which that sample was really taken.
    struct SimBatch {
        stamped_ns: i128,
        true_ns: i128,
    }

    /// Runs a clock through `batches` reads and records what it stamped.
    ///
    /// The device clock runs `drift_ppm` slow, each batch arrives
    /// `latency_ns(k)` after its last sample, and the system clock jumps by
    /// `clock_step` (batch index, nanoseconds) if given.
    fn simulate(
        interval_ns: u64,
        scans: usize,
        batches: u64,
        drift_ppm: f64,
        latency_ns: impl Fn(u64) -> i128,
        clock_step: Option<(u64, i128)>,
    ) -> Vec<SimBatch> {
        let mut clock = StreamClock::new(interval_ns);
        let t0: i128 = 1_790_000_000_000_000_000;
        let real_interval = interval_ns as f64 * (1.0 + drift_ppm * 1e-6);
        let mut out = Vec::new();
        for k in 0..batches {
            let offset = match clock_step {
                Some((at, ns)) if k >= at => ns,
                _ => 0,
            };
            let first_true = t0 + (k as f64 * scans as f64 * real_interval) as i128 + offset;
            let last_true = first_true + ((scans - 1) as f64 * real_interval) as i128;
            let now = (last_true + latency_ns(k)) as u64;
            let (stamped, _) = clock.next_batch_at(scans, now).expect("batch");
            out.push(SimBatch {
                stamped_ns: stamped as i128,
                true_ns: first_true,
            });
        }
        out
    }

    /// Largest change between consecutive batch start times beyond the nominal span.
    fn largest_adjustment(batches: &[SimBatch], span_ns: i128) -> i128 {
        batches
            .windows(2)
            .map(|w| (w[1].stamped_ns - w[0].stamped_ns - span_ns).abs())
            .max()
            .unwrap_or(0)
    }

    /// Largest difference between stamped and true times after `skip` batches.
    fn largest_error(batches: &[SimBatch], skip: usize) -> i128 {
        batches[skip..]
            .iter()
            .map(|b| (b.stamped_ns - b.true_ns).abs())
            .max()
            .unwrap_or(0)
    }

    /// Checks that latency wander like MU1's under CPU load never jumps the timeline.
    #[test]
    fn clock_does_not_chase_latency_wander() {
        // 100 Hz, 100 scans per read, three hours. Latency ramps by 64 ms per
        // minute and resets every seven minutes, with a 300 ms stall every 37
        // batches: the pattern seen on MU1 while the camera loaded the CPU.
        let latency = |k: u64| -> i128 {
            let ramp = (k % 420) as i128 * 64_000_000 / 60;
            let stall = if k % 37 == 0 { 300_000_000 } else { 0 };
            2_000_000 + ramp + stall
        };
        let batches = simulate(10_000_000, 100, 3 * 3600, 0.0, latency, None);
        // The first batch arrived during a stall; the anchor is fixed once after the
        // first minute, and every later adjustment is a small slew.
        assert!(largest_adjustment(&batches[61..], 1_000_000_000) <= 1_000_000);
        // No real drift, so the timeline must stay close to the truth.
        assert!(
            largest_error(&batches, 61) < 25_000_000,
            "{}",
            largest_error(&batches, 61)
        );
    }

    /// Checks that steady crystal drift is followed by small slews.
    #[test]
    fn clock_follows_steady_drift_with_small_slews() {
        // The device runs 10 ppm slow (0.6 ms per minute, five times MU1's measured
        // drift), with 0-3 ms of latency. Uncorrected, the error would reach 108 ms
        // after three hours. The steady-state error is the 5 ms threshold plus the
        // drift over the 15-minute history (9 ms) plus latency.
        let batches = simulate(
            10_000_000,
            100,
            3 * 3600,
            10.0,
            |k| (k % 4) as i128 * 1_000_000,
            None,
        );
        assert!(largest_adjustment(&batches, 1_000_000_000) <= 1_000_000 + 10_000);
        assert!(
            largest_error(&batches, 1200) < 20_000_000,
            "{}",
            largest_error(&batches, 1200)
        );

        // At MU1's measured 2 ppm the error stays within a few milliseconds.
        let batches = simulate(
            10_000_000,
            100,
            6 * 3600,
            2.0,
            |k| (k % 4) as i128 * 1_000_000,
            None,
        );
        assert!(
            largest_error(&batches, 0) < 10_000_000,
            "{}",
            largest_error(&batches, 0)
        );
    }

    /// Checks that at 2 kHz timestamps stay strictly increasing through slews.
    #[test]
    fn clock_keeps_timestamps_increasing_at_high_rates() {
        // 2 kHz, 100 scans per read (50 ms batches), 5 ppm drift, jittery latency.
        let latency = |k: u64| -> i128 { ((k * 7919) % 40) as i128 * 1_000_000 };
        let batches = simulate(500_000, 100, 20 * 60 * 30, 5.0, latency, None);
        let span = 100 * 500_000;
        assert!(largest_adjustment(&batches, span) <= 250_000 + 10_000);
        // The last sample of each batch comes before the first of the next.
        for w in batches.windows(2) {
            assert!(w[1].stamped_ns > w[0].stamped_ns + 99 * 500_000);
        }
    }

    /// Checks that a persistent system clock step moves the timeline in one jump.
    #[test]
    fn clock_steps_after_a_persistent_system_clock_jump() {
        // The system clock jumps forward 3 s ten minutes in.
        let batches = simulate(
            10_000_000,
            100,
            30 * 60,
            0.0,
            |_| 2_000_000,
            Some((600, 3_000_000_000)),
        );
        let jumps: Vec<(usize, i128)> = batches
            .windows(2)
            .enumerate()
            .map(|(i, w)| (i + 1, w[1].stamped_ns - w[0].stamped_ns - 1_000_000_000))
            .filter(|(_, d)| d.abs() > 1_000_000)
            .collect();
        assert_eq!(jumps.len(), 1, "exactly one jump: {jumps:?}");
        let (at, size) = jumps[0];
        assert!(
            (600..600 + 5 * 60).contains(&at),
            "jump within five windows, at {at}"
        );
        assert!(
            (size - 3_000_000_000).abs() < 10_000_000,
            "jump of about 3 s: {size}"
        );
        assert!(largest_error(&batches, at + 1) < 10_000_000);
    }

    /// Checks the slew and step decisions on single inputs.
    #[test]
    fn adjustment_decisions() {
        let ms = 1_000_000_i128;
        assert_eq!(
            decide_adjustment(64 * ms, Some(60 * ms), 0, ms),
            ClockAdjustment::Slew(ms)
        );
        assert_eq!(
            decide_adjustment(-64 * ms, Some(-9 * ms), 0, ms),
            ClockAdjustment::Slew(-ms)
        );
        assert_eq!(
            decide_adjustment(64 * ms, Some(-400 * ms), 0, ms),
            ClockAdjustment::None
        );
        assert_eq!(
            decide_adjustment(64 * ms, None, 0, ms),
            ClockAdjustment::None
        );
        assert_eq!(
            decide_adjustment(4 * ms, Some(4 * ms), 0, ms),
            ClockAdjustment::None
        );
        assert_eq!(
            decide_adjustment(3 * ms / 2 * 4, Some(8 * ms), 0, ms / 4),
            ClockAdjustment::Slew(ms / 4)
        );
        assert_eq!(
            decide_adjustment(3000 * ms, Some(3000 * ms), 2, ms),
            ClockAdjustment::None
        );
        assert_eq!(
            decide_adjustment(3000 * ms, Some(3000 * ms), 3, ms),
            ClockAdjustment::Step(3000 * ms)
        );
    }

    /// Checks that LJM dummy values become NaN and are counted.
    #[test]
    fn dummy_samples_become_nan_and_are_counted() {
        let mut batch = vec![3.72, LJM_DUMMY_VALUE, 3.71, LJM_DUMMY_VALUE];
        assert_eq!(replace_dummy_samples(&mut batch), 2);
        assert_eq!(batch[0], 3.72);
        assert!(batch[1].is_nan());
        assert_eq!(batch[2], 3.71);
        assert!(batch[3].is_nan());
        let mut clean = vec![-3.25, 0.0];
        assert_eq!(replace_dummy_samples(&mut clean), 0);
    }

    /// Checks that an empty batch is an error.
    #[test]
    fn clock_rejects_empty_batch() {
        let mut clock = StreamClock::new(1_000);
        assert!(clock.next_batch(0).is_err());
    }

    /// Checks parsing with `scans_per_read` and `scan_rate_hz`.
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

    /// Checks parsing with the legacy `scan_rate` and `sampling_rate` names.
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

    /// Checks that `labjack_on_off` is carried into the runtime config.
    #[test]
    fn kv_config_preserves_labjack_enabled_state() {
        let config = sample_config_from_json(
            sample_kv_json("scans_per_read", "200", "scan_rate_hz", "5000").as_bytes(),
        )
        .expect("config should parse");

        assert!(config.labjack_on_off);
    }
}
