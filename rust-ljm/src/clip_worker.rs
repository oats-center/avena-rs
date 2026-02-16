use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use async_nats::{
    ConnectOptions, ServerAddr,
    jetstream::{
        self,
        consumer::{AckPolicy, pull},
        kv::{self, Store},
        object_store::{self, ObjectStore},
        stream,
    },
};
use chrono::{DateTime, Duration as ChronoDuration, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Debug, Clone)]
struct WorkerConfig {
    nats_servers: Vec<ServerAddr>,
    nats_creds_file: String,
    video_bucket: String,
    video_tz: Tz,
    ffmpeg_bin: String,
    video_tmp_dir: PathBuf,
    trigger_stream: String,
    trigger_subject_filter: String,
    trigger_consumer_durable: String,
    trigger_state_bucket: String,
    compaction_interval_sec: u64,
    poll_interval_sec: u64,
    clip_pre_sec: f64,
    clip_post_sec: f64,
    clip_process_lag_sec: u64,
    clip_camera_ids: Vec<String>,
    clip_output_prefix: String,
    raw_retention_sec: u64,
}

impl WorkerConfig {
    fn from_env() -> Result<Self> {
        let nats_servers = parse_nats_servers_from_env()?;
        let nats_creds_file =
            std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".to_string());
        let nats_subject = std::env::var("NATS_SUBJECT").unwrap_or_else(|_| "avenabox".to_string());
        let video_bucket =
            std::env::var("VIDEO_BUCKET").unwrap_or_else(|_| "avena_videos".to_string());
        let video_tz_raw =
            std::env::var("VIDEO_TZ").unwrap_or_else(|_| "America/New_York".to_string());
        let video_tz = video_tz_raw
            .parse::<Tz>()
            .map_err(|_| anyhow!("invalid VIDEO_TZ '{}'", video_tz_raw))?;
        let ffmpeg_bin = std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".to_string());
        let video_tmp_dir = PathBuf::from(
            std::env::var("VIDEO_TMP_DIR").unwrap_or_else(|_| "/tmp/avena-video".to_string()),
        );
        let trigger_stream =
            std::env::var("TRIGGER_STREAM").unwrap_or_else(|_| "labjack_triggers".to_string());
        let trigger_subject_filter = std::env::var("TRIGGER_SUBJECT_FILTER")
            .unwrap_or_else(|_| format!("{}.*.trigger.*", nats_subject));
        let trigger_consumer_durable =
            std::env::var("TRIGGER_CONSUMER_DURABLE").unwrap_or_else(|_| "clip_worker".to_string());
        let trigger_state_bucket = std::env::var("TRIGGER_STATE_BUCKET")
            .unwrap_or_else(|_| "video_trigger_events".to_string());
        let compaction_interval_sec = parse_u64_env("CLIP_COMPACTION_INTERVAL_SEC", 3600)?;
        let poll_interval_sec = parse_u64_env("CLIP_WORKER_POLL_INTERVAL_SEC", 2)?;
        let clip_pre_sec = parse_f64_env("CLIP_PRE_SEC", 5.0)?;
        let clip_post_sec = parse_f64_env("CLIP_POST_SEC", 5.0)?;
        let clip_process_lag_sec = parse_u64_env("CLIP_PROCESS_LAG_SEC", 15)?;
        let clip_camera_ids = parse_csv_env("CLIP_CAMERA_IDS");
        let clip_output_prefix =
            std::env::var("CLIP_OUTPUT_PREFIX").unwrap_or_else(|_| "clips".to_string());
        let raw_retention_sec = parse_u64_env("RAW_RETENTION_SEC", 172800)?;

        Ok(Self {
            nats_servers,
            nats_creds_file,
            video_bucket,
            video_tz,
            ffmpeg_bin,
            video_tmp_dir,
            trigger_stream,
            trigger_subject_filter,
            trigger_consumer_durable,
            trigger_state_bucket,
            compaction_interval_sec,
            poll_interval_sec,
            clip_pre_sec,
            clip_post_sec,
            clip_process_lag_sec,
            clip_camera_ids,
            clip_output_prefix,
            raw_retention_sec,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum TriggerType {
    Rising,
    Falling,
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
#[serde(rename_all = "snake_case")]
enum TriggerRecordStatus {
    Pending,
    Processed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TriggerRecord {
    id: String,
    event: TriggerEvent,
    status: TriggerRecordStatus,
    attempts: u32,
    clip_keys: Vec<String>,
    last_error: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct VideoObject {
    name: String,
    camera_id: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

enum ClipSourcePlan {
    Single(VideoObject),
    Stitch(VideoObject, VideoObject),
}

fn parse_u64_env(name: &str, default: u64) -> Result<u64> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<u64>()
        .map_err(|e| anyhow!("invalid {} '{}': {}", name, raw, e))
}

fn parse_f64_env(name: &str, default: f64) -> Result<f64> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<f64>()
        .map_err(|e| anyhow!("invalid {} '{}': {}", name, raw, e))
}

fn parse_csv_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn parse_nats_servers_from_env() -> Result<Vec<ServerAddr>> {
    let raw = std::env::var("NATS_SERVERS")
        .map_err(|_| anyhow!("NATS_SERVERS must be set (comma-separated nats:// URLs)"))?;
    let servers = raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<ServerAddr>()
                .map_err(|e| anyhow!("invalid NATS server '{}': {}", part, e))
        })
        .collect::<Result<Vec<_>>>()?;

    if servers.is_empty() {
        return Err(anyhow!("NATS_SERVERS resolved to an empty list"));
    }
    Ok(servers)
}

fn trigger_event_id(event: &TriggerEvent) -> String {
    let trigger_kind = match event.trigger_type {
        TriggerType::Rising => "r",
        TriggerType::Falling => "f",
    };
    format!(
        "{}_{}_{}_{}",
        event.asset, event.channel, event.trigger_time_unix_ms, trigger_kind
    )
}

fn trigger_record_key(id: &str) -> String {
    format!("event.{}", id)
}

async fn get_or_create_kv(js: &jetstream::Context, bucket: &str) -> Result<Store> {
    if let Ok(store) = js.get_key_value(bucket).await {
        return Ok(store);
    }
    js.create_key_value(kv::Config {
        bucket: bucket.to_string(),
        history: 10,
        ..Default::default()
    })
    .await
    .with_context(|| format!("failed to create KV bucket '{}'", bucket))
}

async fn get_or_create_object_store(js: &jetstream::Context, bucket: &str) -> Result<ObjectStore> {
    if let Ok(store) = js.get_object_store(bucket).await {
        return Ok(store);
    }
    js.create_object_store(object_store::Config {
        bucket: bucket.to_string(),
        ..Default::default()
    })
    .await
    .with_context(|| format!("failed to open or create VIDEO_BUCKET '{}'", bucket))
}

async fn get_or_create_trigger_stream(
    js: &jetstream::Context,
    cfg: &WorkerConfig,
) -> Result<jetstream::stream::Stream> {
    if let Ok(stream) = js.get_stream(cfg.trigger_stream.clone()).await {
        return Ok(stream);
    }

    js.create_stream(stream::Config {
        name: cfg.trigger_stream.clone(),
        subjects: vec![cfg.trigger_subject_filter.clone()],
        storage: stream::StorageType::File,
        retention: stream::RetentionPolicy::Limits,
        ..Default::default()
    })
    .await
    .with_context(|| {
        format!(
            "failed to open or create trigger stream '{}' with subject '{}'",
            cfg.trigger_stream, cfg.trigger_subject_filter
        )
    })
}

async fn connect_jetstream(cfg: &WorkerConfig) -> Result<jetstream::Context> {
    let opts = ConnectOptions::with_credentials_file(cfg.nats_creds_file.clone())
        .await
        .context("failed to load NATS creds")?
        .request_timeout(None);

    let nc = opts
        .connect(cfg.nats_servers.clone())
        .await
        .context("NATS connect failed")?;

    Ok(jetstream::new(nc))
}

async fn init_trigger_consumer(
    js: &jetstream::Context,
    cfg: &WorkerConfig,
) -> Result<jetstream::consumer::Consumer<pull::Config>> {
    let stream = get_or_create_trigger_stream(js, cfg).await?;

    stream
        .get_or_create_consumer(
            cfg.trigger_consumer_durable.as_str(),
            pull::Config {
                durable_name: Some(cfg.trigger_consumer_durable.clone()),
                ack_policy: AckPolicy::Explicit,
                filter_subject: cfg.trigger_subject_filter.clone(),
                ..Default::default()
            },
        )
        .await
        .with_context(|| {
            format!(
                "failed to create/get trigger consumer '{}'",
                cfg.trigger_consumer_durable
            )
        })
}

async fn ingest_new_triggers(
    consumer: &jetstream::consumer::Consumer<pull::Config>,
    state_store: &Store,
) -> Result<usize> {
    let mut ingested = 0usize;
    let mut messages = consumer
        .fetch()
        .max_messages(200)
        .expires(Duration::from_secs(1))
        .messages()
        .await
        .context("failed to fetch trigger messages")?;

    while let Some(message) = messages.next().await {
        let message = message.map_err(|err| anyhow!("failed reading trigger message: {err}"))?;
        let payload: TriggerEvent = serde_json::from_slice(message.payload.as_ref())
            .context("failed to parse trigger event payload")?;
        let id = trigger_event_id(&payload);
        let key = trigger_record_key(&id);

        if state_store
            .entry(key.clone())
            .await
            .with_context(|| format!("failed reading state key {}", key))?
            .is_none()
        {
            let record = TriggerRecord {
                id: id.clone(),
                event: payload,
                status: TriggerRecordStatus::Pending,
                attempts: 0,
                clip_keys: Vec::new(),
                last_error: None,
                updated_at: Utc::now().to_rfc3339(),
            };
            let record_bytes = serde_json::to_vec(&record).context("failed to encode record")?;
            state_store
                .put(key.clone(), record_bytes.into())
                .await
                .with_context(|| format!("failed writing state key {}", key))?;
            ingested += 1;
        }

        message
            .ack()
            .await
            .map_err(|err| anyhow!("failed to ack trigger message: {err}"))?;
    }

    Ok(ingested)
}

async fn load_trigger_records(state_store: &Store) -> Result<Vec<TriggerRecord>> {
    let mut out = Vec::new();
    let mut keys = state_store
        .keys()
        .await
        .context("failed listing trigger state keys")?;
    while let Some(key_result) = keys.next().await {
        let key = key_result.map_err(|err| anyhow!("failed reading trigger state key: {err}"))?;
        if !key.starts_with("event.") {
            continue;
        }
        let Some(entry) = state_store
            .entry(key.clone())
            .await
            .with_context(|| format!("failed reading trigger state entry {}", key))?
        else {
            continue;
        };
        let record: TriggerRecord = serde_json::from_slice(entry.value.as_ref())
            .with_context(|| format!("failed parsing trigger state {}", key))?;
        out.push(record);
    }

    Ok(out)
}

async fn save_trigger_record(state_store: &Store, record: &TriggerRecord) -> Result<()> {
    let key = trigger_record_key(&record.id);
    let bytes = serde_json::to_vec(record).context("failed to encode trigger record")?;
    state_store
        .put(key.clone(), bytes.into())
        .await
        .with_context(|| format!("failed writing trigger state {}", key))?;
    Ok(())
}

async fn list_video_objects(store: &ObjectStore, tz: Tz) -> Result<Vec<VideoObject>> {
    let mut list = store.list().await.context("failed to list objects")?;
    let mut out = Vec::new();
    while let Some(item) = list.next().await {
        let info = item.map_err(|err| anyhow!("failed to read object metadata: {err}"))?;
        if let Some((_, camera_id, start, end)) = parse_video_key_interval(&info.name, tz) {
            out.push(VideoObject {
                name: info.name,
                camera_id,
                start,
                end,
            });
        }
    }
    out.sort_by_key(|obj| obj.start);
    Ok(out)
}

fn parse_video_key_interval(
    name: &str,
    tz: Tz,
) -> Option<(u32, String, DateTime<Utc>, DateTime<Utc>)> {
    let parts: Vec<&str> = name.split('/').collect();
    let (asset_prefix, camera_id, file_name) = match parts.as_slice() {
        [asset, file] => (*asset, "default".to_string(), *file),
        [asset, camera_prefix, file] => {
            let camera = camera_prefix
                .strip_prefix("camera_")
                .unwrap_or(camera_prefix)
                .to_string();
            (*asset, camera, *file)
        }
        _ => return None,
    };

    let asset_str = asset_prefix.strip_prefix("asset")?;
    let asset: u32 = asset_str.parse().ok()?;
    let stem = file_name.strip_suffix(".mp4")?;
    let segments: Vec<&str> = stem.split('_').collect();
    if segments.len() != 9 || segments[0] != "V" {
        return None;
    }
    let start_raw = format!(
        "{}_{}_{}_{}",
        segments[1], segments[2], segments[3], segments[4]
    );
    let end_raw = format!(
        "{}_{}_{}_{}",
        segments[5], segments[6], segments[7], segments[8]
    );
    let start_naive = NaiveDateTime::parse_from_str(&start_raw, "%Y_%m_%d_%H%M%S").ok()?;
    let end_naive = NaiveDateTime::parse_from_str(&end_raw, "%Y_%m_%d_%H%M%S").ok()?;
    let start = local_to_utc(tz, start_naive)?;
    let end = local_to_utc(tz, end_naive)?;
    if end <= start {
        return None;
    }
    Some((asset, camera_id, start, end))
}

fn local_to_utc(tz: Tz, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

fn resolve_clip_sources(
    objects: &[VideoObject],
    clip_start: DateTime<Utc>,
    clip_end: DateTime<Utc>,
) -> std::result::Result<ClipSourcePlan, &'static str> {
    let grace = ChronoDuration::seconds(1);
    for object in objects {
        if object.start - grace <= clip_start && object.end + grace >= clip_end {
            return Ok(ClipSourcePlan::Single(object.clone()));
        }
    }

    let left = objects
        .iter()
        .rev()
        .find(|entry| entry.start - grace <= clip_start && entry.end + grace >= clip_start)
        .cloned();
    let right = objects
        .iter()
        .find(|entry| entry.start - grace <= clip_end && entry.end + grace >= clip_end)
        .cloned();

    let (Some(left), Some(right)) = (left, right) else {
        return Err("no source object covers requested interval");
    };

    if left.name == right.name {
        return Ok(ClipSourcePlan::Single(left));
    }

    if right.start > left.end + grace {
        return Err("requested interval spans a gap between video objects");
    }

    if left.start - grace <= clip_start && right.end + grace >= clip_end {
        return Ok(ClipSourcePlan::Stitch(left, right));
    }

    Err("requested interval is not fully covered by available objects")
}

async fn download_object_to_file(
    store: &ObjectStore,
    object_name: &str,
    target: &Path,
) -> Result<()> {
    let mut object = store
        .get(object_name)
        .await
        .with_context(|| format!("failed to get object '{}'", object_name))?;

    let mut file = tokio::fs::File::create(target)
        .await
        .with_context(|| format!("failed to create file {}", target.display()))?;

    tokio::io::copy(&mut object, &mut file)
        .await
        .with_context(|| format!("failed downloading object '{}'", object_name))?;
    Ok(())
}

async fn run_ffmpeg_trim(
    ffmpeg_bin: &str,
    input_path: &Path,
    output_path: &Path,
    offset_sec: f64,
    duration_sec: f64,
) -> Result<()> {
    let output = Command::new(ffmpeg_bin)
        .arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input_path)
        .arg("-ss")
        .arg(format!("{offset_sec:.3}"))
        .arg("-t")
        .arg(format!("{duration_sec:.3}"))
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("0:a?")
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("23")
        .arg("-c:a")
        .arg("aac")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output_path)
        .output()
        .await
        .with_context(|| format!("failed to invoke ffmpeg '{}'", ffmpeg_bin))?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffmpeg trim failed (status {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

async fn run_ffmpeg_concat_and_trim(
    ffmpeg_bin: &str,
    left_part: &Path,
    right_part: &Path,
    output_path: &Path,
    total_duration_sec: f64,
    temp_dir: &Path,
) -> Result<()> {
    let list_path = temp_dir.join("concat-list.txt");
    let list_content = format!(
        "file '{}'\nfile '{}'\n",
        left_part.to_string_lossy().replace('\'', "'\\''"),
        right_part.to_string_lossy().replace('\'', "'\\''")
    );
    tokio::fs::write(&list_path, list_content)
        .await
        .with_context(|| format!("failed to write concat list {}", list_path.display()))?;

    let stitched_path = temp_dir.join("stitched.mp4");
    let output = Command::new(ffmpeg_bin)
        .arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&list_path)
        .arg("-c")
        .arg("copy")
        .arg(&stitched_path)
        .output()
        .await
        .with_context(|| format!("failed to invoke ffmpeg concat '{}'", ffmpeg_bin))?;
    if !output.status.success() {
        return Err(anyhow!(
            "ffmpeg concat failed (status {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    run_ffmpeg_trim(
        ffmpeg_bin,
        &stitched_path,
        output_path,
        0.0,
        total_duration_sec,
    )
    .await?;
    Ok(())
}

fn seconds_between(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<f64> {
    if end < start {
        return Err(anyhow!("negative time delta"));
    }
    Ok((end - start).num_milliseconds() as f64 / 1000.0)
}

async fn build_and_store_clip(
    store: &ObjectStore,
    cfg: &WorkerConfig,
    event: &TriggerEvent,
    camera_id: &str,
    objects: &[VideoObject],
) -> Result<String> {
    let center = DateTime::parse_from_rfc3339(&event.trigger_time)
        .map_err(|e| anyhow!("invalid trigger_time '{}': {}", event.trigger_time, e))?
        .with_timezone(&Utc);
    let clip_start = center - ChronoDuration::milliseconds((cfg.clip_pre_sec * 1000.0) as i64);
    let clip_end = center + ChronoDuration::milliseconds((cfg.clip_post_sec * 1000.0) as i64);
    let clip_duration = seconds_between(clip_start, clip_end)?;

    let plan = resolve_clip_sources(objects, clip_start, clip_end)
        .map_err(|e| anyhow!("camera {}: {}", camera_id, e))?;

    let request_dir = cfg.video_tmp_dir.join(format!(
        "clip-worker-{}-{}",
        camera_id, event.trigger_time_unix_ms
    ));
    tokio::fs::create_dir_all(&request_dir)
        .await
        .with_context(|| format!("failed to create temp dir {}", request_dir.display()))?;

    let generated = async {
        let output_path = request_dir.join("final.mp4");
        match plan {
            ClipSourcePlan::Single(source) => {
                let source_path = request_dir.join("single.mp4");
                download_object_to_file(store, &source.name, &source_path).await?;
                let offset_sec = seconds_between(source.start, clip_start)
                    .unwrap_or(0.0)
                    .max(0.0);
                run_ffmpeg_trim(
                    &cfg.ffmpeg_bin,
                    &source_path,
                    &output_path,
                    offset_sec,
                    clip_duration,
                )
                .await?;
            }
            ClipSourcePlan::Stitch(left, right) => {
                let left_source = request_dir.join("left_source.mp4");
                let right_source = request_dir.join("right_source.mp4");
                download_object_to_file(store, &left.name, &left_source).await?;
                download_object_to_file(store, &right.name, &right_source).await?;

                let left_part = request_dir.join("left_part.mp4");
                let right_part = request_dir.join("right_part.mp4");

                let left_trim_start = if clip_start > left.start {
                    clip_start
                } else {
                    left.start
                };
                let left_trim_end = if clip_end < left.end {
                    clip_end
                } else {
                    left.end
                };
                let left_offset = seconds_between(left.start, left_trim_start)
                    .unwrap_or(0.0)
                    .max(0.0);
                let left_duration = seconds_between(left_trim_start, left_trim_end)?;

                let right_trim_start = if clip_start > right.start {
                    clip_start
                } else {
                    right.start
                };
                let right_offset = seconds_between(right.start, right_trim_start)
                    .unwrap_or(0.0)
                    .max(0.0);
                let right_duration = seconds_between(right_trim_start, clip_end)?;

                run_ffmpeg_trim(
                    &cfg.ffmpeg_bin,
                    &left_source,
                    &left_part,
                    left_offset,
                    left_duration,
                )
                .await?;
                run_ffmpeg_trim(
                    &cfg.ffmpeg_bin,
                    &right_source,
                    &right_part,
                    right_offset,
                    right_duration,
                )
                .await?;
                run_ffmpeg_concat_and_trim(
                    &cfg.ffmpeg_bin,
                    &left_part,
                    &right_part,
                    &output_path,
                    clip_duration,
                    &request_dir,
                )
                .await?;
            }
        }

        let clip_key = format!(
            "{}/asset{}/camera_{}/C_{}_ch{:02}_{}.mp4",
            cfg.clip_output_prefix,
            event.asset,
            camera_id,
            center.format("%Y%m%dT%H%M%S"),
            event.channel,
            match event.trigger_type {
                TriggerType::Rising => "rising",
                TriggerType::Falling => "falling",
            }
        );

        let mut file = tokio::fs::File::open(&output_path)
            .await
            .with_context(|| format!("failed to open generated clip {}", output_path.display()))?;
        store
            .put(clip_key.as_str(), &mut file)
            .await
            .with_context(|| format!("failed to upload clip object {}", clip_key))?;

        Ok::<String, anyhow::Error>(clip_key)
    }
    .await;

    let _ = tokio::fs::remove_dir_all(&request_dir).await;
    generated
}

async fn run_compaction_cycle(
    cfg: &WorkerConfig,
    store: &ObjectStore,
    state_store: &Store,
) -> Result<()> {
    let mut records = load_trigger_records(state_store).await?;
    if records.is_empty() {
        cleanup_raw_objects(cfg, store, &records).await?;
        return Ok(());
    }

    let all_objects = list_video_objects(store, cfg.video_tz).await?;
    let by_asset: HashMap<u32, Vec<VideoObject>> =
        all_objects
            .iter()
            .cloned()
            .fold(HashMap::new(), |mut acc, obj| {
                if let Some((asset_prefix, _)) = obj.name.split_once('/') {
                    if let Some(asset_str) = asset_prefix.strip_prefix("asset") {
                        if let Ok(asset) = asset_str.parse::<u32>() {
                            acc.entry(asset).or_default().push(obj);
                        }
                    }
                }
                acc
            });

    let now = Utc::now();
    let process_before = now - ChronoDuration::seconds(cfg.clip_process_lag_sec as i64);

    for record in records.iter_mut() {
        if record.status == TriggerRecordStatus::Processed {
            continue;
        }

        let trigger_time = DateTime::parse_from_rfc3339(&record.event.trigger_time)
            .map_err(|e| {
                anyhow!(
                    "invalid trigger_time '{}': {}",
                    record.event.trigger_time,
                    e
                )
            })?
            .with_timezone(&Utc);
        if trigger_time > process_before {
            continue;
        }

        let Some(asset_objects) = by_asset.get(&record.event.asset) else {
            record.attempts += 1;
            record.last_error = Some(format!(
                "no raw video objects found for asset {}",
                record.event.asset
            ));
            record.updated_at = Utc::now().to_rfc3339();
            save_trigger_record(state_store, record).await?;
            continue;
        };

        let mut camera_set: Vec<String> = asset_objects
            .iter()
            .map(|obj| obj.camera_id.clone())
            .collect();
        camera_set.sort();
        camera_set.dedup();
        if !cfg.clip_camera_ids.is_empty() {
            camera_set.retain(|camera| cfg.clip_camera_ids.contains(camera));
        }

        if camera_set.is_empty() {
            record.attempts += 1;
            record.last_error = Some("no camera ids available for clip generation".to_string());
            record.updated_at = Utc::now().to_rfc3339();
            save_trigger_record(state_store, record).await?;
            continue;
        }

        let mut clip_keys = Vec::new();
        let mut failure: Option<String> = None;
        for camera_id in camera_set {
            let camera_objects: Vec<VideoObject> = asset_objects
                .iter()
                .filter(|obj| obj.camera_id == camera_id)
                .cloned()
                .collect();
            if camera_objects.is_empty() {
                continue;
            }
            match build_and_store_clip(store, cfg, &record.event, &camera_id, &camera_objects).await
            {
                Ok(key) => clip_keys.push(key),
                Err(err) => {
                    failure = Some(format!("camera {}: {}", camera_id, err));
                    break;
                }
            }
        }

        record.attempts += 1;
        record.updated_at = Utc::now().to_rfc3339();
        if let Some(err) = failure {
            record.last_error = Some(err);
            record.status = TriggerRecordStatus::Pending;
        } else {
            record.clip_keys = clip_keys;
            record.last_error = None;
            record.status = TriggerRecordStatus::Processed;
        }
        save_trigger_record(state_store, record).await?;
    }

    cleanup_raw_objects(cfg, store, &records).await?;
    Ok(())
}

async fn cleanup_raw_objects(
    cfg: &WorkerConfig,
    store: &ObjectStore,
    records: &[TriggerRecord],
) -> Result<()> {
    let now = Utc::now();
    let retention_cutoff = now - ChronoDuration::seconds(cfg.raw_retention_sec as i64);
    let mut safe_cutoff = retention_cutoff;

    let earliest_pending = records
        .iter()
        .filter(|record| record.status == TriggerRecordStatus::Pending)
        .filter_map(|record| DateTime::parse_from_rfc3339(&record.event.trigger_time).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .min();

    if let Some(pending_time) = earliest_pending {
        let pending_cutoff =
            pending_time - ChronoDuration::milliseconds((cfg.clip_pre_sec * 1000.0) as i64);
        if pending_cutoff < safe_cutoff {
            safe_cutoff = pending_cutoff;
        }
    }

    let objects = list_video_objects(store, cfg.video_tz).await?;
    let mut deleted = 0usize;
    for obj in objects {
        if obj.end >= safe_cutoff {
            continue;
        }
        // Only delete raw V_ objects, never generated clips
        let name_parts: Vec<&str> = obj.name.split('/').collect();
        let Some(file_name) = name_parts.last().copied() else {
            continue;
        };
        if !file_name.starts_with("V_") {
            continue;
        }
        store
            .delete(obj.name.as_str())
            .await
            .with_context(|| format!("failed deleting raw object {}", obj.name))?;
        deleted += 1;
    }

    if deleted > 0 {
        println!(
            "[clip-worker] deleted {} raw objects older than {}",
            deleted, safe_cutoff
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = WorkerConfig::from_env()?;
    let js = connect_jetstream(&cfg).await?;
    let state_store = get_or_create_kv(&js, &cfg.trigger_state_bucket).await?;
    let store = get_or_create_object_store(&js, &cfg.video_bucket).await?;
    let consumer = init_trigger_consumer(&js, &cfg).await?;

    println!(
        "[clip-worker] started trigger_stream={} durable={} bucket={} state_bucket={}",
        cfg.trigger_stream,
        cfg.trigger_consumer_durable,
        cfg.video_bucket,
        cfg.trigger_state_bucket
    );

    let mut last_compaction = Instant::now()
        .checked_sub(Duration::from_secs(cfg.compaction_interval_sec))
        .unwrap_or_else(Instant::now);
    let mut shutdown = Box::pin(tokio::signal::ctrl_c());
    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.poll_interval_sec.max(1)));

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let ingested = ingest_new_triggers(&consumer, &state_store).await?;
                if ingested > 0 {
                    println!("[clip-worker] ingested {} new trigger event(s)", ingested);
                }

                if last_compaction.elapsed() >= Duration::from_secs(cfg.compaction_interval_sec.max(1)) {
                    run_compaction_cycle(&cfg, &store, &state_store).await?;
                    last_compaction = Instant::now();
                    println!("[clip-worker] compaction cycle complete");
                }
            }
            _ = &mut shutdown => {
                println!("[clip-worker] shutdown requested");
                break;
            }
        }
    }

    Ok(())
}
