//! LabJack T7 connection and handle-inspection helpers for the `streamer` binary.
//!
//! The streamer only supports direct Ethernet opens by IP address; it never runs LJM
//! device discovery. These helpers handle environment parsing, device verification,
//! stale stream cleanup, and conversion of LabJack handle metadata into readable
//! values.
//!
//! # Configuration
//!
//! * `LABJACK_IP` - IPv4 address of the T7. Required unless `LABJACK_IDENTIFIER`
//!   holds an IPv4 address.
//! * `LABJACK_IDENTIFIER` - Fallback for `LABJACK_IP`, used only when it parses as
//!   an IPv4 address.
//! * `LABJACK_SERIAL` - Expected serial number. When set, a device with another
//!   serial is rejected. `ANY` or a non-numeric value disables the check.
//! * `LABJACK_NAME` - Logical device name. Only logged.
//!
//! For all of these, surrounding whitespace is trimmed, and an empty value or `ANY`
//! (any case) counts as unset.

use std::net::Ipv4Addr;
use std::str::FromStr;

use ljmrs::handle::{ConnectionType, DeviceHandleInfo, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

/// LJM error code returned when a stream is already active on the handle
/// (`STREAM_IS_ACTIVE`).
///
/// It shows up when an earlier session left a stream running on the device. The
/// self-test write in [`open_streamer_labjack_from_env`] then fails with this code
/// until the stream is stopped.
const STREAM_IS_ACTIVE_ERROR: i32 = 2605;

/// Returns a trimmed environment variable value when set and non-empty.
///
/// # Arguments
///
/// * `name` - Environment variable name.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Returns an environment identifier unless it is empty or the wildcard `ANY`.
///
/// The `ANY` check ignores case.
///
/// # Arguments
///
/// * `name` - Environment variable name.
fn env_identifier(name: &str) -> Option<String> {
    env_var(name).filter(|value| !value.eq_ignore_ascii_case("ANY"))
}

/// Parses an IPv4 string used by the direct Ethernet LabJack path.
///
/// # Arguments
///
/// * `value` - Dotted-quad text, for example `192.168.1.102`.
///
/// # Returns
///
/// The address, or `None` if the text is not a valid IPv4 address.
fn parse_ipv4(value: &str) -> Option<Ipv4Addr> {
    Ipv4Addr::from_str(value).ok()
}

/// Reads the required LabJack IP address from environment variables.
///
/// `LABJACK_IP` is preferred. `LABJACK_IDENTIFIER` is accepted only when it is
/// an IPv4 address because this deployment path avoids broad discovery.
/// `LABJACK_IP` itself is not validated here.
///
/// # Errors
///
/// Returns `LJMError::LibraryError` if neither variable yields an address.
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

/// Reads an optional expected LabJack serial number from `LABJACK_SERIAL`.
///
/// # Returns
///
/// The serial number, or `None` if the variable is unset, `ANY`, or not an `i32`.
/// A value that does not parse is ignored rather than reported.
fn requested_labjack_serial_from_env() -> Option<i32> {
    env_identifier("LABJACK_SERIAL").and_then(|value| value.parse::<i32>().ok())
}

/// Extracts the numeric LJM code from an `LJMError` when present.
///
/// # Arguments
///
/// * `err` - Error returned by an `ljmrs` call.
///
/// # Returns
///
/// The code for `LJMError::ErrorCode`, or `None` for other variants.
fn ljm_error_code(err: &LJMError) -> Option<i32> {
    match err {
        LJMError::ErrorCode(code, _) => Some(code.into()),
        _ => None,
    }
}

/// Returns true when an LJM error indicates a stale active stream.
///
/// # Arguments
///
/// * `err` - Error returned by an `ljmrs` call.
fn is_stream_active_error(err: &LJMError) -> bool {
    ljm_error_code(err) == Some(STREAM_IS_ACTIVE_ERROR)
}

/// Opens the configured LabJack for the default streamer path.
///
/// Calls [`open_streamer_labjack_from_env`].
///
/// # Returns
///
/// The LJM device handle.
///
/// # Errors
///
/// Same as [`open_streamer_labjack_from_env`].
pub fn open_labjack_from_env() -> Result<i32, LJMError> {
    open_streamer_labjack_from_env()
}

/// Opens, verifies, and self-tests the LabJack configured by environment.
///
/// Opens a T7 over Ethernet at the configured IP address, then checks the device
/// type, the IP reported by the handle (skipped when LJM reports none), and the
/// optional serial number. As a self-test it reads `STREAM_SETTLING_US` and writes
/// the same value back. If a stale active stream blocks the write, it sends
/// `stream_stop` once and retries. If any check fails, the handle is closed before
/// the error is returned.
///
/// # Returns
///
/// The LJM device handle. The caller owns it and must close it.
///
/// # Errors
///
/// Returns `LJMError::LibraryError` if no IP address is configured, the device
/// cannot be opened, the handle info cannot be read, the device is not a T7, its
/// IP or serial number does not match, or the self-test read or write fails
/// (including after `stream_stop`, or if `stream_stop` itself fails).
pub fn open_streamer_labjack_from_env() -> Result<i32, LJMError> {
    let requested_ip = required_labjack_ip_from_env()?;
    let expected_serial = requested_labjack_serial_from_env();
    let requested_name = env_identifier("LABJACK_NAME");

    println!("[labjack] trying ethernet identifier '{requested_ip}'");
    if let Some(name) = requested_name.as_deref() {
        println!("[labjack] requested logical device name '{name}'");
    }

    let handle = LJMLibrary::open_jack(
        DeviceType::T7,
        ConnectionType::ETHERNET,
        requested_ip.as_str(),
    )
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

        match LJMLibrary::write_name(handle, "STREAM_SETTLING_US", settling_us) {
            Ok(_) => {}
            Err(err) if is_stream_active_error(&err) => {
                eprintln!(
                    "[labjack] stale active stream detected on '{}'; sending stream_stop and retrying self-test",
                    requested_ip
                );
                LJMLibrary::stream_stop(handle).map_err(|stop_err| {
                    LJMError::LibraryError(format!(
                        "LabJack stream_stop failed for stale stream on '{}': {:?}",
                        requested_ip, stop_err
                    ))
                })?;
                LJMLibrary::write_name(handle, "STREAM_SETTLING_US", settling_us).map_err(
                    |retry_err| {
                        LJMError::LibraryError(format!(
                            "LabJack self-test write failed after stream_stop for '{}': STREAM_SETTLING_US={}: {:?}",
                            requested_ip, settling_us, retry_err
                        ))
                    },
                )?;
            }
            Err(err) => {
                return Err(LJMError::LibraryError(format!(
                    "LabJack self-test write failed for '{}': STREAM_SETTLING_US={}: {:?}",
                    requested_ip, settling_us, err
                )));
            }
        }

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

/// Reads LabJack metadata for an open handle.
///
/// # Arguments
///
/// * `handle` - LJM device handle.
///
/// # Errors
///
/// Returns the LJM error if the handle info cannot be read.
pub fn handle_info(handle: i32) -> Result<DeviceHandleInfo, LJMError> {
    LJMLibrary::get_handle_info(handle)
}

#[allow(dead_code)]
/// Converts the signed IPv4 bits in LabJack handle info to dotted decimal text.
///
/// LJM reports the address as an `i32`; its bits are reinterpreted as `u32`, so
/// addresses at or above `128.0.0.0` (negative as `i32`) convert correctly.
///
/// # Arguments
///
/// * `info` - Handle info from [`handle_info`].
///
/// # Returns
///
/// The address, or `None` when LJM reports `0` (no IP address).
///
/// # Errors
///
/// Never returns an error. The `Result` return type is kept for callers that use
/// `?`.
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

    /// Checks that a negative `i32` address converts to the right dotted quad.
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

    /// Checks that address 0 is treated as missing.
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
