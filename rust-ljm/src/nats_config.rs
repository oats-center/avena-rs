#![allow(dead_code)]

use async_nats::ServerAddr;

const DEFAULT_NATS_SERVERS: &str = "nats://127.0.0.1:4222";

pub fn servers_from_env() -> Result<Vec<ServerAddr>, String> {
    let raw = std::env::var("NATS_SERVERS").unwrap_or_else(|_| DEFAULT_NATS_SERVERS.to_string());
    let servers: Result<Vec<ServerAddr>, _> = raw
        .split(',')
        .map(str::trim)
        .filter(|server| !server.is_empty())
        .map(str::parse)
        .collect();

    let servers = servers.map_err(|e| format!("invalid NATS_SERVERS entry: {e}"))?;
    if servers.is_empty() {
        return Err("NATS_SERVERS did not contain any usable server URLs".to_string());
    }

    Ok(servers)
}

pub fn jetstream_context(client: async_nats::Client) -> async_nats::jetstream::Context {
    match std::env::var("JS_DOMAIN") {
        Ok(domain) if !domain.trim().is_empty() => {
            async_nats::jetstream::with_domain(client, domain.trim())
        }
        _ => async_nats::jetstream::new(client),
    }
}
