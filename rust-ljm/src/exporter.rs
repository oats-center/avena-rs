use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use async_nats::{
    ConnectOptions, ServerAddr,
    jetstream::{self, object_store, object_store::ObjectStore},
};
use axum::{
    Router,
    body::Body,
    extract::{
        Json, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{
    DateTime, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc,
};
use chrono_tz::Tz;
use futures_util::StreamExt;
use parquet::{
    file::reader::{FileReader, SerializedFileReader},
    record::RowAccessor,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;
use uuid::Uuid;

mod calibration;

use calibration::CalibrationSpec;

#[derive(Clone)]
struct AppState {
    parquet_root: Arc<PathBuf>,
    video: Arc<VideoState>,
}

#[derive(Clone)]
struct VideoState {
    jetstream: jetstream::Context,
    video_bucket: String,
    video_tz: Tz,
    ffmpeg_bin: String,
    video_tmp_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExportFormat {
    Csv,
    Parquet,
}

#[derive(Debug, Deserialize)]
struct ExportRequest {
    asset: u32,
    channels: Vec<u8>,
    start: String,
    end: String,
    #[serde(default = "default_format")]
    format: ExportFormat,
    download_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VideoClipRequest {
    asset: u32,
    camera_id: Option<String>,
    center_time: String,
    pre_sec: Option<f64>,
    post_sec: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct VideoCameraListRequest {
    asset: u32,
}

#[derive(Debug, Serialize)]
struct ErrorPayload {
    error: String,
}

#[derive(Debug, Serialize)]
struct VideoCameraListResponse {
    asset: u32,
    cameras: Vec<String>,
    default_clip_pre_sec: f64,
    default_clip_post_sec: f64,
    coverage: Vec<VideoCameraCoverage>,
}

#[derive(Debug, Serialize)]
struct VideoCameraCoverage {
    camera_id: String,
    latest_start: String,
    latest_end: String,
    recommended_center_min: String,
    recommended_center_max: String,
    contiguous_start: String,
    contiguous_end: String,
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
    Multi(Vec<VideoObject>),
}

fn default_format() -> ExportFormat {
    ExportFormat::Csv
}

#[tokio::main]
async fn main() -> Result<()> {
    let listen_addr = std::env::var("EXPORTER_ADDR").unwrap_or_else(|_| "0.0.0.0:9001".into());
    let parquet_root = std::env::var("PARQUET_DIR").unwrap_or_else(|_| "parquet".into());
    let root_path = PathBuf::from(parquet_root);
    if !root_path.exists() {
        println!(
            "[exporter] Warning: parquet directory '{}' does not exist.",
            root_path.display()
        );
    }

    let video_state = build_video_state().await?;
    println!(
        "[exporter] Video clip API enabled. Bucket='{}', tmp='{}'",
        video_state.video_bucket,
        video_state.video_tmp_dir.display()
    );

    let state = AppState {
        parquet_root: Arc::new(root_path),
        video: Arc::new(video_state),
    };

    let app = Router::new()
        .route("/export", get(handle_ws))
        .route("/video/clip", post(handle_video_clip))
        .route("/video/cameras", get(handle_video_cameras))
        .with_state(state);

    println!("[exporter] Listening on http://{listen_addr} (ws /export, post /video/clip)");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn build_video_state() -> Result<VideoState> {
    let video_bucket = std::env::var("VIDEO_BUCKET").unwrap_or_else(|_| "avena_videos".into());
    let video_tz_raw = std::env::var("VIDEO_TZ").unwrap_or_else(|_| "America/New_York".into());
    let video_tz = video_tz_raw.parse::<Tz>().map_err(|_| {
        anyhow!(
            "invalid VIDEO_TZ '{}': expected IANA timezone",
            video_tz_raw
        )
    })?;

    let ffmpeg_bin = std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".into());
    let video_tmp_dir =
        PathBuf::from(std::env::var("VIDEO_TMP_DIR").unwrap_or_else(|_| "/tmp/avena-video".into()));

    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| anyhow!("failed to load NATS creds: {e}"))?
        .request_timeout(None);

    let servers = parse_nats_servers_from_env()?;

    let nc = opts
        .connect(servers)
        .await
        .map_err(|e| anyhow!("NATS connect failed: {e}"))?;
    let js = jetstream::new(nc);

    // Ensure bucket exists for clip endpoints.
    if js.get_object_store(&video_bucket).await.is_err() {
        js.create_object_store(object_store::Config {
            bucket: video_bucket.clone(),
            ..Default::default()
        })
        .await
        .map_err(|e| {
            anyhow!(
                "failed to open or create VIDEO_BUCKET '{}': {e}",
                video_bucket
            )
        })?;
    }

    Ok(VideoState {
        jetstream: js,
        video_bucket,
        video_tz,
        ffmpeg_bin,
        video_tmp_dir,
    })
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
                .map_err(|e| anyhow!("invalid NATS server '{}': {e}", part))
        })
        .collect::<Result<Vec<_>>>()?;

    if servers.is_empty() {
        return Err(anyhow!("NATS_SERVERS resolved to an empty list"));
    }
    Ok(servers)
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    let payload = Json(ErrorPayload {
        error: message.into(),
    });
    (status, payload).into_response()
}

async fn handle_video_clip(
    State(state): State<AppState>,
    Json(req): Json<VideoClipRequest>,
) -> Response {
    match process_video_clip(state, req).await {
        Ok(response) => response,
        Err((status, msg)) => error_response(status, msg),
    }
}

async fn handle_video_cameras(
    State(state): State<AppState>,
    Query(req): Query<VideoCameraListRequest>,
) -> Response {
    let video = state.video.clone();

    let store = match video.jetstream.get_object_store(&video.video_bucket).await {
        Ok(store) => store,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "failed to open object store '{}': {err}",
                    video.video_bucket
                ),
            );
        }
    };

    let objects = match list_asset_video_objects(&store, req.asset, video.video_tz).await {
        Ok(objects) => objects,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list video objects: {err}"),
            );
        }
    };

    let mut cameras: Vec<String> = objects.iter().map(|obj| obj.camera_id.clone()).collect();
    cameras.sort();
    cameras.dedup();

    let mut by_camera: BTreeMap<String, Vec<VideoObject>> = BTreeMap::new();
    for object in &objects {
        by_camera
            .entry(object.camera_id.clone())
            .or_default()
            .push(object.clone());
    }
    let default_pre_sec = 5.0;
    let default_post_sec = 5.0;
    let grace = ChronoDuration::seconds(2);
    let coverage = by_camera
        .into_iter()
        .filter_map(|(camera_id, mut camera_objects)| {
            camera_objects.sort_by_key(|entry| entry.start);
            let latest = camera_objects.last()?;
            let latest_start = latest.start;
            let latest_end = latest.end;
            let (contiguous_start, contiguous_end) =
                latest_contiguous_window(&camera_objects, grace);
            let recommended_center_min =
                contiguous_start + ChronoDuration::milliseconds((default_pre_sec * 1000.0) as i64);
            let recommended_center_max =
                contiguous_end - ChronoDuration::milliseconds((default_post_sec * 1000.0) as i64);
            Some(VideoCameraCoverage {
                camera_id,
                latest_start: latest_start.to_rfc3339(),
                latest_end: latest_end.to_rfc3339(),
                recommended_center_min: recommended_center_min.to_rfc3339(),
                recommended_center_max: recommended_center_max.to_rfc3339(),
                contiguous_start: contiguous_start.to_rfc3339(),
                contiguous_end: contiguous_end.to_rfc3339(),
            })
        })
        .collect::<Vec<_>>();

    Json(VideoCameraListResponse {
        asset: req.asset,
        cameras,
        default_clip_pre_sec: default_pre_sec,
        default_clip_post_sec: default_post_sec,
        coverage,
    })
    .into_response()
}

