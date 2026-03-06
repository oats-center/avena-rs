use ljmrs::handle::{ConnectionType, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

#[path = "../src/ljm_mode.rs"]
mod ljm_mode;

fn main() -> Result<(), LJMError> {
    unsafe {
        ljm_mode::init_ljm()?;
    }
    let handle = LJMLibrary::open_jack(DeviceType::ANY, ConnectionType::ANY, "ANY")?;
    match LJMLibrary::stream_stop(handle) {
        Ok(_) => println!("Stopped active stream."),
        Err(e) => println!("stream_stop returned: {:?}", e),
    }
    LJMLibrary::close_jack(handle)?;
    Ok(())
}
