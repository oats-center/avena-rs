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
import { jetstream } from "@nats-io/jetstream";
import { Kvm, type KV } from "@nats-io/kv";
import { wsconnect, credsAuthenticator, type NatsConnection } from "@nats-io/nats-core";

/** Matches server text that starts with a URL scheme such as `ws://`. */
const SCHEME_PATTERN = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//;

/** Encodes `.creds` text for the authenticator. */
const textEncoder = new TextEncoder();

/**
 * Most KV reads {@link getKeyValues} keeps in flight at once. Enough to hide the round
 * trip on a slow link (100-350 ms), few enough that the last reply of a batch of
 * configs does not queue behind megabytes of others on a 2 Mbit/s link.
 */
export const KV_READ_CONCURRENCY = 16;

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
  return SCHEME_PATTERN.test(server);
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
 * Builds the WebSocket URLs to try, in order, for a NATS server value.
 *
 * The first candidate is the normalized URL: the text is trimmed, the scheme from
 * `defaultWebsocketScheme` is added when none is given, `http:` becomes `ws:` and
 * `https:` becomes `wss:`, and a bare `/` path and any trailing slash are dropped.
 * When the user omitted a scheme, the same URL with the other scheme (`ws` or `wss`)
 * follows, so the dashboard can connect from both local and HTTPS deployments.
 * An explicit scheme yields a single candidate. The text is parsed as a URL once.
 *
 * @param serverName - Server text as typed, e.g. `nats1.oats:8080` or
 *   `wss://nats.example.org`.
 * @returns Distinct candidate URLs, or an empty array for blank input. Text that does
 *   not parse as a URL yields a single candidate: the text with the scheme prefix
 *   added.
 */
