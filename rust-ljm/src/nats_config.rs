#![allow(dead_code)]

use async_nats::ServerAddr;

const DEFAULT_NATS_SERVERS: &str = "nats://127.0.0.1:4222";

pub fn servers_from_env() -> Result<Vec<ServerAddr>, String> {
    servers_from_env_var("NATS_SERVERS", Some(DEFAULT_NATS_SERVERS))
}

pub fn servers_from_env_var(var_name: &str, default: Option<&str>) -> Result<Vec<ServerAddr>, String> {
    let raw = match std::env::var(var_name) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => default.unwrap_or("").to_string(),
    };
    let servers: Result<Vec<ServerAddr>, _> = raw
        .split(',')
        .map(str::trim)
        .filter(|server| !server.is_empty())
        .map(str::parse)
        .collect();

    let servers = servers.map_err(|e| format!("invalid {var_name} entry: {e}"))?;
    if servers.is_empty() {
        return Err(format!(
            "{var_name} did not contain any usable server URLs"
        ));
    }

    Ok(servers)
}

pub fn jetstream_context(client: async_nats::Client) -> async_nats::jetstream::Context {
    let domain = std::env::var("JS_DOMAIN").ok();
    jetstream_context_for_domain(client, domain.as_deref())
}

pub fn jetstream_context_for_domain(
    client: async_nats::Client,
    domain: Option<&str>,
) -> async_nats::jetstream::Context {
    match domain {
        Some(domain) if !domain.trim().is_empty() => {
            async_nats::jetstream::with_domain(client, domain.trim())
        }
        _ => async_nats::jetstream::new(client),
    }
}