async fn process_video_clip(
    state: AppState,
    req: VideoClipRequest,
) -> std::result::Result<Response, (StatusCode, String)> {
    let video = state.video.clone();

    let pre_sec = req.pre_sec.unwrap_or(5.0);
    let post_sec = req.post_sec.unwrap_or(5.0);
    if !pre_sec.is_finite() || pre_sec < 0.0 || !post_sec.is_finite() || post_sec < 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "pre_sec and post_sec must be non-negative finite numbers".to_string(),
        ));
    }

    let center_time = DateTime::parse_from_rfc3339(&req.center_time)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid center_time: {e}")))?
        .with_timezone(&Utc);

    let pre_ms = (pre_sec * 1000.0).round() as i64;
    let post_ms = (post_sec * 1000.0).round() as i64;
    let clip_start = center_time - ChronoDuration::milliseconds(pre_ms);
    let clip_end = center_time + ChronoDuration::milliseconds(post_ms);
    let requested_camera = req
        .camera_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    if clip_end <= clip_start {
        return Err((
            StatusCode::BAD_REQUEST,
            "computed clip interval is invalid".to_string(),
        ));
    }

    let clip_duration = seconds_between(clip_start, clip_end)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid clip window: {e}")))?;

    let store = video
        .jetstream
        .get_object_store(&video.video_bucket)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to open object store '{}': {e}", video.video_bucket),
            )
        })?;

    let objects = list_asset_video_objects(&store, req.asset, video.video_tz)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list video objects: {e}"),
            )
        })?;

    if objects.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("no video objects found for asset {}", req.asset),
        ));
    }

    let selected_camera = if let Some(camera) = requested_camera.clone() {
        camera
    } else {
        let mut cameras: Vec<String> = objects.iter().map(|obj| obj.camera_id.clone()).collect();
        cameras.sort();
        cameras.dedup();
        if cameras.len() > 1 {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "multiple camera feeds found for asset {} ({}); provide camera_id",
                    req.asset,
                    cameras.join(", ")
                ),
            ));
        }
        cameras
            .into_iter()
            .next()
            .unwrap_or_else(|| "default".to_string())
    };

    let filtered_objects: Vec<VideoObject> = objects
        .into_iter()
        .filter(|obj| obj.camera_id == selected_camera)
        .collect();

    if filtered_objects.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "no video objects found for asset {} camera_id {}",
                req.asset, selected_camera
            ),
        ));
    }

    let plan = resolve_clip_sources(&filtered_objects, clip_start, clip_end)?;

    let request_dir = video.video_tmp_dir.join(format!("clip-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&request_dir).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create temp directory: {e}"),
        )
    })?;

    let result: Result<(Vec<u8>, String)> = async {
        let output_path = request_dir.join("final_clip.mp4");

        match plan {
            ClipSourcePlan::Single(source) => {
                let source_path = request_dir.join("single_source.mp4");
                download_object_to_file(&store, &source.name, &source_path).await?;

                let offset_sec = seconds_between(source.start, clip_start)
                    .unwrap_or(0.0)
                    .max(0.0);
                run_ffmpeg_trim(
                    &video.ffmpeg_bin,
                    &source_path,
                    &output_path,
                    offset_sec,
                    clip_duration,
                )
                .await?;
            }
            ClipSourcePlan::Multi(sources) => {
                let grace = ChronoDuration::seconds(2);
                let parts = build_clip_parts(&sources, clip_start, clip_end, grace)?;
                if parts.is_empty() {
                    return Err(anyhow!("resolved clip parts are empty"));
                }

                let mut part_paths = Vec::with_capacity(parts.len());
                for (idx, (source, part_start, part_end)) in parts.iter().enumerate() {
                    let source_path = request_dir.join(format!("source_{idx}.mp4"));
                    download_object_to_file(&store, &source.name, &source_path).await?;

                    let part_path = request_dir.join(format!("part_{idx}.mp4"));
                    let offset_sec = seconds_between(source.start, *part_start)
                        .unwrap_or(0.0)
                        .max(0.0);
                    let duration_sec = seconds_between(*part_start, *part_end)?;
                    if duration_sec <= 0.0 {
                        continue;
                    }
                    run_ffmpeg_trim(
                        &video.ffmpeg_bin,
                        &source_path,
                        &part_path,
                        offset_sec,
                        duration_sec,
                    )
                    .await?;
                    part_paths.push(part_path);
                }

                if part_paths.is_empty() {
                    return Err(anyhow!("no valid clip parts after trimming"));
                }

                run_ffmpeg_concat_and_trim(
                    &video.ffmpeg_bin,
                    &part_paths,
                    &output_path,
                    clip_duration,
                    &request_dir,
                )
                .await?;
            }
        }

        let file_bytes = tokio::fs::read(&output_path)
            .await
            .with_context(|| format!("failed to read generated clip {}", output_path.display()))?;

        let filename = format!(
            "clip_asset{}_camera{}_{}_{}.mp4",
            req.asset,
            selected_camera,
            clip_start.format("%Y%m%dT%H%M%S"),
            clip_end.format("%Y%m%dT%H%M%S")
        );
        Ok((file_bytes, filename))
    }
    .await;

    let _ = tokio::fs::remove_dir_all(&request_dir).await;

    let (bytes, filename) = result.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to generate clip: {e}"),
        )
    })?;

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("video/mp4"));

    let content_disposition = format!("inline; filename=\"{}\"", filename);
    let content_disposition_value = HeaderValue::from_str(&content_disposition).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid response header: {e}"),
        )
    })?;
    response
        .headers_mut()
        .insert(header::CONTENT_DISPOSITION, content_disposition_value);

    Ok(response)
}

