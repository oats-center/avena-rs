import { Kvm, KvWatchInclude } from '@nats-io/kv';
import type { KV } from "@nats-io/kv";
import { connect } from "@nats-io/transport-node";
import type { RequestEvent } from "@sveltejs/kit"

type SensorDataResponse = {
  sensorsList: string[];
  sensorValues: Sensor[] | null;
}

type Sensor = {
  id: string;
  name: string;
  type: string;
  value: string;
}

async function getKeysList(kv: KV): Promise<string[]> {
  const sensorsList: string[] = [];
  const keysList = await kv.keys();
  for await(const k of keysList){
    sensorsList.push(k)
  }
  return sensorsList;
}

async function getKeyValues(kvm: Kvm, sensorsList: string[]):Promise<Sensor[]>  {
  const sensorValues: Sensor[] = [];
    for await (const sensor of sensorsList) {
      let kv = await kvm.open(sensor);
      const keysList = await kv.keys();
      let keysArray: string[] = [];
      for await (const key of keysList){
        keysArray.push(key)
      }
      let keyValues: Sensor = {
        id: sensor,
        name: "",
        type: "",
        value: "",
      };
      for (const key of keysArray){
        let val = await kv.get(key)
        keyValues[key as keyof Sensor] = val?.string() || "";
      }
      sensorValues.push(keyValues)
    }
  return sensorValues;  
}

async function watchValues(kvm: Kvm, sensorsList: string[], sensorValues: Sensor[]): Promise<Sensor[]> {
  const stopSignal = { stopped: false };

  async function watchSensor(sensor: string, i: number, stopSignal: { stopped: boolean }) {
    const kv = await kvm.open(sensor);
    const watch = await kv.watch({
      include: KvWatchInclude.UpdatesOnly,
    });

    try {
      for await (const e of watch) {
        if (stopSignal.stopped) {
          console.log(`Stopping watch for sensor ${i}`);
          break; // Stop watching if another watcher has resolved
        }

        console.log(`watch: ${e.key}: ${e.operation} ${e.value ? e.string() : ""}`);
        sensorValues[i][e.key as keyof Sensor] = e.string();

        stopSignal.stopped = true; // Set the signal to stop all watches
        break;
      }
    } catch (error) {
      console.error("Error Watching Values: ", error)
    }
  }

  // Create an array of promises for all the sensors
  const sensorPromises = sensorsList.map((sensor, i) => watchSensor(sensor, i, stopSignal));
  await Promise.race(sensorPromises);

  return sensorValues;
} 

export async function GET({ url } : RequestEvent): Promise<Response> {
  const type: string | null = url.searchParams.get("type");
  try {
    const nc = await connect({ servers: "demo.nats.io:4222" })
    
    const kvm = new Kvm(nc);
    const kv = await kvm.open("sensorList");
    const sensorsList = await getKeysList(kv);
    let responseData: SensorDataResponse = { sensorsList, sensorValues: null };

    if (type === "getKVs" || type === "watchVals"){
      let sensorValues = await getKeyValues(kvm, sensorsList);
      if(type === "getKVs"){
        responseData.sensorValues = sensorValues;
      } 
      else{
        console.log("Started Watching...")
        sensorValues = await watchValues(kvm, sensorsList, sensorValues);
        responseData = {sensorsList, sensorValues};
      }
    }

    await nc.close();
    return new Response( JSON.stringify(responseData), {
      headers: { "Content-Type": "application/json"},
      status: 200,
    });
  } catch (error: unknown) {
    console.error("NATS Error: ", error);
    return new Response(JSON.stringify({ error: (error as Error).message} ), {
      headers: { 'Content-Type': 'application/json' },
      status : 500});
  }
}

type PutRequestData = {
  bucket: string;
  key: string;
  newValue: string;
}

export async function PUT({request}: RequestEvent): Promise<Response> {
  try {
    const nc = await connect({ servers: "demo.nats.io:4222" });

    const { bucket, key, newValue }: PutRequestData = await request.json()
    if(!key || !newValue || !bucket) {
      return new Response("Missing bucket, key, or newValue", { status: 400 });
    }

    const kvm = new Kvm(nc);
    const kv = await kvm.open(bucket);
    
    await kv.put(key, newValue);
    
    await nc.close();
    return new Response(JSON.stringify({ success: true }), {
      headers: { "Content-Type": "application/json"},
      status: 200,
    });   
  } catch (error: unknown) {
    console.error("NATS Error: ", error);
    return new Response(JSON.stringify({ error: (error as Error).message} ), {
      headers: { 'Content-Type': 'application/json' },
      status : 500});
  }
}