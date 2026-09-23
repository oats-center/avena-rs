/**
 * Connection helpers for central NATS and the KV bucket `avenabox`.
 *
 * The dashboard talks to central NATS over WebSocket only. This module turns the
 * server text a user types (with or without a scheme) into WebSocket URLs, connects
 * with the contents of a `.creds` file, and wraps the KV calls the pages use to list,
 * read, write and delete LabJack configuration keys.
 *
 * @module
 */
import { Kvm } from "@nats-io/kv";
import { wsconnect, credsAuthenticator, type NatsConnection } from "@nats-io/nats-core";

/**
 * Removes one trailing slash from a URL string.
 *
 * @param url - URL text.
 * @returns `url` without its final `/`, or `url` unchanged if it has none.
 */
function stripTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}

/**
 * Reports whether server text already starts with a URL scheme such as `ws://`.
 *
 * @param server - Server text, already trimmed.
 * @returns `true` if the text matches `<scheme>://` at the start.
 */
function hasScheme(server: string): boolean {
  return /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(server);
}

/**
 * Chooses the WebSocket scheme that matches the page's own protocol.
 *
 * @returns `wss` when the page was loaded over HTTPS, otherwise `ws` (including
 *   during server-side rendering, where there is no `window`).
 */
function defaultWebsocketScheme(): "ws" | "wss" {
  if (typeof window !== "undefined" && window.location.protocol === "https:") {
    return "wss";
  }
  return "ws";
}

/**
 * Normalizes user-entered NATS server text into a WebSocket URL.
 *
 * Trims the text, adds the scheme from `defaultWebsocketScheme` when none is
 * given, maps `http:` to `ws:` and `https:` to `wss:`, and drops a bare `/` path and
 * any trailing slash.
 *
 * @param serverName - Server text as typed, e.g. `nats1.oats:8080` or
 *   `wss://nats.example.org`.
 * @returns The WebSocket URL, an empty string for blank input, or the text with the
 *   scheme prefix added if it does not parse as a URL.
 */
function normalizeWebsocketServer(serverName: string): string {
  const trimmed = serverName.trim();
  if (!trimmed) return "";

  const candidate = hasScheme(trimmed) ? trimmed : `${defaultWebsocketScheme()}://${trimmed}`;

  try {
    const parsed = new URL(candidate);
    if (parsed.protocol === "http:") parsed.protocol = "ws:";
    if (parsed.protocol === "https:") parsed.protocol = "wss:";
    if (parsed.pathname === "/") parsed.pathname = "";
    return stripTrailingSlash(parsed.toString());
  } catch {
    return candidate;
  }
}

/**
 * Builds the WebSocket URLs to try, in order, for a NATS server value.
 *
 * The first candidate is the normalized URL from `normalizeWebsocketServer`.
 * When the user omitted a scheme, the same URL with the other scheme (`ws` or `wss`)
 * follows, so the dashboard can connect from both local and HTTPS deployments.
 * An explicit scheme yields a single candidate.
 *
 * @param serverName - Server text as typed.
 * @returns Distinct candidate URLs, or an empty array for blank input.
 */
function buildServerCandidates(serverName: string): string[] {
  const normalized = normalizeWebsocketServer(serverName);
  if (!normalized) return [];

  const candidates = new Set<string>([normalized]);
  const explicitScheme = hasScheme(serverName.trim());
  try {
    const parsed = new URL(normalized);
    if (!explicitScheme && parsed.protocol === "ws:") {
      parsed.protocol = "wss:";
      candidates.add(stripTrailingSlash(parsed.toString()));
    } else if (!explicitScheme && parsed.protocol === "wss:") {
      parsed.protocol = "ws:";
      candidates.add(stripTrailingSlash(parsed.toString()));
    }
  } catch {
    // keep normalized only if URL parsing fails
  }

  return Array.from(candidates);
}

/**
 * Open connection to central NATS plus the KV manager bound to it.
 *
 * Returned by {@link connect} and passed to the KV helpers and the exporter client.
 */
export class NatsService {
  /** Open WebSocket connection to central NATS. */
  public connection: NatsConnection;
  /** Key-Value manager bound to the active connection. */
  public kvm: Kvm;
  /**
   * Wraps an established connection and its KV manager.
   *
   * @param connection - Open NATS connection.
   * @param kvm - KV manager created from `connection`.
   */
  constructor (
    connection: NatsConnection,
    kvm: Kvm
  ) {
    this.connection = connection;
    this.kvm = kvm;
  }
}

/**
 * Connects to central NATS over WebSocket and creates a KV manager.
 *
 * Tries each URL from `buildServerCandidates` in order and returns the first
 * connection that succeeds. TLS is turned off explicitly for `ws:` URLs. Every
 * failure is logged to the console, and the function resolves to `null` instead of
 * rejecting.
 *
 * @param serverName - Server text as typed, e.g. `nats1.oats:8080`. A missing
 *   scheme is filled in from the page protocol, and the other scheme is tried next.
 * @param credentialsContent - Contents of a `.creds` file, not a path. Omit to
 *   connect without credentials.
 * @returns The connected service, or `null` if the server text is blank, the
 *   credentials authenticator cannot be built, or no candidate URL connects.
 *
 * @example
 * ```ts
 * const nats = await connect("nats1.oats:8080", credsText);
 * if (!nats) throw new Error("Could not reach central NATS");
 * ```
 */
