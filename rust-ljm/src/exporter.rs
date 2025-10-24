use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use async_nats::{
    Client, ConnectOptions, HeaderMap, HeaderValue, ServerAddr,
    jetstream::{self, Context as JetStream, object_store},
};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use futures_util::StreamExt;
use parquet::{
    column::writer::ColumnWriter,
    data_type::ByteArray,
    file::{
        properties::WriterProperties,
        reader::{FileReader, SerializedFileReader},
        writer::SerializedFileWriter,
    },
    record::RowAccessor,
    schema::parser::parse_message_type,
};
use serde::{Deserialize, Serialize};
use tokio::{fs as tokio_fs, io::AsyncReadExt};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExportFormat {
    Parquet,
    Csv,
}

#[derive(Debug, Deserialize)]
struct ExportRequest {
    /// Optional caller-specified identifier to mirror in the response.
    request_id: Option<String>,
    asset: u32,
    channels: Vec<u8>,
    start: String,
    end: String,
    format: ExportFormat,
    /// Optional override for the object bucket name.
    bucket: Option<String>,
    /// Optional filename hint for the download.
    download_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ExportResponse {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    download_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    control_subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chunk_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    missing_channels: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

struct CombinedSample {
    timestamp_utc: DateTime<Utc>,
    timestamp_str: String,
    channel: u8,
    value: f64,
}

#[derive(Debug, Serialize)]
struct DownloadMeta {
    bucket: String,
    #[serde(skip_serializing)]
    object: String,
    download_name: String,
    size_bytes: u64,
    format: String,
    chunk_size: usize,
}

fn schedule_download_stream(
    client: Client,
    object_store: object_store::ObjectStore,
    control_subject: String,
    data_subject: String,
    meta: DownloadMeta,
) {
    tokio::spawn(async move {
        if let Err(err) = run_download_stream(
            client.clone(),
            object_store,
            control_subject,
            data_subject.clone(),
            meta,
        )
        .await
        {
            eprintln!("[exporter] download stream error: {err:?}");
            let _ = send_error_event(&client, &data_subject, err.to_string()).await;
        }
    });
}

async fn run_download_stream(
    client: Client,
    object_store: object_store::ObjectStore,
    control_subject: String,
    data_subject: String,
    meta: DownloadMeta,
) -> Result<()> {
    let mut control_sub = client.subscribe(control_subject).await?;
    let start_msg = match control_sub.next().await {
        Some(msg) => msg,
        None => return Ok(()),
    };
    if let Some(reply) = &start_msg.reply {
        let _ = client
            .publish(
                reply.clone(),
                serde_json::to_vec(&serde_json::json!({"status": "ready"}))?.into(),
            )
            .await;
    }
    drop(control_sub);

    let object = object_store
        .get(&meta.object)
        .await
        .map_err(|e| anyhow!("failed to fetch object from store: {e}"))?;

    let mut meta_headers = HeaderMap::new();
    meta_headers.insert("Nats-Download-Event", HeaderValue::from("meta"));
    client
        .publish_with_headers(
            data_subject.clone(),
            meta_headers,
            serde_json::to_vec(&meta)?.into(),
        )
        .await?;

    tokio::pin!(object);
    let mut buffer = vec![0u8; meta.chunk_size];
    let mut seq: u64 = 0;

    loop {
        let read = object.as_mut().read(&mut buffer).await?;
        if read == 0 {
            break;
        }

        let mut headers = HeaderMap::new();
        headers.insert("Nats-Download-Event", HeaderValue::from("chunk"));
        headers.insert("Nats-Download-Seq", HeaderValue::from(seq.to_string()));
        client
            .publish_with_headers(
                data_subject.clone(),
                headers,
                buffer[..read].to_vec().into(),
            )
            .await?;
        seq += 1;
    }

    let mut done_headers = HeaderMap::new();
    done_headers.insert("Nats-Download-Event", HeaderValue::from("complete"));
    client
        .publish_with_headers(data_subject, done_headers, Vec::<u8>::new().into())
        .await?;

    Ok(())
}

async fn send_error_event(client: &Client, subject: &str, message: String) -> Result<()> {
    let mut headers = HeaderMap::new();
    headers.insert("Nats-Download-Event", HeaderValue::from("error"));
    client
        .publish_with_headers(
            subject.to_string(),
            headers,
            serde_json::to_vec(&serde_json::json!({ "error": message }))?.into(),
        )
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let creds_path = std::env::var("NATS_CREDS_FILE").unwrap_or_else(|_| "apt.creds".into());
    let opts = ConnectOptions::with_credentials_file(creds_path)
        .await
        .map_err(|e| anyhow!("Failed to load creds: {e}"))?;

    let servers: Vec<ServerAddr> = vec![
        "nats://nats1.oats:4222"
            .parse()
            .map_err(|e| anyhow!("invalid server addr: {e}"))?,
        "nats://nats2.oats:4222"
            .parse()
            .map_err(|e| anyhow!("invalid server addr: {e}"))?,
        "nats://nats3.oats:4222"
            .parse()
            .map_err(|e| anyhow!("invalid server addr: {e}"))?,
    ];

    let nc = opts
        .connect(servers)
        .await
        .map_err(|e| anyhow!("NATS connect failed: {e}"))?;
    println!("[exporter] Connected to NATS via creds");

    let js = jetstream::new(nc.clone());
    let default_bucket =
        std::env::var("EXPORT_BUCKET").unwrap_or_else(|_| "labjack_exports".into());
    let request_subject = std::env::var("EXPORT_REQUEST_SUBJECT")
        .unwrap_or_else(|_| "avenabox.export.request".into());
    let parquet_root = std::env::var("PARQUET_DIR").unwrap_or_else(|_| "parquet".into());

    let base_dir = Arc::new(PathBuf::from(parquet_root));
    if !base_dir.exists() {
        println!(
            "[exporter] Warning: parquet root '{}' does not exist yet.",
            base_dir.display()
        );
    }

    let store = ensure_object_store(&js, &default_bucket).await?;
    println!(
        "[exporter] Ready. Listening on '{}' and writing objects to bucket '{}'",
        request_subject, default_bucket
    );

    let mut sub = nc.subscribe(request_subject.clone()).await?;
    while let Some(msg) = sub.next().await {
        let store = store.clone();
        let client = nc.clone();
        let js = js.clone();
        let default_bucket = default_bucket.clone();
        let base_dir = base_dir.clone();

        tokio::spawn(async move {
            let reply_subject = match msg.reply.clone() {
                Some(subject) => subject,
                None => {
                    eprintln!("[exporter] Received request without reply subject");
                    return;
                }
            };

            let response = match process_request(
                msg.payload.as_ref(),
                store,
                js,
                client.clone(),
                base_dir,
                default_bucket,
            )
            .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    eprintln!("[exporter] request error: {err:?}");
                    ExportResponse {
                        status: "error".into(),
                        request_id: None,
                        object: None,
                        bucket: None,
                        size_bytes: None,
                        download_name: None,
                        control_subject: None,
                        data_subject: None,
                        chunk_size: None,
                        content_type: None,
                        missing_channels: Vec::new(),
                        error: Some(err.to_string()),
                    }
                }
            };

            if let Err(err) = client
                .publish(reply_subject, serde_json::to_vec(&response).unwrap().into())
                .await
            {
                eprintln!("[exporter] Failed to send response: {err:?}");
            }
        });
    }

