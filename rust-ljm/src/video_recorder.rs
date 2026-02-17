use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::SystemTime,
};

use anyhow::{Context, Result, anyhow};
use async_nats::{
    ConnectOptions, ServerAddr,
    jetstream::{self, object_store, object_store::ObjectStore},
};
use chrono::{DateTime, Duration as ChronoDuration, LocalResult, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use tokio::{
    fs,
    process::{Child, Command},
    time::{Duration, interval},
};

#[derive(Debug, Clone)]
struct RecorderConfig {
    nats_servers: Vec<ServerAddr>,
    nats_creds_file: String,
    video_bucket: String,
    video_asset_number: u32,
    video_camera_id: String,
    video_tz: Tz,
    ffmpeg_bin: String,
    ffprobe_bin: String,
    video_source_url: String,
    video_rtsp_transport: String,
    video_segment_sec: u64,
    video_upload_settle_sec: u64,
    video_scan_interval_sec: u64,
    video_js_timeout_sec: u64,
    video_object_chunk_size_bytes: usize,
    video_spool_dir: PathBuf,
}

impl RecorderConfig {
    fn from_env() -> Result<Self> {
        let nats_servers_raw = std::env::var("NATS_SERVERS")
            .map_err(|_| anyhow!("NATS_SERVERS must be set (comma-separated nats:// URLs)"))?;
        let nats_servers = nats_servers_raw
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| {
                part.parse::<ServerAddr>()
                    .map_err(|e| anyhow!("invalid NATS server '{}': {}", part, e))
            })
            .collect::<Result<Vec<_>>>()?;
        if nats_servers.is_empty() {
            return Err(anyhow!("NATS_SERVERS resolved to an empty list"));
        }

        let nats_creds_file =
            std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".to_string());
        let video_bucket =
            std::env::var("VIDEO_BUCKET").unwrap_or_else(|_| "avena_videos".to_string());

        let video_asset_number = parse_u32_env(
            "VIDEO_ASSET_NUMBER",
            std::env::var("ASSET_NUMBER")
                .unwrap_or_else(|_| "1001".to_string())
                .as_str(),
        )?;
        let video_camera_id = sanitize_camera_id(
            &std::env::var("VIDEO_CAMERA_ID")
                .or_else(|_| std::env::var("INSTANCE"))
                .unwrap_or_else(|_| "default".to_string()),
        );

        let video_tz_raw =
            std::env::var("VIDEO_TZ").unwrap_or_else(|_| "America/New_York".to_string());
        let video_tz = video_tz_raw
            .parse::<Tz>()
            .map_err(|_| anyhow!("invalid VIDEO_TZ '{}'", video_tz_raw))?;

        let ffmpeg_bin = std::env::var("VIDEO_RECORDER_FFMPEG_BIN")
            .or_else(|_| std::env::var("FFMPEG_BIN"))
            .unwrap_or_else(|_| "ffmpeg".to_string());
        let ffprobe_bin = std::env::var("VIDEO_RECORDER_FFPROBE_BIN")
            .or_else(|_| std::env::var("FFPROBE_BIN"))
            .unwrap_or_else(|_| "ffprobe".to_string());

        let video_source_url = std::env::var("VIDEO_SOURCE_URL")
            .map_err(|_| anyhow!("VIDEO_SOURCE_URL must be set"))?;
        let video_rtsp_transport =
            std::env::var("VIDEO_RTSP_TRANSPORT").unwrap_or_else(|_| "tcp".to_string());

        let video_segment_sec = parse_u64_env("VIDEO_SEGMENT_SEC", "5")?;
        let video_upload_settle_sec = parse_u64_env("VIDEO_UPLOAD_SETTLE_SEC", "2")?;
        let video_scan_interval_sec = parse_u64_env("VIDEO_SCAN_INTERVAL_SEC", "2")?;
        let video_js_timeout_sec = parse_u64_env("VIDEO_JS_TIMEOUT_SEC", "20")?;
        let video_object_chunk_size_bytes =
            parse_usize_env("VIDEO_OBJECT_CHUNK_SIZE_BYTES", "262144")?;
        let video_spool_dir = PathBuf::from(
            std::env::var("VIDEO_SPOOL_DIR")
                .unwrap_or_else(|_| "/tmp/avena-video-recorder".to_string()),
        );

        Ok(Self {
            nats_servers,
            nats_creds_file,
            video_bucket,
            video_asset_number,
            video_camera_id,
            video_tz,
            ffmpeg_bin,
            ffprobe_bin,
            video_source_url,
            video_rtsp_transport,
            video_segment_sec,
            video_upload_settle_sec,
            video_scan_interval_sec,
            video_js_timeout_sec,
            video_object_chunk_size_bytes,
            video_spool_dir,
        })
    }

    fn segment_pattern(&self) -> PathBuf {
        self.video_spool_dir.join("segment_%Y%m%d_%H%M%S.mp4")
    }
}

