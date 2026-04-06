use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::process::Command;
use std::str::FromStr;
use std::sync::mpsc;
use std::time::Duration;

use ljmrs::handle::{ConnectionType, DeviceHandleInfo, DeviceType};
use ljmrs::{LJMError, LJMLibrary};

#[derive(Clone, Debug)]
struct LocalIpv4Interface {
    address: Ipv4Addr,
    prefix_len: u8,
}

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

fn push_unique(values: &mut Vec<String>, value: Option<String>) {
    if let Some(value) = value {
        if !values.iter().any(|existing| existing == &value) {
            values.push(value);
        }
    }
}

fn direct_ethernet_identifiers_from_env() -> Vec<String> {
    let mut identifiers = Vec::new();

    push_unique(
        &mut identifiers,
        env_identifier("LABJACK_IDENTIFIER").filter(|value| parse_ipv4(value).is_some()),
    );
    push_unique(&mut identifiers, env_identifier("LABJACK_IP"));

    identifiers
}

fn indirect_ethernet_identifiers_from_env() -> Vec<String> {
    let mut identifiers = Vec::new();

    push_unique(
        &mut identifiers,
        env_identifier("LABJACK_IDENTIFIER").filter(|value| parse_ipv4(value).is_none()),
    );
    push_unique(&mut identifiers, env_identifier("LABJACK_SERIAL"));
    push_unique(&mut identifiers, env_identifier("LABJACK_NAME"));

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

fn requested_ethernet_serial() -> Option<i32> {
    env_identifier("LABJACK_SERIAL")
        .or_else(|| {
            env_identifier("LABJACK_IDENTIFIER").filter(|value| parse_ipv4(value).is_none())
        })
        .and_then(|value| value.parse().ok())
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
        let label = format!("{mode} identifier '{identifier}'");
        println!("[labjack] trying {label}");

        match LJMLibrary::open_jack(DeviceType::T7, connection_type.clone(), identifier.as_str()) {
            Ok(handle) => return Some(handle),
            Err(err) => failures.push(format!("{}: {:?}", format_attempt(mode, &identifier), err)),
        }
    }

    None
}

fn try_open_usb(failures: &mut Vec<String>) -> Option<i32> {
    try_open_with_identifiers(
        "usb",
        ConnectionType::USB,
        usb_identifiers_from_env(),
        failures,
    )
}

fn should_scan_interface(name: &str) -> bool {
    !matches!(
        name,
        "lo" | "tailscale0" | "docker0" | "podman0" | "virbr0" | "zt0" | "tun0" | "tap0"
    ) && !name.starts_with("br-")
        && !name.starts_with("docker")
        && !name.starts_with("tailscale")
        && !name.starts_with("veth")
        && !name.starts_with("virbr")
        && !name.starts_with("zt")
        && !name.starts_with("tun")
        && !name.starts_with("tap")
}

#[cfg(target_os = "linux")]
fn local_ipv4_interfaces() -> Result<Vec<LocalIpv4Interface>, LJMError> {
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show", "up", "scope", "global"])
        .output()
        .map_err(|err| {
            LJMError::LibraryError(format!("Failed to inspect local interfaces: {err}"))
        })?;

    if !output.status.success() {
        return Err(LJMError::LibraryError(format!(
            "Failed to inspect local interfaces: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut interfaces = Vec::new();

    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let _index = parts.next();
        let Some(name) = parts.next() else {
            continue;
        };

        if !should_scan_interface(name.trim_end_matches(':')) {
            continue;
        }

        while let Some(token) = parts.next() {
            if token != "inet" {
                continue;
            }

            let Some(addr) = parts.next() else {
                break;
            };
            let Some((ip, prefix_len)) = addr.split_once('/') else {
                break;
            };
            let Some(address) = parse_ipv4(ip) else {
                break;
            };
            let Ok(prefix_len) = prefix_len.parse::<u8>() else {
                break;
            };

            if prefix_len >= 31 {
                break;
            }

            interfaces.push(LocalIpv4Interface {
                address,
                prefix_len,
            });
            break;
        }
    }

    Ok(interfaces)
}

#[cfg(not(target_os = "linux"))]
fn local_ipv4_interfaces() -> Result<Vec<LocalIpv4Interface>, LJMError> {
    Ok(Vec::new())
}

fn ipv4_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from_be_bytes(ip.octets())
}

fn u32_to_ipv4(value: u32) -> Ipv4Addr {
    Ipv4Addr::from(value.to_be_bytes())
}

fn scan_prefix_len(prefix_len: u8) -> Option<u8> {
    if prefix_len >= 31 {
        None
    } else {
        Some(prefix_len.max(24))
    }
}

fn candidate_ipv4s(interface: &LocalIpv4Interface) -> Vec<Ipv4Addr> {
    let Some(scan_prefix_len) = scan_prefix_len(interface.prefix_len) else {
        return Vec::new();
    };

    let mask = if scan_prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - scan_prefix_len)
    };
    let host_ip = ipv4_to_u32(interface.address);
    let network = host_ip & mask;
    let broadcast = network | !mask;

    if broadcast <= network + 1 {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    for raw in (network + 1)..broadcast {
        if raw == host_ip {
            continue;
        }
        candidates.push(u32_to_ipv4(raw));
    }

    candidates
}

fn tcp_port_open(ip: Ipv4Addr, port: u16, timeout: Duration) -> bool {
    TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(ip), port), timeout).is_ok()
}

