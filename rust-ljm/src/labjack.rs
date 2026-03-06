use ljmrs::handle::{ConnectionType, DeviceHandleInfo, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_identifier(name: &str) -> Option<String> {
    env_var(name).filter(|value| !value.eq_ignore_ascii_case("ANY"))
}

fn push_unique(values: &mut Vec<String>, value: Option<String>) {
    if let Some(value) = value {
        if !values.iter().any(|existing| existing == &value) {
            values.push(value);
        }
    }
}

fn ethernet_identifiers_from_env() -> Vec<String> {
    let mut identifiers = Vec::new();
    push_unique(&mut identifiers, env_identifier("LABJACK_IDENTIFIER"));
    push_unique(&mut identifiers, env_identifier("LABJACK_SERIAL"));
    push_unique(&mut identifiers, env_identifier("LABJACK_NAME"));
    push_unique(&mut identifiers, env_identifier("LABJACK_IP"));

    if identifiers.is_empty() {
        identifiers.push("ANY".to_string());
    }

    identifiers
}

fn usb_identifiers_from_env() -> Vec<String> {
    let mut identifiers = Vec::new();
    push_unique(&mut identifiers, env_identifier("LABJACK_USB_ID"));
    push_unique(&mut identifiers, env_identifier("LABJACK_SERIAL"));

    if identifiers.is_empty() {
        identifiers.push("ANY".to_string());
    }

    identifiers
}

fn any_identifiers_from_env() -> Vec<String> {
    let mut identifiers = Vec::new();
    push_unique(&mut identifiers, env_identifier("LABJACK_IDENTIFIER"));
    push_unique(&mut identifiers, env_identifier("LABJACK_SERIAL"));
    push_unique(&mut identifiers, env_identifier("LABJACK_NAME"));
    push_unique(&mut identifiers, env_identifier("LABJACK_IP"));
    push_unique(&mut identifiers, env_identifier("LABJACK_USB_ID"));

    if identifiers.is_empty() {
        identifiers.push("ANY".to_string());
    }

    identifiers
}

fn format_attempt(mode: &str, identifier: &str) -> String {
    format!("{mode}('{identifier}')")
}

fn try_open_with_identifiers(
    mode: &str,
    connection_type: ConnectionType,
    identifiers: Vec<String>,
    failures: &mut Vec<String>,
) -> Option<i32> {
    for identifier in identifiers {
        let label = if identifier == "ANY" {
            format!("{mode} auto-discovery")
        } else {
            format!("{mode} identifier '{identifier}'")
        };
        println!("[labjack] trying {label}");

        match LJMLibrary::open_jack(DeviceType::T7, connection_type.clone(), identifier.as_str()) {
            Ok(handle) => return Some(handle),
            Err(err) => failures.push(format!("{}: {:?}", format_attempt(mode, &identifier), err)),
        }
    }

    None
}

pub fn connection_order_from_env() -> Vec<String> {
    let raw_order =
        std::env::var("LABJACK_OPEN_ORDER").unwrap_or_else(|_| "ethernet,usb".to_string());

    let mut order: Vec<String> = raw_order
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    if order.is_empty() {
        order = vec!["ethernet".to_string(), "usb".to_string()];
    }

    order
}

pub fn open_labjack_from_env() -> Result<i32, LJMError> {
    let raw_order =
        std::env::var("LABJACK_OPEN_ORDER").unwrap_or_else(|_| "ethernet,usb".to_string());
    let mut failures = Vec::new();

    for mode in connection_order_from_env() {
        let handle = match mode.as_str() {
            "ethernet" | "tcp" => try_open_with_identifiers(
                "ethernet",
                ConnectionType::ETHERNET,
                ethernet_identifiers_from_env(),
                &mut failures,
            ),
            "usb" => try_open_with_identifiers(
                "usb",
                ConnectionType::USB,
                usb_identifiers_from_env(),
                &mut failures,
            ),
            "any" => try_open_with_identifiers(
                "any",
                ConnectionType::ANY,
                any_identifiers_from_env(),
                &mut failures,
            ),
            other => {
                failures.push(format!("{other}: unsupported mode"));
                None
            }
        };

        if let Some(handle) = handle {
            return Ok(handle);
        }
    }

    Err(LJMError::LibraryError(format!(
        "Could not open LabJack. Tried LABJACK_OPEN_ORDER='{}'. Failures: {}",
        raw_order,
        failures.join(" | ")
    )))
}

pub fn handle_info(handle: i32) -> Result<DeviceHandleInfo, LJMError> {
    LJMLibrary::get_handle_info(handle)
}

pub fn handle_ip_address(info: &DeviceHandleInfo) -> Result<Option<String>, LJMError> {
    if info.ip_address == 0 {
        return Ok(None);
    }

    let ip = unsafe { LJMLibrary::number_to_ip(info.ip_address)? };
    Ok(Some(ip))
}
