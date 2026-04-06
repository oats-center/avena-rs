import { Kvm } from "@nats-io/kv";
import { wsconnect, credsAuthenticator, type NatsConnection } from "@nats-io/nats-core";

function stripTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}

function hasScheme(server: string): boolean {
  return /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(server);
}

function defaultWebsocketScheme(): "ws" | "wss" {
  if (typeof window !== "undefined" && window.location.protocol === "https:") {
    return "wss";
  }
  return "ws";
}

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

function buildServerCandidates(serverName: string): string[] {
  const normalized = normalizeWebsocketServer(serverName);
  if (!normalized) return [];

  const candidates = new Set<string>([normalized]);
  try {
    const parsed = new URL(normalized);
    if (parsed.protocol === "ws:") {
      parsed.protocol = "wss:";
      candidates.add(stripTrailingSlash(parsed.toString()));
    } else if (parsed.protocol === "wss:") {
      parsed.protocol = "ws:";
      candidates.add(stripTrailingSlash(parsed.toString()));
    }
  } catch {
    // keep normalized only if URL parsing fails
  }

  return Array.from(candidates);
}

export class NatsService {
  public connection: NatsConnection;
  public kvm: Kvm;
  constructor (
    connection: NatsConnection,
    kvm: Kvm
  ) {
    this.connection = connection;
    this.kvm = kvm;
  }
}

//connects to a nats server & initialized kvm
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
      const nc = await wsconnect({
        ...connectionOptions,
        servers: server
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

//gets entire list of keys that bucket contains
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

//gets one value in the given bucket at the given key
export async function getKeyValue(nats: NatsService, bucket: string, key: string): Promise<string> {
  if (!nats) throw new Error("Nats connection is not initialized");
  
  const kv = await nats.kvm.open(bucket);
  let val = await kv.get(key);
  
  const valStr = val?.string() || "Key value does not exist";
  
  return valStr;
}


//puts a values in the given bucket at the given key
export async function putKeyValue(nats: NatsService, bucket: string, key: string, newValue: string): Promise<void> {
  if (!nats) throw new Error("Nats connection is not initialized");
  const kv = await nats.kvm.open(bucket);
  await kv.put(key, newValue);
}

//updates a configuration in NATS using credentials
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

//deletes a key from NATS bucket using credentials
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
