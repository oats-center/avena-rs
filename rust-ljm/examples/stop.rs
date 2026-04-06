use ljmrs::{LJMError, LJMLibrary};

#[path = "../src/labjack.rs"]
mod labjack;
#[path = "../src/ljm_mode.rs"]
mod ljm_mode;

fn main() -> Result<(), LJMError> {
    unsafe {
        ljm_mode::init_ljm()?;
    }
    let handle = labjack::open_labjack_from_env()?;
    match LJMLibrary::stream_stop(handle) {
        Ok(_) => println!("Stopped active stream."),
        Err(e) => println!("stream_stop returned: {:?}", e),
    }
    LJMLibrary::close_jack(handle)?;
    Ok(())
}
