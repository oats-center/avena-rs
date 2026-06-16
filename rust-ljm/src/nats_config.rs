#![allow(dead_code)]

use async_nats::ServerAddr;

const DEFAULT_NATS_SERVERS: &str = "nats://127.0.0.1:4222";

pub fn servers_from_env_var(var_name: &str, default: &str) -> Result<Vec<ServerAddr>, String> {
    let raw = std::env::var(var_name).unwrap_or_else(|_| default.to_string());
    let servers: Result<Vec<ServerAddr>, _> = raw
        .split(',')
        .map(str::trim)
        .filter(|server| !server.is_empty())
        .map(str::parse)
        .collect();

    let servers = servers.map_err(|e| format!("invalid {var_name} entry: {e}"))?;
    if servers.is_empty() {
        return Err(format!("{var_name} did not contain any usable server URLs"));
    }

    Ok(servers)
}

pub fn servers_from_env() -> Result<Vec<ServerAddr>, String> {
    servers_from_env_var("NATS_SERVERS", DEFAULT_NATS_SERVERS)
}

pub fn jetstream_context_from_env(
    client: async_nats::Client,
    domain_var: &str,
) -> async_nats::jetstream::Context {
    match std::env::var(domain_var) {
        Ok(domain) if !domain.trim().is_empty() => {
            async_nats::jetstream::with_domain(client, domain.trim())
        }
        _ => async_nats::jetstream::new(client),
    }
}

pub fn jetstream_context_for_domain(
    client: async_nats::Client,
    domain: Option<&str>,
) -> async_nats::jetstream::Context {
    match domain.map(str::trim).filter(|value| !value.is_empty()) {
        Some(domain) => async_nats::jetstream::with_domain(client, domain),
        None => async_nats::jetstream::new(client),
    }
}

pub fn jetstream_context(client: async_nats::Client) -> async_nats::jetstream::Context {
    jetstream_context_from_env(client, "JS_DOMAIN")
}