fn parse_u64_env(name: &str, default: &str) -> Result<u64> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<u64>()
        .map_err(|e| anyhow!("invalid {} '{}': {}", name, raw, e))
}

fn parse_u32_env(name: &str, default: &str) -> Result<u32> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<u32>()
        .map_err(|e| anyhow!("invalid {} '{}': {}", name, raw, e))
}

fn parse_usize_env(name: &str, default: &str) -> Result<usize> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<usize>()
        .map_err(|e| anyhow!("invalid {} '{}': {}", name, raw, e))
}

fn sanitize_camera_id(raw: &str) -> String {
    let filtered: String = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if filtered.is_empty() {
        "default".to_string()
    } else {
        filtered
    }
}

async fn connect_jetstream(cfg: &RecorderConfig) -> Result<jetstream::Context> {
    let opts = ConnectOptions::with_credentials_file(cfg.nats_creds_file.clone())
        .await
        .context("failed to load NATS credentials")?
        .request_timeout(None);

    let client = opts
        .connect(cfg.nats_servers.clone())
        .await
        .context("failed to connect to NATS")?;

    let mut js = jetstream::new(client);
    js.set_timeout(Duration::from_secs(cfg.video_js_timeout_sec.max(1)));
    Ok(js)
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

async fn spawn_ffmpeg_segmenter(cfg: &RecorderConfig) -> Result<Child> {
    fs::create_dir_all(&cfg.video_spool_dir)
        .await
        .with_context(|| {
            format!(
                "failed to create spool dir {}",
                cfg.video_spool_dir.display()
            )
        })?;

    let segment_pattern = cfg.segment_pattern();
    println!(
        "[video-recorder] starting ffmpeg source={} segment={}s pattern={}",
        cfg.video_source_url,
        cfg.video_segment_sec,
        segment_pattern.display()
    );

    let mut command = Command::new(&cfg.ffmpeg_bin);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("warning")
        .arg("-rtsp_transport")
        .arg(&cfg.video_rtsp_transport)
        .arg("-i")
        .arg(&cfg.video_source_url)
        .arg("-map")
        .arg("0")
        .arg("-c")
        .arg("copy")
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(cfg.video_segment_sec.to_string())
        .arg("-reset_timestamps")
        .arg("1")
        .arg("-strftime")
        .arg("1")
        .arg(segment_pattern.as_os_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    command.spawn().with_context(|| {
        format!(
            "failed to start ffmpeg binary '{}'; verify VIDEO_RECORDER_FFMPEG_BIN/FFMPEG_BIN",
            cfg.ffmpeg_bin
        )
    })
}

fn parse_segment_start_from_path(path: &Path, tz: Tz) -> Option<DateTime<Tz>> {
    let name = path.file_name()?.to_string_lossy();
    let ts = name
        .strip_prefix("segment_")?
        .strip_suffix(".mp4")
        .and_then(|raw| NaiveDateTime::parse_from_str(raw, "%Y%m%d_%H%M%S").ok())?;

    match tz.from_local_datetime(&ts) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
        LocalResult::None => None,
    }
}

fn format_key_timestamp(dt: DateTime<Tz>) -> String {
    dt.format("%Y_%m_%d_%H%M%S").to_string()
}

async fn list_ready_segments(cfg: &RecorderConfig) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(&cfg.video_spool_dir)
        .await
        .with_context(|| format!("failed reading {}", cfg.video_spool_dir.display()))?;

    let now = SystemTime::now();
    let mut paths: Vec<PathBuf> = Vec::new();
    let min_settle_sec = cfg.video_segment_sec.saturating_add(1);
    let settle_sec = cfg.video_upload_settle_sec.max(min_settle_sec);
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("mp4") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if !file_name.starts_with("segment_") {
            continue;
        }

        let metadata = entry.metadata().await?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let age_sec = now.duration_since(modified).unwrap_or_default().as_secs();
        if age_sec < settle_sec {
            continue;
        }

        paths.push(path);
    }

    paths.sort();
    Ok(paths)
}

