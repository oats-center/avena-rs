//! Stops an active LabJack stream without running the normal self-test path.
//!
//! Use this helper when a previous process left a stream active and the normal
//! examples or streamer cannot configure the device cleanly.

use ljmrs::handle::{ConnectionType, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

#[path = "common/example_env.rs"]
mod example_env;
#[path = "../src/ljm_mode.rs"]
mod ljm_mode;

/// Reads the direct Ethernet LabJack IP required by this recovery helper.
fn labjack_ip_from_env() -> Result<String, LJMError> {
    std::env::var("LABJACK_IP")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LJMError::LibraryError("LABJACK_IP is required".to_string()))
}

/// Opens the configured LabJack and sends `stream_stop`.
fn main() -> Result<(), LJMError> {
    match example_env::load_example_env() {
        Ok(Some(path)) => println!("Loaded example env from {}", path.display()),
        Ok(None) => println!("No example env file found. {}", example_env::config_hint()),
        Err(err) => eprintln!("Failed to load example env: {err}"),
    }

    unsafe {
        ljm_mode::init_ljm()?;
    }
    let labjack_ip = labjack_ip_from_env()?;
    println!("Opening LabJack at {labjack_ip} without self-test for stream_stop.");
    let handle = LJMLibrary::open_jack(DeviceType::T7, ConnectionType::ETHERNET, labjack_ip)?;
    match LJMLibrary::stream_stop(handle) {
        Ok(_) => println!("Stopped active stream."),
        Err(e) => println!("stream_stop returned: {:?}", e),
    }
    LJMLibrary::close_jack(handle)?;
    Ok(())
}
