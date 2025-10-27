use ljmrs::handle::{ConnectionType, DeviceType};
use ljmrs::{LJMError, LJMLibrary};
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
    let handle = LJMLibrary::open_jack(DeviceType::ANY, ConnectionType::ANY, "ANY")?;
    match LJMLibrary::stream_stop(handle) {
        Ok(_) => println!("Stopped active stream."),
        Err(e) => println!("stream_stop returned: {:?}", e),
    }
    LJMLibrary::close_jack(handle)?;
    Ok(())
}
