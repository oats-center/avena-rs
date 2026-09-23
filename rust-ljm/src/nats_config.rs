//! Shared NATS server-list and JetStream context helpers.
//!
//! The streamer, archiver, exporter and subscriber all connect to a NATS server (by
//! default the local one), and the streamer and archiver open JetStream contexts,
//! including ones on a central server for configuration mirroring.
//! These helpers read the server list and the optional JetStream domain from the
//! environment in one consistent way.
//!
//! # Configuration
//!
//! * `NATS_SERVERS` - Comma-separated NATS server URLs, read by [`servers_from_env`].
//!   Default: `nats://127.0.0.1:4222`.
//! * `JS_DOMAIN` - JetStream domain, read by [`jetstream_context`]. Unset or blank
//!   means no domain.

#![allow(dead_code)]

use async_nats::ServerAddr;

/// Default NATS server URL used when `NATS_SERVERS` is unset: the local server on the
/// standard NATS port.
const DEFAULT_NATS_SERVERS: &str = "nats://127.0.0.1:4222";

/// Parses a comma-separated NATS server list from an environment variable.
///
/// Each entry is trimmed and empty entries are ignored, so a trailing comma is
/// harmless. `default` is parsed the same way when the variable is unset (or not valid
/// Unicode). A variable that is set but empty is not replaced by `default`.
///
/// # Arguments
///
/// * `var_name` - Name of the environment variable to read, for example
///   `NATS_SERVERS`. Also used in error messages.
/// * `default` - Comma-separated server list used when the variable is unset.
///
/// # Returns
///
/// The parsed server addresses, in the order given.
///
/// # Errors
///
/// Returns an error message if any entry cannot be parsed as a [`ServerAddr`], or if
/// no non-empty entries remain.
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

/// Parses `NATS_SERVERS`, defaulting to [`DEFAULT_NATS_SERVERS`].
///
/// # Errors
///
/// Returns an error message under the same conditions as [`servers_from_env_var`].
pub fn servers_from_env() -> Result<Vec<ServerAddr>, String> {
    servers_from_env_var("NATS_SERVERS", DEFAULT_NATS_SERVERS)
}

/// Creates a JetStream context using a domain name read from an environment variable.
///
/// If the variable is set and not blank, its trimmed value is used as the JetStream
/// domain. Otherwise the context has no domain.
///
/// # Arguments
///
/// * `client` - Connected NATS client; the context takes ownership of it.
/// * `domain_var` - Name of the environment variable holding the domain.
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

/// Creates a JetStream context for an explicit optional domain.
///
/// The domain is trimmed, and a blank domain is treated as `None`.
///
/// # Arguments
///
/// * `client` - Connected NATS client; the context takes ownership of it.
/// * `domain` - JetStream domain, or `None` for no domain.
pub fn jetstream_context_for_domain(
    client: async_nats::Client,
    domain: Option<&str>,
) -> async_nats::jetstream::Context {
    match domain.map(str::trim).filter(|value| !value.is_empty()) {
        Some(domain) => async_nats::jetstream::with_domain(client, domain),
        None => async_nats::jetstream::new(client),
    }
}

/// Creates the default JetStream context, using `JS_DOMAIN` as the domain if set.
///
/// Equivalent to [`jetstream_context_from_env`] with `JS_DOMAIN`.
///
/// # Arguments
///
/// * `client` - Connected NATS client; the context takes ownership of it.
pub fn jetstream_context(client: async_nats::Client) -> async_nats::jetstream::Context {
    jetstream_context_from_env(client, "JS_DOMAIN")
}