async fn list_asset_video_objects(
    store: &ObjectStore,
    asset: u32,
    tz: Tz,
) -> Result<Vec<VideoObject>> {
    let mut list = store
        .list()
        .await
        .map_err(|e| anyhow!("failed to start object listing: {e}"))?;

    let mut out = Vec::new();
    while let Some(item) = list.next().await {
        let info = item.map_err(|e| anyhow!("failed reading object metadata: {e}"))?;
        if let Some((key_asset, camera_id, start, end)) = parse_video_key_interval(&info.name, tz) {
            if key_asset == asset {
                out.push(VideoObject {
                    name: info.name,
                    camera_id,
                    start,
                    end,
                });
            }
        }
    }

    out.sort_by_key(|entry| entry.start);
    Ok(out)
}

fn parse_video_key_interval(
    name: &str,
    tz: Tz,
) -> Option<(u32, String, DateTime<Utc>, DateTime<Utc>)> {
    let parts: Vec<&str> = name.split('/').collect();
    let (asset_prefix, camera_id, file_name) = match parts.as_slice() {
        // Backward compatibility: asset1456/V_...mp4
        [asset, file] => (*asset, "default".to_string(), *file),
        // New format: asset1456/camera_cam11/V_...mp4
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
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() != 9 || parts[0] != "V" {
        return None;
    }

    let start_str = format!("{}_{}_{}_{}", parts[1], parts[2], parts[3], parts[4]);
    let end_str = format!("{}_{}_{}_{}", parts[5], parts[6], parts[7], parts[8]);

    let start_naive = NaiveDateTime::parse_from_str(&start_str, "%Y_%m_%d_%H%M%S").ok()?;
    let end_naive = NaiveDateTime::parse_from_str(&end_str, "%Y_%m_%d_%H%M%S").ok()?;

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
) -> std::result::Result<ClipSourcePlan, (StatusCode, String)> {
    if objects.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            "no video objects available".to_string(),
        ));
    }

    let grace = ChronoDuration::seconds(2);

    for object in objects {
        if object.start - grace <= clip_start && object.end + grace >= clip_end {
            return Ok(ClipSourcePlan::Single(object.clone()));
        }
    }

    let mut candidates: Vec<VideoObject> = objects
        .iter()
        .filter(|entry| entry.end + grace >= clip_start && entry.start - grace <= clip_end)
        .cloned()
        .collect();
    candidates.sort_by_key(|entry| entry.start);
    if candidates.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            "no source object covers requested interval".to_string(),
        ));
    }

    let mut best_chain: Option<Vec<VideoObject>> = None;
    for start in &candidates {
        if !(start.start - grace <= clip_start && start.end + grace >= clip_start) {
            continue;
        }
        let mut chain = vec![start.clone()];
        let mut reach = start.end;

        while reach + grace < clip_end {
            let next = candidates
                .iter()
                .filter(|entry| entry.start - grace <= reach && entry.end > reach)
                .max_by(|a, b| {
                    a.end
                        .cmp(&b.end)
                        .then_with(|| b.start.cmp(&a.start))
                        .then_with(|| a.name.cmp(&b.name))
                })
                .cloned();

            let Some(next) = next else { break };
            if chain
                .last()
                .map(|entry| entry.name == next.name)
                .unwrap_or(false)
            {
                break;
            }
            reach = next.end;
            chain.push(next);
            if chain.len() > candidates.len() + 1 {
                break;
            }
        }

        if reach + grace >= clip_end {
            let better = match best_chain.as_ref() {
                None => true,
                Some(current) => chain.len() < current.len(),
            };
            if better {
                best_chain = Some(chain);
            }
        }
    }

    if let Some(mut chain) = best_chain {
        chain.dedup_by(|a, b| a.name == b.name);
        if chain.len() == 1 {
            return Ok(ClipSourcePlan::Single(chain.remove(0)));
        }
        return Ok(ClipSourcePlan::Multi(chain));
    }

    if let Some(max_reach) = candidates
        .iter()
        .filter(|entry| entry.start - grace <= clip_start && entry.end + grace >= clip_start)
        .map(|entry| entry.end)
        .max()
    {
        if max_reach < clip_end {
            let gap_ms = (clip_end - max_reach).num_milliseconds().max(0);
            let gap_sec = gap_ms as f64 / 1000.0;
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "requested interval spans a gap between video objects ({gap_sec:.3}s missing after {})",
                    max_reach.to_rfc3339()
                ),
            ));
        }
    }

    Err((
        StatusCode::NOT_FOUND,
        "requested interval is not fully covered by available objects".to_string(),
    ))
}

