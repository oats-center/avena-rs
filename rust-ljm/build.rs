use std::env;
use std::path::Path;

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
