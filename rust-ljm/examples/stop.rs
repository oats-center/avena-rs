use ljmrs::{LJMError, LJMLibrary};

#[path = "common/example_env.rs"]
mod example_env;
#[path = "../src/labjack.rs"]
mod labjack;
#[path = "../src/ljm_mode.rs"]
mod ljm_mode;

fn main() -> Result<(), LJMError> {
    match example_env::load_example_env() {
        Ok(Some(path)) => println!("Loaded example env from {}", path.display()),
        Ok(None) => println!("No example env file found. {}", example_env::config_hint()),
        Err(err) => eprintln!("Failed to load example env: {err}"),
    }

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
