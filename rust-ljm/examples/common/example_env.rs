use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub fn load_example_env() -> Result<Option<PathBuf>, String> {
    let Some(config_path) = find_config_path() else {
        return Ok(None);
    };

    let raw = fs::read_to_string(&config_path)
        .map_err(|e| format!("failed to read {}: {e}", config_path.display()))?;
    let json: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse {}: {e}", config_path.display()))?;
    let env_obj = json
        .get("env")
        .unwrap_or(&json)
        .as_object()
        .ok_or_else(|| format!("{} must contain an object or env object", config_path.display()))?;

    for (key, value) in env_obj {
        let value = match value {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            _ => value.to_string(),
        };
        unsafe {
            std::env::set_var(key, value);
        }
    }

    Ok(Some(config_path))
}

fn find_config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CONFIG_FILE") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    for candidate in [
        PathBuf::from("streamer.env.json"),
        PathBuf::from("rust-ljm/streamer.env.json"),
        PathBuf::from("../rust-ljm/streamer.env.json"),
    ] {
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

pub fn config_hint() -> &'static str {
    "Set CONFIG_FILE=/path/to/streamer.env.json or export LABJACK_IP directly."
}
