//! Build script for optional static LabJack LJM linking.
//!
//! When the `staticlib` feature is enabled, this script adds common native
//! library search paths and honors `LJM_LIB_DIR`.

use std::env;
use std::path::Path;

/// Emits Cargo linker search paths for static LJM builds.
fn main() {
    if env::var_os("CARGO_FEATURE_STATICLIB").is_none() {
        return;
    }

    if let Some(lib_dir) = env::var_os("LJM_LIB_DIR") {
        let lib_dir = lib_dir.to_string_lossy();
        println!("cargo:rustc-link-search=native={lib_dir}");
    }

    for dir in ["/usr/local/lib", "/opt/homebrew/lib", "/usr/lib"] {
        if Path::new(dir).exists() {
            println!("cargo:rustc-link-search=native={dir}");
        }
    }
}
