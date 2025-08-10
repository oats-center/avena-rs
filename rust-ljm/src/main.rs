use tokio::sync::watch;
use tokio::time::{sleep, Duration};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, time::SystemTime};
use chrono::{TimeZone, Utc};
use chrono_tz::America::New_York;
use std::fs::File;
use std::io::Write;

use ljmrs::{LJMLibrary, LJMError};
use ljmrs::handle::{DeviceType, ConnectionType};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SampleConfig {
    scans_per_read: i32,
    suggested_scan_rate: f64,
    channels: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<(), LJMError> {
    let config_path = "config/sample.json";
    ensure_csv_dir().map_err(|e| LJMError::LibraryError(format!("Failed to create CSV dir: {}", e)))?;

    let cfg = load_config(config_path)
        .map_err(|e| LJMError::LibraryError(format!("Failed to load config: {}", e)))?;
    let (config_tx, config_rx) = watch::channel(cfg);

    // Initialize LJM
    #[cfg(feature = "staticlib")]
    unsafe { LJMLibrary::init()?; }
    #[cfg(all(feature = "dynlink", not(feature = "staticlib")))]
    unsafe {
        let path = std::env::var("LJM_PATH").ok();
        LJMLibrary::init(path)?;
    }

    tokio::spawn(run_sampler(config_rx.clone()));
    tokio::spawn(watch_config_file(config_path.to_string(), config_tx));

    tokio::signal::ctrl_c()
        .await
        .map_err(|e| LJMError::LibraryError(format!("Failed to listen for Ctrl+C: {}", e)))?;

    println!("Shutting down...");
    Ok(())
}

fn ensure_csv_dir() -> std::io::Result<()> {
    let path = Path::new("outputs/csv");
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

fn make_csv_filename(cfg: &SampleConfig) -> String {
    let channels_str = cfg.channels.iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join("-");
    let now_nyc = New_York.from_utc_datetime(&Utc::now().naive_utc());
    format!(
        "outputs/csv/{}_scans{}_rate{}_ch{}.csv",
        now_nyc.format("%Y-%m-%d_%H-%M-%S"),
        cfg.scans_per_read,
        cfg.suggested_scan_rate,
        channels_str
    )
}

fn load_config<P: AsRef<Path>>(path: P) -> Result<SampleConfig, std::io::Error> {
    let data = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("JSON parse error: {}", e))
    })?)
}

async fn watch_config_file(path: String, config_tx: watch::Sender<SampleConfig>) {
    let mut last_modified: Option<SystemTime> = None;

    loop {
        if let Ok(metadata) = fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                if last_modified.map_or(true, |lm| lm != modified) {
                    last_modified = Some(modified);
                    match load_config(&path) {
                        Ok(cfg) => {
                            println!("Config updated: {:?}", cfg);
                            let _ = config_tx.send(cfg);
                        }
                        Err(e) => eprintln!("Failed to reload config: {:?}", e),
                    }
                }
            }
        }
        sleep(Duration::from_secs(1)).await;
    }
}

async fn run_sampler(mut config_rx: watch::Receiver<SampleConfig>) {
    loop {
        let cfg = config_rx.borrow().clone();
        println!("Starting sampler with {:?}", cfg);

        if let Err(e) = sample_with_config(cfg.clone(), &mut config_rx).await {
            eprintln!("Sampler error: {:?}", e);
        }

        println!("Restarting sampler after config change...");
    }
}

async fn sample_with_config(
    cfg: SampleConfig,
    config_rx: &mut watch::Receiver<SampleConfig>,
) -> Result<(), LJMError> {
    let handle = LJMLibrary::open_jack(DeviceType::ANY, ConnectionType::ANY, "ANY")?;
    let info = LJMLibrary::get_handle_info(handle)?;
    println!("Connected to {:?} (serial {})", info.device_type, info.serial_number);

    if matches!(info.device_type, DeviceType::T7) {
        LJMLibrary::write_name(handle, "AIN_ALL_NEGATIVE_CH", 199_u32)?;
    }
    LJMLibrary::write_name(handle, "AIN_ALL_RANGE", 10.0_f64)?;
    LJMLibrary::write_name(handle, "AIN_ALL_RESOLUTION_INDEX", 0_u32)?;
    LJMLibrary::write_name(handle, "STREAM_SETTLING_US", 50_u32)?;


    // Prepare channel addresses
    let channel_addresses: Vec<i32> = cfg.channels.iter()
        .map(|ch| LJMLibrary::name_to_address(&format!("AIN{}", ch)).unwrap().0)
        .collect();
    let num_channels = channel_addresses.len();
    println!("Expecting {} channels per scan", num_channels);

    // Start stream
    // Start stream
    let actual_rate = LJMLibrary::stream_start(
        handle,
        cfg.scans_per_read,
        cfg.suggested_scan_rate,
        channel_addresses.clone(), // pass Vec<i32>
    )?;
    println!(
        "Streaming started: {} scans/read @ {} Hz",
        cfg.scans_per_read, actual_rate
    );


    // Open CSV file
    let filename = make_csv_filename(&cfg);
    let mut file = File::create(&filename)
        .map_err(|e| LJMError::LibraryError(format!("Failed to create CSV file: {}", e)))?;
    writeln!(file, "timestamp,values")
        .map_err(|e| LJMError::LibraryError(format!("Failed to write CSV header: {}", e)))?;

    loop {
        tokio::select! {
            _ = config_rx.changed() => {
                println!("Config change detected. Stopping stream...");
                LJMLibrary::stream_stop(handle)?;
                LJMLibrary::close_jack(handle)?;
                return Ok(());
            }
            result = tokio::task::spawn_blocking({
                let handle = handle;
                move || LJMLibrary::stream_read(handle)
            }) => {
                let batch = result.unwrap()?; // Vec<f64>
                let scans = batch.chunks(num_channels).collect::<Vec<_>>();

                for scan in scans {
                    let ts = New_York.from_utc_datetime(&Utc::now().naive_utc()).to_rfc3339();
                    let json_values = serde_json::to_string(scan).unwrap();
                    writeln!(file, "{},{}", ts, json_values)
                        .map_err(|e| LJMError::LibraryError(format!("Failed to write CSV row: {}", e)))?;
                }
                file.flush()
                    .map_err(|e| LJMError::LibraryError(format!("Failed to flush CSV: {}", e)))?;
            }
        }
    }
}
