use std::net::Ipv4Addr;
use std::str::FromStr;

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

fn parse_ipv4(value: &str) -> Option<Ipv4Addr> {
    Ipv4Addr::from_str(value).ok()
}

fn required_labjack_ip_from_env() -> Result<String, LJMError> {
    env_identifier("LABJACK_IP")
        .or_else(|| {
            env_identifier("LABJACK_IDENTIFIER").filter(|value| parse_ipv4(value).is_some())
        })
        .ok_or_else(|| {
            LJMError::LibraryError(
                "LABJACK_IP is required; direct Ethernet IP open is the only supported path"
                    .to_string(),
            )
        })
}

fn requested_labjack_serial_from_env() -> Option<i32> {
    env_identifier("LABJACK_SERIAL").and_then(|value| value.parse::<i32>().ok())
}

pub fn open_labjack_from_env() -> Result<i32, LJMError> {
    open_streamer_labjack_from_env()
}

pub fn open_streamer_labjack_from_env() -> Result<i32, LJMError> {
    let requested_ip = required_labjack_ip_from_env()?;
    let expected_serial = requested_labjack_serial_from_env();
    let requested_name = env_identifier("LABJACK_NAME");

    println!("[labjack] trying ethernet identifier '{requested_ip}'");
    if let Some(name) = requested_name.as_deref() {
        println!("[labjack] requested logical device name '{name}'");
    }

    let handle =
        LJMLibrary::open_jack(DeviceType::T7, ConnectionType::ETHERNET, requested_ip.as_str())
            .map_err(|err| {
                LJMError::LibraryError(format!(
                    "Could not open LabJack via LABJACK_IP='{}': {:?}",
                    requested_ip, err
                ))
            })?;

    let verification = (|| -> Result<DeviceHandleInfo, LJMError> {
        let info = handle_info(handle).map_err(|err| {
            LJMError::LibraryError(format!(
                "Opened LabJack at '{}' but failed to read handle info: {:?}",
                requested_ip, err
            ))
        })?;

        if !matches!(info.device_type, DeviceType::T7) {
            return Err(LJMError::LibraryError(format!(
                "Connected device at '{}' is not a T7: {:?}",
                requested_ip, info.device_type
            )));
        }

        let actual_ip = handle_ip_address(&info)?.unwrap_or_else(|| "N/A".to_string());
        if actual_ip != "N/A" && actual_ip != requested_ip {
            return Err(LJMError::LibraryError(format!(
                "Connected device IP mismatch: requested '{}', got '{}'",
                requested_ip, actual_ip
            )));
        }

        if let Some(expected_serial) = expected_serial {
            if info.serial_number != expected_serial {
                return Err(LJMError::LibraryError(format!(
                    "Connected LabJack serial mismatch at '{}': expected {}, got {}",
                    requested_ip, expected_serial, info.serial_number
                )));
            }
        }

        let settling_us = LJMLibrary::read_name(handle, "STREAM_SETTLING_US").map_err(|err| {
            LJMError::LibraryError(format!(
                "LabJack self-test read failed for '{}': STREAM_SETTLING_US: {:?}",
                requested_ip, err
            ))
        })?;

        LJMLibrary::write_name(handle, "STREAM_SETTLING_US", settling_us).map_err(|err| {
            LJMError::LibraryError(format!(
                "LabJack self-test write failed for '{}': STREAM_SETTLING_US={}: {:?}",
                requested_ip, settling_us, err
            ))
        })?;

        println!(
            "[labjack] connected via {:?}, serial {}, ip {}, self-test ok",
            info.connection_type, info.serial_number, actual_ip
        );

        Ok(info)
    })();

    if let Err(err) = verification {
        let _ = LJMLibrary::close_jack(handle);
        return Err(err);
    }

    Ok(handle)
}

pub fn handle_info(handle: i32) -> Result<DeviceHandleInfo, LJMError> {
    LJMLibrary::get_handle_info(handle)
}

#[allow(dead_code)]
pub fn handle_ip_address(info: &DeviceHandleInfo) -> Result<Option<String>, LJMError> {
    if info.ip_address == 0 {
        return Ok(None);
    }

    let ip = Ipv4Addr::from(info.ip_address as u32);
    Ok(Some(ip.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_ip_address_converts_signed_ipv4_bits() {
        let info = DeviceHandleInfo {
            device_type: DeviceType::T7,
            connection_type: ConnectionType::ETHERNET,
            ip_address: -1062731418,
            max_bytes_per_megabyte: 0,
            serial_number: 0,
            port: 0,
        };

        let ip = handle_ip_address(&info).expect("conversion should succeed");
        assert_eq!(ip.as_deref(), Some("192.168.1.102"));
    }

    #[test]
    fn handle_ip_address_returns_none_for_zero() {
        let info = DeviceHandleInfo {
            device_type: DeviceType::T7,
            connection_type: ConnectionType::ETHERNET,
            ip_address: 0,
            max_bytes_per_megabyte: 0,
            serial_number: 0,
            port: 0,
        };

        let ip = handle_ip_address(&info).expect("zero should be treated as missing");
        assert_eq!(ip, None);
    }
}