async fn download_object_to_file(
    store: &ObjectStore,
    object_name: &str,
    target: &Path,
) -> Result<()> {
    let mut object = store
        .get(object_name)
        .await
        .map_err(|e| anyhow!("failed to get object '{}': {e}", object_name))?;

    let mut file = tokio::fs::File::create(target)
        .await
        .with_context(|| format!("failed to create target file {}", target.display()))?;

    tokio::io::copy(&mut object, &mut file)
        .await
        .with_context(|| format!("failed to download object '{}'", object_name))?;

    Ok(())
}

async fn run_ffmpeg_trim(
    ffmpeg_bin: &str,
    input_path: &Path,
    output_path: &Path,
    offset_sec: f64,
    duration_sec: f64,
) -> Result<()> {
    if duration_sec <= 0.0 {
        return Err(anyhow!("trim duration must be positive"));
    }

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
        .with_context(|| format!("failed to invoke ffmpeg binary '{}'", ffmpeg_bin))?;

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
    part_paths: &[PathBuf],
    output_path: &Path,
    total_duration_sec: f64,
    temp_dir: &Path,
) -> Result<()> {
    if part_paths.is_empty() {
        return Err(anyhow!("concat requires at least one part"));
    }

    let list_path = temp_dir.join("concat-list.txt");
    let mut list_content = String::new();
    for part_path in part_paths {
        let escaped = escape_ffmpeg_path(part_path.as_path());
        list_content.push_str(&format!("file '{}'\n", escaped));
    }
    tokio::fs::write(&list_path, list_content)
        .await
        .with_context(|| format!("failed to write concat list {}", list_path.display()))?;

    let stitched_path = temp_dir.join("stitched.mp4");
    let concat_output = Command::new(ffmpeg_bin)
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
        .with_context(|| format!("failed to invoke ffmpeg concat with '{}'", ffmpeg_bin))?;

    if !concat_output.status.success() {
        return Err(anyhow!(
            "ffmpeg concat failed (status {}): {}",
            concat_output.status,
            String::from_utf8_lossy(&concat_output.stderr)
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

fn escape_ffmpeg_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\\\''")
}

fn latest_contiguous_window(
    camera_objects: &[VideoObject],
    grace: ChronoDuration,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let latest = camera_objects
        .last()
        .expect("latest_contiguous_window requires non-empty camera_objects");
    let mut window_start = latest.start;
    let mut window_end = latest.end;

    for object in camera_objects.iter().rev().skip(1) {
        if object.end + grace >= window_start {
            if object.start < window_start {
                window_start = object.start;
            }
            if object.end > window_end {
                window_end = object.end;
            }
            continue;
        }
        break;
    }

    (window_start, window_end)
}

fn build_clip_parts(
    sources: &[VideoObject],
    clip_start: DateTime<Utc>,
    clip_end: DateTime<Utc>,
    grace: ChronoDuration,
) -> Result<Vec<(VideoObject, DateTime<Utc>, DateTime<Utc>)>> {
    let mut sorted = sources.to_vec();
    sorted.sort_by_key(|entry| entry.start);

    let mut cursor = clip_start;
    let mut parts = Vec::new();
    for source in sorted {
        if cursor >= clip_end {
            break;
        }
        if source.end + grace < cursor {
            continue;
        }
        if source.start - grace > cursor {
            break;
        }

        let part_start = if source.start > cursor {
            source.start
        } else {
            cursor
        };
        let part_end = if source.end < clip_end {
            source.end
        } else {
            clip_end
        };
        if part_end <= part_start {
            continue;
        }
        cursor = part_end;
        parts.push((source, part_start, part_end));
    }

    if cursor + grace < clip_end {
        return Err(anyhow!(
            "resolved sources do not fully cover requested clip window"
        ));
    }

    Ok(parts)
}

fn seconds_between(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<f64> {
    if end < start {
        return Err(anyhow!("negative time delta"));
    }

    let delta = end - start;
    let milliseconds = delta.num_milliseconds();
    Ok(milliseconds as f64 / 1000.0)
}

async fn handle_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = process_socket(socket, state).await {
            eprintln!("[exporter] websocket error: {err:?}");
        }
    })
}

async fn process_socket(mut socket: WebSocket, state: AppState) -> Result<()> {
    let Some(msg) = socket.next().await else {
        return Ok(());
    };

    let Message::Text(text) = msg? else {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"expected JSON request"})
                    .to_string()
                    .into(),
            ))
            .await
            .ok();
        return Ok(());
    };

    let mut req: ExportRequest =
        serde_json::from_str(&text).map_err(|e| anyhow!("invalid request payload: {e}"))?;

    if req.channels.is_empty() {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"no channels requested"})
                    .to_string()
                    .into(),
            ))
            .await
            .ok();
        return Ok(());
    }
    req.channels.sort_unstable();
    req.channels.dedup();

    let (start, end) = parse_range(&req.start, &req.end)?;
    if end < start {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"end must be after start"})
                    .to_string()
                    .into(),
            ))
            .await
            .ok();
        return Ok(());
    }

    if !matches!(req.format, ExportFormat::Csv) {
        socket
            .send(Message::Text(
                json!({"type":"error","message":"parquet streaming not yet supported"})
                    .to_string()
                    .into(),
            ))
            .await
            .ok();
        return Ok(());
    }

    let file_name = req.download_name.clone().unwrap_or_else(|| {
        format!(
            "labjack_asset{:03}_{}_{}.csv",
            req.asset,
            start.format("%Y%m%dT%H%M%S"),
            end.format("%Y%m%dT%H%M%S"),
        )
    });

    socket
        .send(Message::Text(
            json!({
                "type": "meta",
                "fileName": file_name,
                "contentType": "text/csv"
            })
            .to_string()
            .into(),
        ))
        .await?;

    let mut stream = CsvStreamer::new(socket, req.asset, start, end);
    let missing = stream
        .stream_channels(&state.parquet_root, &req.channels)
        .await?;
    stream.finish(missing).await?;
    Ok(())
}

