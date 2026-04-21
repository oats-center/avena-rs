use ljmrs::handle::DeviceType;
use ljmrs::{LJMError, LJMLibrary};
use std::thread;
use std::time::Duration;

#[path = "common/example_env.rs"]
mod example_env;
#[path = "../src/labjack.rs"]
mod labjack;
#[path = "../src/ljm_mode.rs"]
mod ljm_mode;

fn main() -> Result<(), LJMError> {
    match example_env::load_example_env() {
        Ok(Some(path)) => println!("Loaded example env from {}", path.display()),
        Ok(None) => println!("No example env file found. {}", example_env::config_hint()),
        Err(err) => eprintln!("Failed to load example env: {err}"),
    }

    unsafe {
        ljm_mode::init_ljm()?;
    }

    let handle = labjack::open_labjack_from_env()?;
    let info = labjack::handle_info(handle)?;

    // Configure AIN for stream
    if matches!(info.device_type, DeviceType::T7) {
        LJMLibrary::write_name(handle, "AIN_ALL_NEGATIVE_CH", 199_u32)?;
    }
    LJMLibrary::write_name(handle, "AIN_ALL_NEGATIVE_CH", 199_u32)?; // single-ended
    LJMLibrary::write_name(handle, "AIN_ALL_RANGE", 1.0_f64)?; // ±10 V
    LJMLibrary::write_name(handle, "AIN_ALL_RESOLUTION_INDEX", 8_u32)?; // match read mode
    LJMLibrary::write_name(handle, "STREAM_SETTLING_US", 0.0_f64)?;

    println!(
        "Opened {:?} (serial {}), streaming AIN0..AIN7 — Ctrl+C to stop.",
        info.device_type, info.serial_number
    );

    // Build scan list
    let chans = vec![
        LJMLibrary::name_to_address("AIN0")?.0,
        LJMLibrary::name_to_address("AIN1")?.0,
        LJMLibrary::name_to_address("AIN2")?.0,
        LJMLibrary::name_to_address("AIN3")?.0,
        LJMLibrary::name_to_address("AIN4")?.0,
        LJMLibrary::name_to_address("AIN5")?.0,
        LJMLibrary::name_to_address("AIN6")?.0,
        LJMLibrary::name_to_address("AIN7")?.0,
    ];

    // Start stream
    let scans_per_read = 2; // small batch, similar to your per-loop reads
    let scan_rate_hz = 500.0; // per-channel sample rate with this one-pass scan list
    let actual_rate =
        LJMLibrary::stream_start(handle, scans_per_read, scan_rate_hz, chans.clone())?;
    println!("Streaming started @ {:.2} Hz", actual_rate);

    // Loop like the read version
    loop {
        let data = LJMLibrary::stream_read(handle)?;
        // Data is [AIN0, AIN1, AIN0, AIN1, ...]
        for scan in data.chunks_exact(chans.len()) {
            for (i, &val) in scan.iter().enumerate() {
                print!("AIN{:<2} = {:>8.5} V   ", i, val);
                if (i + 1) % 4 == 0 {
                    println!();
                }
            }
            println!();
        }
        thread::sleep(Duration::from_millis(200));
    }

    // LJMLibrary::stream_stop(handle)?; // unreachable under Ctrl+C
    // Ok(())
}