fn candidate_ips_from_local_tcp_scan() -> Result<Vec<Ipv4Addr>, LJMError> {
    let interfaces = local_ipv4_interfaces()?;
    if interfaces.is_empty() {
        return Ok(Vec::new());
    }

    let mut unique_candidates = BTreeSet::new();
    for interface in &interfaces {
        for candidate in candidate_ipv4s(interface) {
            unique_candidates.insert(candidate);
        }
    }

    let candidates: Vec<Ipv4Addr> = unique_candidates.into_iter().collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    println!("[labjack] scanning local ethernet subnets for tcp/502 candidates");

    let worker_count = candidates.len().min(32);
    let chunk_size = candidates.len().div_ceil(worker_count);
    let (sender, receiver) = mpsc::channel();

    for chunk in candidates.chunks(chunk_size) {
        let sender = sender.clone();
        let chunk = chunk.to_vec();
        std::thread::spawn(move || {
            for ip in chunk {
                if tcp_port_open(ip, 502, Duration::from_millis(75)) {
                    let _ = sender.send(ip);
                }
            }
        });
    }
    drop(sender);

    let mut reachable = receiver.into_iter().collect::<Vec<_>>();
    reachable.sort();
    reachable.dedup();
    Ok(reachable)
}

fn try_open_ethernet_from_local_scan(failures: &mut Vec<String>) -> Option<i32> {
    let serial_filter = requested_ethernet_serial();
    let candidates = match candidate_ips_from_local_tcp_scan() {
        Ok(candidates) => candidates,
        Err(err) => {
            failures.push(format!("ethernet(local-scan): {:?}", err));
            return None;
        }
    };

    if candidates.is_empty() {
        failures
            .push("ethernet(local-scan): no tcp/502 hosts found on local interfaces".to_string());
        return None;
    }

    let mut matches = Vec::new();

    for ip in candidates {
        let identifier = ip.to_string();
        println!("[labjack] probing ethernet candidate '{identifier}'");

        let handle = match LJMLibrary::open_jack(
            DeviceType::T7,
            ConnectionType::ETHERNET,
            identifier.as_str(),
        ) {
            Ok(handle) => handle,
            Err(err) => {
                failures.push(format!("ethernet(local-scan '{}'): {:?}", identifier, err));
                continue;
            }
        };

        let info = match handle_info(handle) {
            Ok(info) => info,
            Err(err) => {
                let _ = LJMLibrary::close_jack(handle);
                failures.push(format!(
                    "ethernet(local-scan '{}'): handle info failed: {:?}",
                    identifier, err
                ));
                continue;
            }
        };

        if let Some(serial_filter) = serial_filter {
            if info.serial_number != serial_filter {
                let _ = LJMLibrary::close_jack(handle);
                continue;
            }
        }

        matches.push((identifier, handle, info));
    }

    if matches.is_empty() {
        failures.push(
            "ethernet(local-scan): found tcp/502 hosts, but none opened as a matching T7"
                .to_string(),
        );
        return None;
    }

    if matches.len() > 1 {
        let summary = matches
            .iter()
            .map(|(ip, _, info)| format!("{ip} (serial {})", info.serial_number))
            .collect::<Vec<_>>()
            .join(", ");

        for (_, handle, _) in matches {
            let _ = LJMLibrary::close_jack(handle);
        }

        failures.push(format!(
            "ethernet(local-scan): multiple T7 devices matched: {summary}. Set LABJACK_IP or LABJACK_SERIAL."
        ));
        return None;
    }

    let (identifier, handle, info) = matches.pop().unwrap();
    println!(
        "[labjack] resolved ethernet LabJack to {} (serial {})",
        identifier, info.serial_number
    );
    Some(handle)
}

fn try_open_ethernet(failures: &mut Vec<String>) -> Option<i32> {
    if let Some(handle) = try_open_with_identifiers(
        "ethernet",
        ConnectionType::ETHERNET,
        direct_ethernet_identifiers_from_env(),
        failures,
    ) {
        return Some(handle);
    }

    if let Some(handle) = try_open_ethernet_from_local_scan(failures) {
        return Some(handle);
    }

    try_open_with_identifiers(
        "ethernet",
        ConnectionType::ETHERNET,
        indirect_ethernet_identifiers_from_env(),
        failures,
    )
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
            "ethernet" | "tcp" => try_open_ethernet(&mut failures),
            "usb" => try_open_usb(&mut failures),
            "any" => try_open_ethernet(&mut failures).or_else(|| try_open_usb(&mut failures)),
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

#[allow(dead_code)]
pub fn handle_ip_address(info: &DeviceHandleInfo) -> Result<Option<String>, LJMError> {
    if info.ip_address == 0 {
        return Ok(None);
    }

    // LJM exposes the raw IPv4 value through a signed i32. Reinterpret the bits
    // directly so addresses above 127.255.255.255 do not fail conversion.
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