export async function connect(serverName: string, credentialsContent?: string): Promise<NatsService | null> {
  const servers = buildServerCandidates(serverName);
  if (servers.length === 0) return null;

  const connectionOptions: any = {};
  if (credentialsContent) {
    try {
      const creds = new TextEncoder().encode(credentialsContent);
      connectionOptions.authenticator = credsAuthenticator(creds);
    } catch (error) {
      console.error("Failed to process credentials:", error);
      return null;
    }
  }

  let lastError: unknown = null;
  for (const server of servers) {
    try {
      const parsed = new URL(server);
      const nc = await wsconnect({
        ...connectionOptions,
        servers: server,
        tls: parsed.protocol === "ws:" ? false : undefined
      });
      const kvm = new Kvm(nc);
      return new NatsService(nc, kvm);
    } catch (error) {
      lastError = error;
      console.error(`Failed to connect to NATS at ${server}:`, error);
    }
  }

  console.error(
    `Failed to connect to NATS. Tried endpoints: ${servers.join(", ")}.`,
    lastError
  );
  return null;
}

/**
 * Lists the keys in a KV bucket.
 *
 * @param nats - Connected service from {@link connect}.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param filter - Optional key filter with NATS wildcards (`*`, `>`). Omit to list
 *   every key.
 * @returns The keys, in the order the server lists them.
 * @throws If `nats` is not set, the bucket cannot be opened, or listing fails.
 */
export async function getKeys(nats: NatsService, bucket: string, filter?: string): Promise<string[]> {
  if (!nats) throw new Error("NATS connection is not initialized");
  
  const kv = await nats.kvm.open(bucket);
  const keysList: string[] = [];
  const keys = await kv.keys(filter);
  
  for await (const key of keys ) {
    keysList.push(key);
  }
  
  return keysList;
}

/**
 * Reads one value from a KV bucket as a string.
 *
 * @param nats - Connected service from {@link connect}.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param key - Key to read, e.g. `<site>.<box>.<source>.config`.
 * @returns The value decoded as UTF-8 text. When the key has no entry or its value
 *   is empty, resolves to the literal string `"Key value does not exist"` rather
 *   than rejecting, so callers must check for it before parsing.
 * @throws If `nats` is not set, the bucket cannot be opened, or the read fails.
 */
export async function getKeyValue(nats: NatsService, bucket: string, key: string): Promise<string> {
  if (!nats) throw new Error("Nats connection is not initialized");
  
  const kv = await nats.kvm.open(bucket);
  let val = await kv.get(key);
  
  const valStr = val?.string() || "Key value does not exist";
  
  return valStr;
}

/**
 * Writes one string value to a KV bucket.
 *
 * @param nats - Connected service from {@link connect}.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param key - Key to write.
 * @param newValue - Value to store.
 * @throws If `nats` is not set, the bucket cannot be opened, or the put fails.
 */
export async function putKeyValue(nats: NatsService, bucket: string, key: string, newValue: string): Promise<void> {
  if (!nats) throw new Error("Nats connection is not initialized");
  const kv = await nats.kvm.open(bucket);
  await kv.put(key, newValue);
}

/**
 * Opens a new connection, writes a configuration object to KV as JSON, and closes it.
 *
 * The object is serialized with two-space indentation. Errors are logged to the
 * console and reported through the return value instead of being thrown.
 *
 * @remarks
 * The connection is closed only on success. If the write fails, the connection
 * opened here is left open.
 *
 * @param serverName - Server text passed to {@link connect}.
 * @param credentialsContent - Contents of a `.creds` file, not a path.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param key - Key to write, e.g. `<site>.<box>.<source>.config`.
 * @param configData - Value passed to `JSON.stringify`.
 * @returns `true` if the value was written, `false` if connecting or writing failed.
 */
export async function updateConfig(serverName: string, credentialsContent: string, bucket: string, key: string, configData: any): Promise<boolean> {
  try {
    const nats = await connect(serverName, credentialsContent);
    if (!nats) {
      console.error("Failed to connect to NATS for update");
      return false;
    }
    
    const configJson = JSON.stringify(configData, null, 2);
    await putKeyValue(nats, bucket, key, configJson);
    
    nats.connection.close();
    return true;
  } catch (error) {
    console.error("Failed to update config:", error);
    return false;
  }
}

/**
 * Opens a new connection, deletes one key from a KV bucket, and closes it.
 *
 * Uses the KV `delete` operation, which records a delete marker rather than purging
 * the key's history. Errors are logged to the console and reported through the
 * return value instead of being thrown.
 *
 * @remarks
 * The connection is closed only on success. If the delete fails, the connection
 * opened here is left open.
 *
 * @param serverName - Server text passed to {@link connect}.
 * @param credentialsContent - Contents of a `.creds` file, not a path.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param key - Key to delete.
 * @returns `true` if the key was deleted, `false` if connecting or deleting failed.
 */
export async function deleteKey(serverName: string, credentialsContent: string, bucket: string, key: string): Promise<boolean> {
  try {
    const nats = await connect(serverName, credentialsContent);
    if (!nats) {
      console.error("Failed to connect to NATS for deletion");
      return false;
    }
    
    const kv = await nats.kvm.open(bucket);
    await kv.delete(key);
    
    nats.connection.close();
    return true;
  } catch (error) {
    console.error("Failed to delete key:", error);
    return false;
  }
}
