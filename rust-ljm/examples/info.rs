use ljmrs::{ConnectionType, DeviceType, LJMLibrary};

fn open_labjack_with_fallback(device_type: DeviceType) -> i32 {
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

    let mut errors = Vec::new();
    for mode in modes {
        match mode.as_str() {
            "ethernet" | "tcp" => {
                match LJMLibrary::open_jack(device_type, ConnectionType::ETHERNET, lj_ip.as_str()) {
                    Ok(handle) => return handle,
                    Err(e) => errors.push(format!("ethernet({}): {:?}", lj_ip, e)),
                }
            }
            "usb" => match LJMLibrary::open_jack(device_type, ConnectionType::USB, usb_id.as_str()) {
                Ok(handle) => return handle,
                Err(e) => errors.push(format!("usb({}): {:?}", usb_id, e)),
            },
            "any" => match LJMLibrary::open_jack(device_type, ConnectionType::ANY, "ANY") {
                Ok(handle) => return handle,
                Err(e) => errors.push(format!("any: {:?}", e)),
            },
            other => errors.push(format!("unsupported mode '{}'", other)),
        }
    }

    panic!(
        "Could not open LabJack with order '{}': {}",
        order,
        errors.join(" | ")
    );
}

fn main() {
    #[cfg(feature = "staticlib")]
    unsafe {
        LJMLibrary::init().expect("Failed to init LJM (static)");
    }

    // If you enable dynlink instead:
    // #[cfg(all(feature = "dynlink", not(feature = "staticlib")))]
    // unsafe {
    //     let path = std::env::var("LJM_PATH").ok();
    //     LJMLibrary::init(path).expect("Failed to init LJM (dynlink)");
    // }

    let handle = open_labjack_with_fallback(DeviceType::T7);

    println!("Opened LabJack, got handle: {}", handle);

    let info = LJMLibrary::get_handle_info(handle).expect("Handle verification failed.");

    // Prefer direct conversion from u32 (network order)
    let ip_addr = std::net::Ipv4Addr::from(info.ip_address as u32);

    println!("Device Type: {:?}", info.device_type);
    println!("IP Address: {}", ip_addr);
    println!("Port: {}", info.port);
    println!("Connection Type: {:?}", info.connection_type);
    println!("Max Bytes per Megabyte: {}", info.max_bytes_per_megabyte);
}
