use ljmrs::handle::{ConnectionType, DeviceType};
use ljmrs::{LJMError, LJMLibrary};
use std::thread;
use std::time::Duration;

fn open_labjack_with_fallback(device_type: DeviceType) -> Result<i32, LJMError> {
    let lj_ip = std::env::var("LABJACK_IP").unwrap_or_else(|_| "10.165.77.233".to_string());
    let usb_id = std::env::var("LABJACK_USB_ID").unwrap_or_else(|_| "ANY".to_string());
    let order = std::env::var("LABJACK_OPEN_ORDER").unwrap_or_else(|_| "ethernet,usb".to_string());

    let mut modes: Vec<String> = order
        .split(',')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect();

    if modes.is_empty() {
        modes = vec!["ethernet".to_string(), "usb".to_string()];
    }

    let mut errors: Vec<String> = Vec::new();
    for mode in modes {
        match mode.as_str() {
            "ethernet" | "tcp" => {
                match LJMLibrary::open_jack(device_type, ConnectionType::ETHERNET, lj_ip.as_str()) {
                    Ok(handle) => return Ok(handle),
                    Err(e) => errors.push(format!("ethernet({}): {:?}", lj_ip, e)),
                }
            }
            "usb" => match LJMLibrary::open_jack(device_type, ConnectionType::USB, usb_id.as_str()) {
                Ok(handle) => return Ok(handle),
                Err(e) => errors.push(format!("usb({}): {:?}", usb_id, e)),
            },
            "any" => match LJMLibrary::open_jack(device_type, ConnectionType::ANY, "ANY") {
                Ok(handle) => return Ok(handle),
                Err(e) => errors.push(format!("any: {:?}", e)),
            },
            other => errors.push(format!("unsupported mode '{}'", other)),
        }
    }

    Err(LJMError::LibraryError(format!(
        "Could not open LabJack with order '{}': {}",
        order,
        errors.join(" | ")
    )))
}

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

    let handle = open_labjack_with_fallback(DeviceType::ANY)?;

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
    LJMLibrary::write_name(handle, "AIN_ALL_RANGE", 1.0_f64)?; // ±10 V (±11 V on T8)
    LJMLibrary::write_name(handle, "AIN_ALL_RESOLUTION_INDEX", 0_u32)?; // default

    println!(
        "Opened {:?} (serial {}), reading AIN0..AIN{} — Ctrl+C to stop.",
        info.device_type,
        info.serial_number,
        num_ains - 1
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
