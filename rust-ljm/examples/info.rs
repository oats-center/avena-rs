use ljmrs::LJMLibrary;

#[path = "common/example_env.rs"]
mod example_env;
#[path = "../src/labjack.rs"]
mod labjack;
#[path = "../src/ljm_mode.rs"]
mod ljm_mode;

fn main() {
    match example_env::load_example_env() {
        Ok(Some(path)) => println!("Loaded example env from {}", path.display()),
        Ok(None) => println!("No example env file found. {}", example_env::config_hint()),
        Err(err) => eprintln!("Failed to load example env: {err}"),
    }

    unsafe {
        ljm_mode::init_ljm().expect("Failed to init LJM");
    }

    let handle = labjack::open_labjack_from_env().expect("Could not open LabJack");

    println!("Opened LabJack, got handle: {}", handle);

    let info = labjack::handle_info(handle).expect("Handle verification failed.");
    let ip_addr = labjack::handle_ip_address(&info)
        .expect("Could not resolve LabJack IP")
        .unwrap_or_else(|| "N/A".to_string());

    println!("Device Type: {:?}", info.device_type);
    println!("IP Address: {}", ip_addr);
    println!("Port: {}", info.port);
    println!("Connection Type: {:?}", info.connection_type);
    println!("Max Bytes per Megabyte: {}", info.max_bytes_per_megabyte);

    LJMLibrary::close_jack(handle).expect("Failed to close LabJack");
}