struct CsvStreamer {
    socket: WebSocket,
    chunk: Vec<u8>,
    bytes_sent: usize,
    asset: u32,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

impl CsvStreamer {
    const CHUNK_SIZE: usize = 128 * 1024;

    fn new(socket: WebSocket, asset: u32, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        let mut chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        chunk.extend_from_slice(b"timestamp,channel,raw_value,calibrated_value,calibration_id\n");
        Self {
            socket,
            chunk,
            bytes_sent: 0,
            asset,
            start,
            end,
        }
    }

    async fn stream_channels(&mut self, parquet_root: &Path, channels: &[u8]) -> Result<Vec<u8>> {
        let mut missing = Vec::new();
        for &channel in channels {
            let found = self
                .stream_channel(parquet_root, channel)
                .await
                .map_err(|e| anyhow!("channel {channel:02}: {e}"))?;
            if !found {
                missing.push(channel);
            }
        }
        Ok(missing)
    }

    async fn flush(&mut self) -> Result<()> {
        if self.chunk.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.chunk);
        self.bytes_sent += data.len();
        self.socket.send(Message::Binary(data.into())).await?;
        self.chunk = Vec::with_capacity(Self::CHUNK_SIZE);
        Ok(())
    }

    async fn push_record(
        &mut self,
        timestamp: &str,
        channel: u8,
        raw_value: f64,
        calibrated_value: f64,
        calibration_id: &str,
    ) -> Result<()> {
        let line =
            format!("{timestamp},ch{channel:02},{raw_value},{calibrated_value},{calibration_id}\n");
        self.chunk.extend_from_slice(line.as_bytes());
        if self.chunk.len() >= Self::CHUNK_SIZE {
            self.flush().await?;
        }
        Ok(())
    }

