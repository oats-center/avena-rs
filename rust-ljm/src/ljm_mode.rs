//! LabJack LJM library initialization for the selected linking mode.
//!
//! The crate is built with exactly one of the `dynlink` or `staticlib` features.
//! Dynamic mode can load a library path from `LJM_PATH`; static mode initializes
//! the linked library directly.

use ljmrs::{LJMError, LJMLibrary};

#[cfg(all(feature = "dynlink", feature = "staticlib"))]
compile_error!(
    "Choose only one LJM mode. Use the default `dynlink`, or `--no-default-features --features staticlib`."
);

#[cfg(all(not(feature = "dynlink"), not(feature = "staticlib")))]
compile_error!("Enable one LJM mode: `dynlink` or `staticlib`.");

/// Initializes the LabJack LJM library for the configured feature mode.
///
/// # Safety
///
/// This delegates to `ljmrs::LJMLibrary::init`, which is unsafe because it
/// initializes process-wide FFI library state. Call it once during service
/// startup before using other LJM operations.
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