    Ok(())
}

async fn ensure_object_store(js: &JetStream, bucket: &str) -> Result<object_store::ObjectStore> {
    match js.get_object_store(bucket).await {
        Ok(store) => Ok(store),
        Err(_) => {
            println!("[exporter] Creating object store bucket '{bucket}'");
            let config = object_store::Config {
                bucket: bucket.to_string(),
                description: Some("LabJack combined exports".into()),
                max_age: std::time::Duration::from_secs(7 * 24 * 60 * 60),
                max_bytes: -1,
                storage: jetstream::stream::StorageType::File,
                num_replicas: 1,
                compression: true,
                placement: None,
            };
            js.create_object_store(config)
                .await
                .map_err(|e| anyhow!("failed to create object store '{bucket}': {e}"))
        }
    }
}

async fn process_request(
    payload: &[u8],
    store: object_store::ObjectStore,
    js: JetStream,
    client: Client,
    parquet_root: Arc<PathBuf>,
    default_bucket: String,
) -> Result<ExportResponse> {
    let mut req: ExportRequest = serde_json::from_slice(payload)
        .map_err(|e| anyhow!("failed to parse request JSON: {e}"))?;

    if req.channels.is_empty() {
        return Err(anyhow!("request must include at least one channel"));
    }

    req.channels.sort_unstable();
    req.channels.dedup();

    let (start, end) = parse_range(&req.start, &req.end)?;
    if end < start {
        return Err(anyhow!("end must be after start"));
    }

    let bucket = req.bucket.clone().unwrap_or_else(|| default_bucket.clone());
    let object_store = if bucket == default_bucket {
        store
    } else {
        ensure_object_store(&js, &bucket).await?
    };

    let parquet_dir = parquet_root.as_ref();
    let mut records = Vec::new();
    let mut missing_channels: Vec<u8> = Vec::new();
    for channel in &req.channels {
        match collect_channel_data(parquet_dir.as_path(), req.asset, *channel, start, end) {
            Ok(mut channel_records) => {
                if channel_records.is_empty() {
                    missing_channels.push(*channel);
                } else {
                    records.append(&mut channel_records);
                }
            }
            Err(err) => {
                eprintln!(
                    "[exporter] failed to collect data for asset {:03} channel {:02}: {err}",
                    req.asset, channel
                );
                missing_channels.push(*channel);
            }
        }
    }

    missing_channels.sort_unstable();
    missing_channels.dedup();

    if records.is_empty() {
        return Ok(ExportResponse {
            status: "empty".into(),
            request_id: req.request_id,
            object: None,
            bucket: Some(bucket),
            size_bytes: Some(0),
            download_name: req.download_name,
            control_subject: None,
            data_subject: None,
            chunk_size: None,
            content_type: None,
            missing_channels,
            error: None,
        });
    }

    records.sort_by(|a, b| {
        a.timestamp_utc
            .cmp(&b.timestamp_utc)
            .then_with(|| a.channel.cmp(&b.channel))
    });

    let (file_path, extension) = create_temp_file(&req.format).await?;
    match req.format {
        ExportFormat::Csv => write_csv(&records, &file_path)?,
        ExportFormat::Parquet => write_parquet(&records, &file_path)?,
    }

    let object_name = build_object_name(
        req.asset,
        &req.channels,
        start,
        end,
        extension,
        req.request_id.as_deref(),
    );

    let download_name = req.download_name.unwrap_or_else(|| {
        format!(
            "labjack_asset{:03}_{}_{}.{}",
            req.asset,
            start.format("%Y%m%dT%H%M%S"),
            end.format("%Y%m%dT%H%M%S"),
            extension
        )
    });

    let metadata = object_store::ObjectMetadata {
        name: object_name.clone(),
        description: Some(format!(
            "LabJack export asset {:03} channels {:?}",
            req.asset, req.channels
        )),
        metadata: HashMap::from([
            ("asset".into(), req.asset.to_string()),
            ("channels".into(), format!("{:?}", req.channels)),
            ("start".into(), start.to_rfc3339()),
            ("end".into(), end.to_rfc3339()),
            ("format".into(), extension.to_string()),
        ]),
        ..Default::default()
    };

    let mut file = tokio_fs::File::open(&file_path).await?;
    let object_info = object_store
        .put(metadata, &mut file)
        .await
        .map_err(|e| anyhow!("failed to upload object: {e}"))?;
    let size_bytes = object_info.size as u64;

    tokio_fs::remove_file(&file_path).await.ok();

    let chunk_size: usize = 128 * 1024;
    let inbox = client.new_inbox();
    let control_subject = format!("{inbox}.control");
    let data_subject = format!("{inbox}.data");

    let meta = DownloadMeta {
        bucket: bucket.clone(),
        object: object_name.clone(),
        download_name: download_name.clone(),
        size_bytes,
        format: extension.to_string(),
        chunk_size,
    };

    schedule_download_stream(
        client,
        object_store,
        control_subject.clone(),
        data_subject.clone(),
        meta,
    );

    Ok(ExportResponse {
        status: "ok".into(),
        request_id: req.request_id,
        object: Some(object_name),
        bucket: Some(bucket),
        size_bytes: Some(size_bytes),
        download_name: Some(download_name),
        control_subject: Some(control_subject),
        data_subject: Some(data_subject),
        chunk_size: Some(chunk_size as u32),
        content_type: Some(match req.format {
            ExportFormat::Csv => "text/csv".to_string(),
            ExportFormat::Parquet => "application/x-parquet".to_string(),
        }),
        missing_channels,
        error: None,
    })
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

fn collect_channel_data(
    root: &Path,
    asset: u32,
    channel: u8,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CombinedSample>> {
    let mut samples = Vec::new();
    for day in date_range(start.date_naive(), end.date_naive()) {
        let day_dir = root
            .join(format!("asset{:03}", asset))
            .join(day.format("%Y-%m-%d").to_string())
            .join(format!("ch{:02}", channel));
        if !day_dir.exists() {
            continue;
        }

        let mut files: Vec<PathBuf> = fs::read_dir(&day_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.ends_with(".parquet"))
                    .unwrap_or(false)
            })
            .map(|entry| entry.path())
            .collect();
        files.sort();

        for path in files {
            if let Err(err) = read_parquet_file(&path, channel, start, end, &mut samples) {
                eprintln!("[exporter] unable to read {}: {err}", path.display());
            }
        }
    }

    Ok(samples)
}

