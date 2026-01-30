use ljmrs::{ConnectionType, DeviceType, LJMLibrary};

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

   let lj_ip = std::env::var("LABJACK_IP")
        .unwrap_or_else(|_| "10.165.77.233".to_string());
    
    let handle = LJMLibrary::open_jack(
        DeviceType::T7,
        ConnectionType::ETHERNET,
        lj_ip.as_str(),
    ).expect("Could not open LabJack");
    
    

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
