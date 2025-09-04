use std::thread;
use std::time::Duration;
use ljmrs::{LJMLibrary, LJMError};
use ljmrs::handle::{ConnectionType, DeviceType};

fn main() -> Result<(), LJMError> {
    // Choose one feature at build time
    #[cfg(all(feature = "dynlink", not(feature = "staticlib")))]
    unsafe {
        let path = std::env::var("LJM_PATH").ok();
        LJMLibrary::init(path)?;
    }
    #[cfg(all(feature = "staticlib", not(feature = "dynlink")))]
    unsafe {
        LJMLibrary::init()?;
    }

    let handle = LJMLibrary::open_jack(DeviceType::ANY, ConnectionType::ANY, "ANY")?;

    let info = LJMLibrary::get_handle_info(handle)?;
    let num_ains = match info.device_type {
        DeviceType::T4 => 12, // AIN0–AIN11
        DeviceType::T8 => 8,  // AIN0–AIN7
        DeviceType::T7 => 14, // AIN0–AIN13
        _ => 14,
    };

    if matches!(info.device_type, DeviceType::T7) {
        // 199 = single-ended
        LJMLibrary::write_name(handle, "AIN_ALL_NEGATIVE_CH", 199_u32)?;
    }
    LJMLibrary::write_name(handle, "AIN_ALL_RANGE", 1.0_f64)?;         // ±10 V (±11 V on T8)
    LJMLibrary::write_name(handle, "AIN_ALL_RESOLUTION_INDEX", 0_u32)?; // default

    println!(
        "Opened {:?} (serial {}), reading AIN0..AIN{} — Ctrl+C to stop.",
        info.device_type, info.serial_number, num_ains - 1
    );

    loop {
        for ch in 0..14 {
            let name = format!("AIN{}", ch);
            let v: f64 = LJMLibrary::read_name(handle, name)?; // move the String
            print!("AIN{:<2} = {:>8.5} V   ", ch, v);
            if (ch + 1) % 4 == 0 {
                println!();
            }
        }

        println!();
        thread::sleep(Duration::from_millis(200)); // ~5 Hz
    }

    // LJMLibrary::close_jack(handle)?; // unreachable under Ctrl+C
    // Ok(())
}
