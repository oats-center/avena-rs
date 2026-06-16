import { Kvm } from "@nats-io/kv";
import { wsconnect, credsAuthenticator, type NatsConnection } from "@nats-io/nats-core";

/** Removes a trailing slash from a websocket URL string. */
function stripTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}

/** Returns true when a server string already includes a URL scheme. */
function hasScheme(server: string): boolean {
  return /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(server);
}

/** Chooses the websocket scheme that matches the current page security. */
function defaultWebsocketScheme(): "ws" | "wss" {
  if (typeof window !== "undefined" && window.location.protocol === "https:") {
    return "wss";
  }
  return "ws";
}

/** Normalizes user-entered NATS server text into a websocket URL. */
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
 * Builds websocket endpoint candidates for a NATS server value.
 *
 * When the user omits a scheme, both `ws` and `wss` variants are tried so the
 * dashboard can connect from local and HTTPS deployments.
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

/** Connected NATS client plus Key-Value manager used by dashboard actions. */
export class NatsService {
  /** Active websocket NATS connection. */
  public connection: NatsConnection;
  /** Key-Value manager bound to the active connection. */
  public kvm: Kvm;
  /** Creates a wrapper around an established NATS connection and KV manager. */
  constructor (
    connection: NatsConnection,
    kvm: Kvm
  ) {
    this.connection = connection;
    this.kvm = kvm;
  }
}

/** Connects to NATS over websocket and initializes KV access. */
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

/** Lists keys in a NATS KV bucket, optionally filtered by a subject pattern. */
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

/** Reads one string value from a NATS KV bucket. */
export async function getKeyValue(nats: NatsService, bucket: string, key: string): Promise<string> {
  if (!nats) throw new Error("Nats connection is not initialized");
  
  const kv = await nats.kvm.open(bucket);
  let val = await kv.get(key);
  
  const valStr = val?.string() || "Key value does not exist";
  
  return valStr;
}

/** Writes one string value to a NATS KV bucket. */
export async function putKeyValue(nats: NatsService, bucket: string, key: string, newValue: string): Promise<void> {
  if (!nats) throw new Error("Nats connection is not initialized");
  const kv = await nats.kvm.open(bucket);
  await kv.put(key, newValue);
}

/** Connects with credentials and writes a JSON configuration object to KV. */
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

/** Connects with credentials and deletes one key from a NATS KV bucket. */
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
