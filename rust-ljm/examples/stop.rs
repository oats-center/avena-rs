use ljmrs::handle::{ConnectionType, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

fn open_labjack_with_fallback(device_type: DeviceType) -> Result<i32, LJMError> {
    let lj_ip = std::env::var("LABJACK_IP").ok();
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
                let Some(lj_ip) = lj_ip.as_deref() else {
                    errors.push("ethernet: LABJACK_IP not set".to_string());
                    continue;
                };
                match LJMLibrary::open_jack(device_type, ConnectionType::ETHERNET, lj_ip) {
                    Ok(handle) => return Ok(handle),
                    Err(e) => errors.push(format!("ethernet({}): {:?}", lj_ip, e)),
                }
            }
            "usb" => match LJMLibrary::open_jack(device_type, ConnectionType::USB, usb_id.as_str())
            {
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
    match LJMLibrary::stream_stop(handle) {
        Ok(_) => println!("Stopped active stream."),
        Err(e) => println!("stream_stop returned: {:?}", e),
    }
    LJMLibrary::close_jack(handle)?;
    Ok(())
}
