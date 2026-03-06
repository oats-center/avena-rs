use ljmrs::{ConnectionType, DeviceType, LJMLibrary};

#[path = "../src/ljm_mode.rs"]
mod ljm_mode;

fn open_labjack_from_env() -> Result<i32, ljmrs::LJMError> {
    let labjack_ip = std::env::var("LABJACK_IP").unwrap_or_else(|_| "10.165.77.233".to_string());
    let usb_id = std::env::var("LABJACK_USB_ID").unwrap_or_else(|_| "ANY".to_string());
    let raw_order =
        std::env::var("LABJACK_OPEN_ORDER").unwrap_or_else(|_| "ethernet,usb".to_string());

    let mut order: Vec<String> = raw_order
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if order.is_empty() {
        order = vec!["ethernet".to_string(), "usb".to_string()];
    }

    let mut failures = Vec::new();
    for mode in order {
        let attempt = match mode.as_str() {
            "ethernet" | "tcp" => {
                println!(
                    "[labjack] trying ETHERNET/TCP with identifier '{}'",
                    labjack_ip
                );
                LJMLibrary::open_jack(
                    DeviceType::T7,
                    ConnectionType::ETHERNET,
                    labjack_ip.as_str(),
                )
            }
            "usb" => {
                println!("[labjack] trying USB with identifier '{}'", usb_id);
                LJMLibrary::open_jack(DeviceType::T7, ConnectionType::USB, usb_id.as_str())
            }
            "any" => {
                println!("[labjack] trying ANY transport");
                LJMLibrary::open_jack(DeviceType::T7, ConnectionType::ANY, "ANY")
            }
            other => {
                failures.push(format!("{}: unsupported mode", other));
                continue;
            }
        };

        match attempt {
            Ok(handle) => return Ok(handle),
            Err(err) => failures.push(format!("{}: {:?}", mode, err)),
        }
    }

    Err(ljmrs::LJMError::LibraryError(format!(
        "Could not open LabJack. Tried LABJACK_OPEN_ORDER='{}'. Failures: {}",
        raw_order,
        failures.join(" | ")
    )))
}

fn main() {
    unsafe {
        ljm_mode::init_ljm().expect("Failed to init LJM");
    }

    let handle = open_labjack_from_env().expect("Could not open LabJack");

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
