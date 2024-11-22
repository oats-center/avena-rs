import { Kvm } from "@nats-io/kv";
import { wsconnect, type NatsConnection } from "@nats-io/nats-core";

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
export async function connect(serverName: string): Promise<NatsService | null> {
  let nc: NatsConnection | null = null;
  if (serverName) {
    try {
      nc = await wsconnect({ servers: serverName });
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
  const valStr = val?.string() || "";
  return valStr;
}

//gets multiple values in the given bucket at each of the given keys
export async function getKeyValues(nats: NatsService, bucket: string, keys: string[]): Promise<string[]>{
  if (!nats) throw new Error("Nats connection is not initialized");
  const kv = await nats.kvm.open(bucket);
  const values: string[] = [];
  for await (const key of keys) {
    let val = await kv.get(key);
    console.log(val)
    const valStr = val?.string() || "";
    values.push(valStr);
  }
  return values;
}

//puts a values in the given bucket at the given key
export async function putKeyValue(nats: NatsService, bucket: string, key: string, newValue: string): Promise<void> {
  if (!nats) throw new Error("Nats connection is not initialized");
  const kv = await nats.kvm.open(bucket);
  await kv.put(key, newValue);
}
