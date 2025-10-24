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