    async fn finish(mut self, mut missing_channels: Vec<u8>) -> Result<()> {
        self.flush().await?;
        missing_channels.sort_unstable();
        missing_channels.dedup();
        let summary = json!({
            "type": "summary",
            "bytesSent": self.bytes_sent,
            "missingChannels": missing_channels
        });
        self.socket
            .send(Message::Text(summary.to_string().into()))
            .await?;
        self.socket
            .send(Message::Text(json!({"type":"complete"}).to_string().into()))
            .await?;
        self.socket.close().await.ok();
        Ok(())
    }

    async fn stream_channel(&mut self, root: &Path, channel: u8) -> Result<bool> {
        let mut found = false;
        for day in date_range(self.start.date_naive(), self.end.date_naive()) {
            let day_dir = root
                .join(format!("asset{:03}", self.asset))
                .join(day.format("%Y-%m-%d").to_string())
                .join(format!("ch{:02}", channel));
            if !day_dir.exists() {
                continue;
            }

            let mut files: Vec<PathBuf> = fs::read_dir(&day_dir)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.ends_with(".parquet"))
                        .unwrap_or(false)
                })
                .collect();
            files.sort();

            for path in files {
                if let Err(err) = self.stream_parquet_file(&path, channel, &mut found).await {
                    eprintln!("[exporter] skipping {} due to error: {err}", path.display());
                }
            }
        }
        Ok(found)
    }

    async fn stream_parquet_file(
        &mut self,
        path: &Path,
        channel: u8,
        found: &mut bool,
    ) -> Result<()> {
        let file = fs::File::open(path)
            .with_context(|| format!("failed to open parquet file {}", path.display()))?;
        let reader = SerializedFileReader::new(file)
            .with_context(|| format!("failed to create reader for {}", path.display()))?;
        let calibration = read_calibration_from_metadata(&reader, path);
        let calibration_id = calibration.id_or_default().to_string();
        let mut iter = reader.get_row_iter(None)?;
        while let Some(row) = iter.next() {
            let row = row?;
            let ts = row.get_string(0)?;
            let ts_parsed = match DateTime::parse_from_rfc3339(ts) {
                Ok(dt) => dt.with_timezone(&Utc),
                Err(_) => continue,
            };
            if ts_parsed < self.start || ts_parsed > self.end {
                continue;
            }
            let raw_value = row.get_double(1)?;
            let calibrated_value = calibration.apply(raw_value);
            *found = true;
            self.push_record(ts, channel, raw_value, calibrated_value, &calibration_id)
                .await?;
        }
        Ok(())
    }
}