fn read_parquet_file(
    path: &Path,
    channel: u8,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    out: &mut Vec<CombinedSample>,
) -> Result<()> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open parquet file {}", path.display()))?;
    let reader = SerializedFileReader::new(file)
        .with_context(|| format!("failed to create reader for {}", path.display()))?;
    let mut iter = reader.get_row_iter(None)?;
    while let Some(row) = iter.next() {
        let row = row?;
        let ts_str = row.get_string(0)?;
        let ts = DateTime::parse_from_rfc3339(ts_str)
            .map_err(|e| anyhow!("invalid timestamp '{ts_str}' in {}: {e}", path.display()))?
            .with_timezone(&Utc);
        if ts < start || ts > end {
            continue;
        }

        let value = row.get_double(1)?;
        out.push(CombinedSample {
            timestamp_utc: ts,
            timestamp_str: ts_str.to_string(),
            channel,
            value,
        });
    }
    Ok(())
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

async fn create_temp_file(format: &ExportFormat) -> Result<(PathBuf, &'static str)> {
    let extension = match format {
        ExportFormat::Csv => "csv",
        ExportFormat::Parquet => "parquet",
    };
    let temp_dir = std::env::temp_dir();
    let file_name = format!("labjack-export-{}.{}", Uuid::new_v4(), extension);
    let path = temp_dir.join(file_name);
    tokio_fs::File::create(&path)
        .await
        .with_context(|| format!("failed to create temp file {}", path.display()))?;
    Ok((path, extension))
}