function buildServerCandidates(serverName: string): string[] {
  const trimmed = serverName.trim();
  if (!trimmed) return [];

  const explicitScheme = hasScheme(trimmed);
  const candidate = explicitScheme ? trimmed : `${defaultWebsocketScheme()}://${trimmed}`;

  let parsed: URL;
  try {
    parsed = new URL(candidate);
  } catch {
    return [candidate];
  }
  if (parsed.protocol === "http:") parsed.protocol = "ws:";
  if (parsed.protocol === "https:") parsed.protocol = "wss:";
  if (parsed.pathname === "/") parsed.pathname = "";

  const candidates = new Set<string>([stripTrailingSlash(parsed.toString())]);
  if (!explicitScheme && parsed.protocol === "ws:") {
    parsed.protocol = "wss:";
    candidates.add(stripTrailingSlash(parsed.toString()));
  } else if (!explicitScheme && parsed.protocol === "wss:") {
    parsed.protocol = "ws:";
    candidates.add(stripTrailingSlash(parsed.toString()));
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
  /** Buckets opened by {@link NatsService.bucket}, by name. */
  private buckets = new Map<string, Promise<KV>>();
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

  /**
   * Opens a KV bucket once per connection and returns the same handle afterwards.
   *
   * Opening only binds a handle to the bucket's stream; the helpers below used to do it
   * on every call. A failed open is not kept, so the next call tries again.
   *
   * @param name - KV bucket name, e.g. `avenabox`.
   * @returns The bucket handle.
   */
  bucket(name: string): Promise<KV> {
    let opened = this.buckets.get(name);
    if (!opened) {
      opened = this.kvm.open(name);
      this.buckets.set(name, opened);
      opened.catch(() => {
        if (this.buckets.get(name) === opened) this.buckets.delete(name);
      });
    }
    return opened;
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
 * The KV manager's JetStream client skips the JetStream account check
 * (`checkAPI: false`), which would otherwise cost one extra round trip each time a
 * bucket is opened. An account without JetStream then fails on the first KV request
 * instead of on open.
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
      const creds = textEncoder.encode(credentialsContent);
      connectionOptions.authenticator = credsAuthenticator(creds);
    } catch (error) {
      console.error("Failed to process credentials:", error);
      return null;
    }
  }

  let lastError: unknown = null;
  for (const server of servers) {
    try {
      // Candidates come from `URL.toString()`, so the scheme is already lower case.
      const nc = await wsconnect({
        ...connectionOptions,
        servers: server,
        tls: server.startsWith("ws:") ? false : undefined
      });
      const kvm = new Kvm(jetstream(nc, { checkAPI: false }));
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
  
  const kv = await nats.bucket(bucket);
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
  
  const kv = await nats.bucket(bucket);
  let val = await kv.get(key);
  
  const valStr = val?.string() || "Key value does not exist";
  
  return valStr;
}

/**
 * Reads several values from a KV bucket, keeping up to `concurrency` reads in flight.
 *
 * Each read costs one round trip, so reading N keys one after another costs N round
 * trips; this costs about N / `concurrency`. Each key is read as by
 * {@link getKeyValue}, and one failed read does not stop the others.
 *
 * @param nats - Connected service from {@link connect}.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param keys - Keys to read.
 * @param concurrency - Most reads in flight at once. Default {@link KV_READ_CONCURRENCY}.
 * @returns One result per key, in the order of `keys`: the value (see
 *   {@link getKeyValue}, including its `"Key value does not exist"` text) or the error.
 */
export async function getKeyValues(
  nats: NatsService,
  bucket: string,
  keys: readonly string[],
  concurrency: number = KV_READ_CONCURRENCY
): Promise<PromiseSettledResult<string>[]> {
  const results: PromiseSettledResult<string>[] = new Array(keys.length);
  let next = 0;
  const worker = async () => {
    while (next < keys.length) {
      const index = next++;
      try {
        results[index] = { status: "fulfilled", value: await getKeyValue(nats, bucket, keys[index]) };
      } catch (reason) {
        results[index] = { status: "rejected", reason };
      }
    }
  };
  const workers = Math.max(1, Math.min(Math.floor(concurrency) || 1, keys.length));
  await Promise.all(Array.from({ length: workers }, worker));
  return results;
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
  const kv = await nats.bucket(bucket);
  await kv.put(key, newValue);
}

/**
 * Closes a short-lived connection, logging instead of throwing if closing fails.
 *
 * @param nats - Service to close, or `null` if connecting failed.
 */
async function closeQuietly(nats: NatsService | null): Promise<void> {
  if (!nats) return;
  try {
    await nats.connection.close();
  } catch (error) {
    console.error("Failed to close NATS connection:", error);
  }
}

/**
 * Returns `shared` when it is still open, so a write can skip opening a connection.
 *
 * @param shared - Connection the caller already holds, or `null`/`undefined`.
 * @returns `shared`, or `null` when it is missing or closed.
 */
function openShared(shared: NatsService | null | undefined): NatsService | null {
  return shared && !shared.connection.isClosed() ? shared : null;
}

/**
 * Writes a configuration object to KV as JSON.
 *
 * The object is serialized with two-space indentation. Errors are logged to the
 * console and reported through the return value instead of being thrown.
 *
 * Uses `shared` when it is given and still open, which saves the connection setup
 * (several round trips on a slow link); it is left open. Otherwise a new connection
 * is opened and always closed before returning.
 *
 * @param serverName - Server text passed to {@link connect}.
 * @param credentialsContent - Contents of a `.creds` file, not a path.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param key - Key to write, e.g. `<site>.<box>.<source>.config`.
 * @param configData - Value passed to `JSON.stringify`.
 * @param shared - Open connection to reuse, such as the page's own. Optional.
 * @returns `true` if the value was written, `false` if connecting or writing failed.
 */
export async function updateConfig(serverName: string, credentialsContent: string, bucket: string, key: string, configData: any, shared?: NatsService | null): Promise<boolean> {
  const reused = openShared(shared);
  let nats: NatsService | null = null;
  try {
    nats = reused ?? await connect(serverName, credentialsContent);
    if (!nats) {
      console.error("Failed to connect to NATS for update");
      return false;
    }
    
    const configJson = JSON.stringify(configData, null, 2);
    await putKeyValue(nats, bucket, key, configJson);
    return true;
  } catch (error) {
    console.error("Failed to update config:", error);
    return false;
  } finally {
    if (!reused) await closeQuietly(nats);
  }
}

/**
 * Deletes one key from a KV bucket.
 *
 * Uses the KV `delete` operation, which records a delete marker rather than purging
 * the key's history. Errors are logged to the console and reported through the
 * return value instead of being thrown.
 *
 * Uses `shared` when it is given and still open, and leaves it open. Otherwise a new
 * connection is opened and always closed before returning.
 *
 * @param serverName - Server text passed to {@link connect}.
 * @param credentialsContent - Contents of a `.creds` file, not a path.
 * @param bucket - KV bucket name, e.g. `avenabox`.
 * @param key - Key to delete.
 * @param shared - Open connection to reuse, such as the page's own. Optional.
 * @returns `true` if the key was deleted, `false` if connecting or deleting failed.
 */
export async function deleteKey(serverName: string, credentialsContent: string, bucket: string, key: string, shared?: NatsService | null): Promise<boolean> {
  const reused = openShared(shared);
  let nats: NatsService | null = null;
  try {
    nats = reused ?? await connect(serverName, credentialsContent);
    if (!nats) {
      console.error("Failed to connect to NATS for deletion");
      return false;
    }
    
    const kv = await nats.bucket(bucket);
    await kv.delete(key);
    return true;
  } catch (error) {
    console.error("Failed to delete key:", error);
    return false;
  } finally {
    if (!reused) await closeQuietly(nats);
  }
}
