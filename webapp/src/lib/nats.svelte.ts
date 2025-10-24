import { Kvm } from "@nats-io/kv";
import { wsconnect, credsAuthenticator, type NatsConnection } from "@nats-io/nats-core";

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
  let nc: NatsConnection | null = null;
  if (serverName) {
    try {
      const connectionOptions: any = { 
        servers: serverName 
      };
      
      // Add credentials authentication if credentials content is provided
      if (credentialsContent) {
        try {
          const creds = new TextEncoder().encode(credentialsContent);
          connectionOptions.authenticator = credsAuthenticator(creds);
        } catch (error) {
          console.error("Failed to process credentials:", error);
          return null;
        }
      }
      
      nc = await wsconnect(connectionOptions);
    } catch (error) {
      console.error("Failed to connect to NATS: ", error);
    }
  }
  let nats: NatsService
  if(nc){
    const kvm = new Kvm(nc);
    nats = new NatsService(nc, kvm);
    return nats;
  }
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

export interface ExportRequestPayload {
  asset: number;
  channels: number[];
  start: string;
  end: string;
  format: "csv" | "parquet";
  bucket?: string;
  download_name?: string;
}

export interface ExportResponsePayload {
  status: string;
  request_id?: string;
  object?: string;
  bucket?: string;
  size_bytes?: number;
  download_name?: string;
  control_subject?: string;
  data_subject?: string;
  chunk_size?: number;
  content_type?: string;
  missing_channels?: number[];
  error?: string;
}

interface DownloadMetaPayload {
  bucket: string;
  download_name: string;
  size_bytes: number;
  format: string;
  chunk_size: number;
}

export interface DownloadResult {
  blob: Blob;
  fileName: string;
  size: number;
}

export async function requestExport(
  nats: NatsService,
  payload: ExportRequestPayload,
  subject = "avenabox.export.request",
  timeoutMs = 60000
): Promise<ExportResponsePayload> {
  const data = new TextEncoder().encode(JSON.stringify(payload));
  const msg = await nats.connection.request(subject, data, { timeout: timeoutMs });
  const text = new TextDecoder().decode(msg.data);
  return JSON.parse(text) as ExportResponsePayload;
}

export async function downloadExport(
  nats: NatsService,
  response: ExportResponsePayload,
  onProgress?: (received: number, total?: number) => void
): Promise<DownloadResult> {
  if (!response.control_subject || !response.data_subject) {
    throw new Error("Export response missing download subjects");
  }

  const decoder = new TextDecoder();
  const encoder = new TextEncoder();
  const chunks: Uint8Array[] = [];
  let received = 0;
  let meta: DownloadMetaPayload | null = null;

  const subscription = nats.connection.subscribe(response.data_subject);
  const handshake = nats.connection.request(
    response.control_subject,
    encoder.encode(JSON.stringify({ start: true }))
  );

  for await (const msg of subscription) {
    const event = msg.headers?.get("Nats-Download-Event") ?? "chunk";
    if (event === "meta") {
      meta = JSON.parse(decoder.decode(msg.data)) as DownloadMetaPayload;
    } else if (event === "chunk") {
      chunks.push(msg.data);
      received += msg.data.length;
      onProgress?.(received, response.size_bytes ?? meta?.size_bytes);
    } else if (event === "complete") {
      subscription.unsubscribe();
      break;
    } else if (event === "error") {
      subscription.unsubscribe();
      const details = JSON.parse(decoder.decode(msg.data));
      throw new Error(details?.error ?? "Export failed");
    }
  }

  await handshake;

  const fileName =
    meta?.download_name ?? response.download_name ?? "labjack-export";
  const mimeType =
    response.content_type ??
    (meta?.format === "csv" ? "text/csv" : "application/octet-stream");
  const blob = new Blob(chunks, { type: mimeType });
  return {
    blob,
    fileName,
    size: meta?.size_bytes ?? blob.size,
  };
}
