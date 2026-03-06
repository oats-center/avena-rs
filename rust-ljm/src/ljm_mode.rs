use ljmrs::{LJMError, LJMLibrary};

#[cfg(all(feature = "dynlink", feature = "staticlib"))]
compile_error!(
    "Choose only one LJM mode. Use the default `dynlink`, or `--no-default-features --features staticlib`."
);

#[cfg(all(not(feature = "dynlink"), not(feature = "staticlib")))]
compile_error!("Enable one LJM mode: `dynlink` or `staticlib`.");

pub unsafe fn init_ljm() -> Result<(), LJMError> {
    #[cfg(all(feature = "dynlink", not(feature = "staticlib")))]
    {
        let path = std::env::var("LJM_PATH").ok();
        unsafe { LJMLibrary::init(path) }
    }

    #[cfg(all(feature = "staticlib", not(feature = "dynlink")))]
    {
        unsafe { LJMLibrary::init() }
    }
}