async fn validate_segment_file(ffprobe_bin: &str, path: &Path) -> bool {
    let output = match Command::new(ffprobe_bin)
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .output()
        .await
    {
        Ok(out) => out,
        Err(err) => {
            eprintln!(
                "[video-recorder] ffprobe launch failed for {}: {}",
                path.display(),
                err
            );
            return false;
        }
    };

    if !output.status.success() {
        return false;
    }

    let duration_raw = String::from_utf8_lossy(&output.stdout);
    let duration_sec = duration_raw.trim().parse::<f64>().ok();
    matches!(duration_sec, Some(v) if v.is_finite() && v > 0.0)
}

async fn upload_ready_segments(store: &ObjectStore, cfg: &RecorderConfig) -> Result<()> {
    let ready_paths = list_ready_segments(cfg).await?;
    for path in ready_paths {
        let metadata = fs::metadata(&path).await.ok();
        let now = SystemTime::now();
        let age_sec = metadata
            .and_then(|m| m.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let valid = validate_segment_file(&cfg.ffprobe_bin, &path).await;
        if !valid {
            if age_sec > cfg.video_segment_sec.saturating_mul(6) {
                let _ = fs::remove_file(&path).await;
                eprintln!(
                    "[video-recorder] dropped invalid stale segment {}",
                    path.display()
                );
            } else {
                eprintln!(
                    "[video-recorder] segment not finalized/invalid yet, will retry: {}",
                    path.display()
                );
            }
            continue;
        }

        let Some(start_local) = parse_segment_start_from_path(&path, cfg.video_tz) else {
            eprintln!(
                "[video-recorder] skipping unrecognized segment filename {}",
                path.display()
            );
            continue;
        };
        let end_local = start_local + ChronoDuration::seconds(cfg.video_segment_sec as i64);
        let object_key = format!(
            "asset{}/camera_{}/V_{}_{}.mp4",
            cfg.video_asset_number,
            cfg.video_camera_id,
            format_key_timestamp(start_local),
            format_key_timestamp(end_local)
        );

        let mut file = fs::File::open(&path)
            .await
            .with_context(|| format!("failed to open segment {}", path.display()))?;

        let upload_meta = object_store::ObjectMetadata {
            name: object_key.clone(),
            chunk_size: Some(cfg.video_object_chunk_size_bytes),
            ..Default::default()
        };
        if let Err(err) = store.put(upload_meta, &mut file).await {
            eprintln!(
                "[video-recorder] upload failed (will retry): key={} error={}",
                object_key, err
            );
            continue;
        }

        fs::remove_file(&path)
            .await
            .with_context(|| format!("failed to remove uploaded segment {}", path.display()))?;

        println!(
            "[video-recorder] uploaded {} -> {}",
            path.display(),
            object_key
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = RecorderConfig::from_env()?;
    let js = connect_jetstream(&cfg).await?;
    let store = get_or_create_object_store(&js, &cfg.video_bucket).await?;
    let mut ffmpeg = spawn_ffmpeg_segmenter(&cfg).await?;

    let mut ticker = interval(Duration::from_secs(cfg.video_scan_interval_sec.max(1)));
    let mut shutdown = Box::pin(tokio::signal::ctrl_c());

    println!(
        "[video-recorder] running asset={} camera_id={} bucket='{}' tz='{}' spool='{}' source={} ffprobe='{}'",
        cfg.video_asset_number,
        cfg.video_camera_id,
        cfg.video_bucket,
        cfg.video_tz,
        cfg.video_spool_dir.display(),
        cfg.video_source_url,
        cfg.ffprobe_bin
    );

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                upload_ready_segments(&store, &cfg).await?;

                if let Some(status) = ffmpeg
                    .try_wait()
                    .with_context(|| "failed to query ffmpeg process status")?
                {
                    return Err(anyhow!("ffmpeg exited unexpectedly: {}", status));
                }
            }
            _ = &mut shutdown => {
                println!("[video-recorder] shutdown requested");
                break;
            }
        }
    }

    if ffmpeg.id().is_some() {
        let _ = ffmpeg.start_kill();
        let _ = ffmpeg.wait().await;
    }

    upload_ready_segments(&store, &cfg).await?;
    println!("[video-recorder] stopped");
    Ok(())
}