fn read_calibration_from_metadata(
    reader: &SerializedFileReader<fs::File>,
    path: &Path,
) -> CalibrationSpec {
    let Some(kv) = reader.metadata().file_metadata().key_value_metadata() else {
        return CalibrationSpec::default();
    };

    let mut calibration_json = None;
    for item in kv {
        if item.key == "calibration" {
            calibration_json = item.value.as_deref();
            break;
        }
    }

    let Some(json) = calibration_json else {
        return CalibrationSpec::default();
    };

    match serde_json::from_str::<CalibrationSpec>(json) {
        Ok(spec) => spec,
        Err(err) => {
            eprintln!(
                "[exporter] invalid calibration metadata in {}: {err}",
                path.display()
            );
            CalibrationSpec::default()
        }
    }
}

fn parse_range(start: &str, end: &str) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start = DateTime::parse_from_rfc3339(start)
        .map_err(|e| anyhow!("invalid start timestamp: {e}"))?
        .with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(end)
        .map_err(|e| anyhow!("invalid end timestamp: {e}"))?
        .with_timezone(&Utc);
    Ok((start, end))
}

fn date_range(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    let mut days = Vec::new();
    let mut current = start;
    while current <= end {
        days.push(current);
        current += ChronoDuration::days(1);
    }
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt_utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("valid dt")
            .with_timezone(&Utc)
    }

    #[test]
    fn parse_key_accepts_valid_object_name() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let parsed = parse_video_key_interval(
            "asset1456/camera_cam11/V_2026_02_13_090035_2026_02_13_091035.mp4",
            tz,
        )
        .expect("should parse");

        assert_eq!(parsed.0, 1456);
        assert_eq!(parsed.1, "cam11");
        assert!(parsed.3 > parsed.2);
    }

    #[test]
    fn parse_key_rejects_invalid_name() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let parsed = parse_video_key_interval("asset1456/bad_name.mp4", tz);
        assert!(parsed.is_none());
    }

    #[test]
    fn parse_key_supports_legacy_without_camera_prefix() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let parsed =
            parse_video_key_interval("asset1456/V_2026_02_13_090035_2026_02_13_091035.mp4", tz)
                .expect("should parse");
        assert_eq!(parsed.0, 1456);
        assert_eq!(parsed.1, "default");
        assert!(parsed.3 > parsed.2);
    }

    #[test]
    fn resolve_clip_single_source() {
        let objects = vec![VideoObject {
            name: "asset1456/V_2026_02_13_090000_2026_02_13_091000.mp4".to_string(),
            camera_id: "default".to_string(),
            start: dt_utc("2026-02-13T14:00:00Z"),
            end: dt_utc("2026-02-13T14:10:00Z"),
        }];

        let plan = resolve_clip_sources(
            &objects,
            dt_utc("2026-02-13T14:05:00Z"),
            dt_utc("2026-02-13T14:05:10Z"),
        )
        .expect("single source");

        match plan {
            ClipSourcePlan::Single(_) => {}
            ClipSourcePlan::Multi(_) => panic!("expected single source"),
        }
    }

    #[test]
    fn resolve_clip_stitched_sources() {
        let objects = vec![
            VideoObject {
                name: "asset1456/V_2026_02_13_090000_2026_02_13_091000.mp4".to_string(),
                camera_id: "default".to_string(),
                start: dt_utc("2026-02-13T14:00:00Z"),
                end: dt_utc("2026-02-13T14:10:00Z"),
            },
            VideoObject {
                name: "asset1456/V_2026_02_13_091000_2026_02_13_092000.mp4".to_string(),
                camera_id: "default".to_string(),
                start: dt_utc("2026-02-13T14:10:00Z"),
                end: dt_utc("2026-02-13T14:20:00Z"),
            },
        ];

        let plan = resolve_clip_sources(
            &objects,
            dt_utc("2026-02-13T14:09:55Z"),
            dt_utc("2026-02-13T14:10:05Z"),
        )
        .expect("stitch source");

        match plan {
            ClipSourcePlan::Single(_) => panic!("expected stitched source"),
            ClipSourcePlan::Multi(parts) => assert_eq!(parts.len(), 2),
        }
    }
}