fn write_csv(records: &[CombinedSample], path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record(["timestamp", "channel", "value"])?;
    for record in records {
        let channel_str = record.channel.to_string();
        let value_str = record.value.to_string();
        wtr.write_record([
            record.timestamp_str.as_str(),
            channel_str.as_str(),
            value_str.as_str(),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}

fn write_parquet(records: &[CombinedSample], path: &Path) -> Result<()> {
    let schema_str = "
        message schema {
            REQUIRED BINARY timestamp (UTF8);
            REQUIRED INT32 channel;
            REQUIRED DOUBLE value;
        }
    ";
    let schema = Arc::new(parse_message_type(schema_str)?);
    let props = Arc::new(WriterProperties::builder().build());
    let file = fs::File::create(path)?;
    let mut writer = SerializedFileWriter::new(file, schema, props)?;

    if records.is_empty() {
        writer.close()?;
        return Ok(());
    }

    let mut row_group = writer.next_row_group()?;
    {
        let mut column = row_group.next_column()?.expect("timestamp column");
        match column.untyped() {
            ColumnWriter::ByteArrayColumnWriter(typed) => {
                let values: Vec<ByteArray> = records
                    .iter()
                    .map(|r| ByteArray::from(r.timestamp_str.as_str()))
                    .collect();
                typed.write_batch(&values, None, None)?;
            }
            _ => return Err(anyhow!("unexpected parquet writer type for timestamp")),
        };
        column.close()?;
    }
    {
        let mut column = row_group.next_column()?.expect("channel column");
        match column.untyped() {
            ColumnWriter::Int32ColumnWriter(typed) => {
                let values: Vec<i32> = records.iter().map(|r| r.channel as i32).collect();
                typed.write_batch(&values, None, None)?;
            }
            _ => return Err(anyhow!("unexpected parquet writer type for channel")),
        };
        column.close()?;
    }
    {
        let mut column = row_group.next_column()?.expect("value column");
        match column.untyped() {
            ColumnWriter::DoubleColumnWriter(typed) => {
                let values: Vec<f64> = records.iter().map(|r| r.value).collect();
                typed.write_batch(&values, None, None)?;
            }
            _ => return Err(anyhow!("unexpected parquet writer type for value")),
        };
        column.close()?;
    }
    row_group.close()?;
    writer.close()?;
    Ok(())
}

fn build_object_name(
    asset: u32,
    channels: &[u8],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    extension: &str,
    request_id: Option<&str>,
) -> String {
    let channels_str: String = channels
        .iter()
        .map(|ch| format!("ch{:02}", ch))
        .collect::<Vec<_>>()
        .join("-");

    let base = format!(
        "asset{:03}_{}_{}_{}",
        asset,
        start.format("%Y%m%dT%H%M%S"),
        end.format("%Y%m%dT%H%M%S"),
        channels_str
    );

    if let Some(id) = request_id {
        format!("{base}_{id}.{extension}")
    } else {
        format!("{base}.{}", extension)
    }
}
