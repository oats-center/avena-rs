use std::ffi::CString;
use std::fmt::{Display, Formatter};

#[cfg(feature = "dynlink")]
use libloading::{Library, Symbol};
use ljmrs::handle::{ConnectionType, DeviceHandleInfo, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

const LJM_LIST_ALL_SIZE: usize = 128;

#[cfg(all(feature = "staticlib", not(feature = "dynlink")))]
unsafe extern "C" {
    fn LJM_ListAllS(
        device_type: *const std::os::raw::c_char,
        connection_type: *const std::os::raw::c_char,
        num_found: *mut i32,
        device_types: *mut i32,
        connection_types: *mut i32,
        serial_numbers: *mut i32,
        ip_addresses: *mut i32,
    ) -> i32;
}

#[derive(Clone, Debug)]
pub struct DiscoveredLabJack {
    pub device_type: DeviceType,
    pub connection_type: ConnectionType,
    pub serial_number: i32,
    pub ip_address: Option<String>,
}

impl Display for DiscoveredLabJack {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.ip_address {
            Some(ip) => write!(
                f,
                "{:?} via {:?}, serial {}, ip {}",
                self.device_type, self.connection_type, self.serial_number, ip
            ),
            None => write!(
                f,
                "{:?} via {:?}, serial {}",
                self.device_type, self.connection_type, self.serial_number
            ),
        }
    }
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn ljm_result(error_code: i32) -> Result<(), LJMError> {
    if error_code == 0 {
        return Ok(());
    }

    let message = LJMLibrary::error_to_string(error_code)
        .unwrap_or_else(|_| format!("LJM error code {error_code}"));
    Err(LJMError::ErrorCode(error_code.into(), message))
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
        push_discovered_serials(
            &mut identifiers,
            discover_labjacks(ConnectionType::ETHERNET).ok(),
        );
    }

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
        push_discovered_serials(
            &mut identifiers,
            discover_labjacks(ConnectionType::USB).ok(),
        );
    }

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
        push_discovered_serials(
            &mut identifiers,
            discover_labjacks(ConnectionType::ANY).ok(),
        );
    }

    if identifiers.is_empty() {
        identifiers.push("ANY".to_string());
    }

    identifiers
}

fn push_discovered_serials(
    identifiers: &mut Vec<String>,
    discovered: Option<Vec<DiscoveredLabJack>>,
) {
    if let Some(discovered) = discovered {
        for device in discovered {
            let serial = device.serial_number.to_string();
            if !identifiers.iter().any(|existing| existing == &serial) {
                identifiers.push(serial);
            }
        }
    }
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

fn discovery_target(connection_type: &ConnectionType) -> &'static str {
    match connection_type {
        ConnectionType::USB => "USB",
        ConnectionType::ETHERNET => "ETHERNET",
        ConnectionType::WIFI => "WIFI",
        ConnectionType::ANY | ConnectionType::UNKNOWN(_) => "ANY",
    }
}

#[cfg(feature = "dynlink")]
fn call_list_all(
    device_type: &CString,
    connection_type: &CString,
    num_found: &mut i32,
    device_types: &mut [i32; LJM_LIST_ALL_SIZE],
    connection_types: &mut [i32; LJM_LIST_ALL_SIZE],
    serial_numbers: &mut [i32; LJM_LIST_ALL_SIZE],
    ip_addresses: &mut [i32; LJM_LIST_ALL_SIZE],
) -> Result<(), LJMError> {
    type ListAllS = unsafe extern "C" fn(
        *const std::os::raw::c_char,
        *const std::os::raw::c_char,
        *mut i32,
        *mut i32,
        *mut i32,
        *mut i32,
        *mut i32,
    ) -> i32;

    let library_path = std::env::var("LJM_PATH").unwrap_or_else(|_| LJMLibrary::get_library_path());
    let library = unsafe { Library::new(&library_path) }.map_err(|e| {
        LJMError::LibraryError(format!("Failed to load LJM library '{library_path}': {e}"))
    })?;
    let list_all: Symbol<ListAllS> = unsafe { library.get(b"LJM_ListAllS") }
        .map_err(|e| LJMError::LibraryError(format!("Failed to load LJM_ListAllS: {e}")))?;

    let error_code = unsafe {
        list_all(
            device_type.as_ptr(),
            connection_type.as_ptr(),
            num_found,
            device_types.as_mut_ptr(),
            connection_types.as_mut_ptr(),
            serial_numbers.as_mut_ptr(),
            ip_addresses.as_mut_ptr(),
        )
    };

    ljm_result(error_code)
}

