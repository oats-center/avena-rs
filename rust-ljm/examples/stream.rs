use std::thread;
use std::time::Duration;
use ljmrs::{LJMLibrary, LJMError};
use ljmrs::handle::{ConnectionType, DeviceType};

fn main() -> Result<(), LJMError> {
    // Init LJM
    #[cfg(all(feature = "dynlink", not(feature = "staticlib")))]
    unsafe {
        let path = std::env::var("LJM_PATH").ok();
        LJMLibrary::init(path)?;
    }
    #[cfg(all(feature = "staticlib", not(feature = "dynlink")))]
    unsafe {
        LJMLibrary::init()?;
    }

    // Open device
    let handle = LJMLibrary::open_jack(DeviceType::ANY, ConnectionType::ANY, "ANY")?;
    let info = LJMLibrary::get_handle_info(handle)?;
    

    // Configure AIN for stream
    if matches!(info.device_type, DeviceType::T7) {
        LJMLibrary::write_name(handle, "AIN_ALL_NEGATIVE_CH", 199_u32)?;
    }
    LJMLibrary::write_name(handle, "AIN_ALL_NEGATIVE_CH", 199_u32)?; // single-ended
    LJMLibrary::write_name(handle, "AIN_ALL_RANGE", 1.0_f64)?;      // ±10 V
    LJMLibrary::write_name(handle, "AIN_ALL_RESOLUTION_INDEX", 8_u32)?; // match read mode
    LJMLibrary::write_name(handle, "STREAM_SETTLING_US", 0.0_f64)?;

    println!(
        "Opened {:?} (serial {}), streaming AIN0..AIN1 — Ctrl+C to stop.",
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
    let scan_rate = 5.0;    // ~same as your 200 ms delay (5 Hz)
    let actual_rate = LJMLibrary::stream_start(handle, scans_per_read, scan_rate, chans.clone())?;
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