#[cfg(all(feature = "staticlib", not(feature = "dynlink")))]
fn call_list_all(
    device_type: &CString,
    connection_type: &CString,
    num_found: &mut i32,
    device_types: &mut [i32; LJM_LIST_ALL_SIZE],
    connection_types: &mut [i32; LJM_LIST_ALL_SIZE],
    serial_numbers: &mut [i32; LJM_LIST_ALL_SIZE],
    ip_addresses: &mut [i32; LJM_LIST_ALL_SIZE],
) -> Result<(), LJMError> {
    let error_code = unsafe {
        LJM_ListAllS(
            device_type.as_ptr(),
            connection_type.as_ptr(),
            num_found,
            device_types.as_mut_ptr(),
            connection_types.as_mut_ptr(),
            serial_numbers.as_mut_ptr(),
            ip_addresses.as_mut_ptr(),
        )
    };

    ljm_result(error_code)
}

pub fn discover_labjacks(
    connection_type: ConnectionType,
) -> Result<Vec<DiscoveredLabJack>, LJMError> {
    let device_type = CString::new("T7")
        .map_err(|_| LJMError::LibraryError("Invalid device type".to_string()))?;
    let connection_type_name = CString::new(discovery_target(&connection_type))
        .map_err(|_| LJMError::LibraryError("Invalid connection type".to_string()))?;

    let mut num_found = 0;
    let mut device_types = [0i32; LJM_LIST_ALL_SIZE];
    let mut connection_types = [0i32; LJM_LIST_ALL_SIZE];
    let mut serial_numbers = [0i32; LJM_LIST_ALL_SIZE];
    let mut ip_addresses = [0i32; LJM_LIST_ALL_SIZE];

    call_list_all(
        &device_type,
        &connection_type_name,
        &mut num_found,
        &mut device_types,
        &mut connection_types,
        &mut serial_numbers,
        &mut ip_addresses,
    )?;

    let count = num_found.clamp(0, LJM_LIST_ALL_SIZE as i32) as usize;
    let mut devices = Vec::with_capacity(count);

    for index in 0..count {
        let ip_address = if ip_addresses[index] == 0 {
            None
        } else {
            Some(unsafe { LJMLibrary::number_to_ip(ip_addresses[index])? })
        };

        devices.push(DiscoveredLabJack {
            device_type: DeviceType::from(device_types[index]),
            connection_type: ConnectionType::from(connection_types[index]),
            serial_number: serial_numbers[index],
            ip_address,
        });
    }

    Ok(devices)
}

fn discovery_modes_from_env() -> Vec<ConnectionType> {
    let mut modes = Vec::new();

    for mode in connection_order_from_env() {
        let connection_type = match mode.as_str() {
            "ethernet" | "tcp" => ConnectionType::ETHERNET,
            "usb" => ConnectionType::USB,
            "any" => ConnectionType::ANY,
            _ => continue,
        };

        if !modes
            .iter()
            .any(|existing| discovery_target(existing) == discovery_target(&connection_type))
        {
            modes.push(connection_type);
        }
    }

    modes
}

pub fn log_discovery_snapshot() {
    for connection_type in discovery_modes_from_env() {
        match discover_labjacks(connection_type.clone()) {
            Ok(devices) if devices.is_empty() => {
                println!(
                    "[labjack] discovery found no T7 devices over {}",
                    discovery_target(&connection_type)
                );
            }
            Ok(devices) => {
                println!(
                    "[labjack] discovery found {} T7 device(s) over {}",
                    devices.len(),
                    discovery_target(&connection_type)
                );
                for device in devices {
                    println!("[labjack]   {device}");
                }
            }
            Err(err) => {
                eprintln!(
                    "[labjack] discovery failed over {}: {:?}",
                    discovery_target(&connection_type),
                    err
                );
            }
        }
    }
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
