import { Kvm, KvWatchInclude } from '@nats-io/kv';
import type { KV } from "@nats-io/kv";
import {connect, type NatsConnection} from "@nats-io/transport-node"
export type Sensor = {
  id: string;
  name: string;
  type: string;
  value: string;
}

export class NatsService {
  constructor(
    public connection: NatsConnection, 
    public kvm: Kvm,
    public bucketList: string[], 
    public currentValues: Sensor[] | null = null
  ){}
}

export async function setupNats(serverName: string): Promise<NatsService> {
  const nc = await connect({servers: serverName});
  const kvm = new Kvm(nc);
  const kv = await kvm.open("sensorList");
  const bucketList = await getKeysList(kv);

  return new NatsService(nc, kvm, bucketList);
}

export async function getKeysList(kv: KV): Promise<string[]> {
  const sensorsList: string[] = [];
  const keysList = await kv.keys();
  for await(const k of keysList){
    sensorsList.push(k)
  }
  return sensorsList;
}

export async function getKeyValues(kvm: Kvm, sensorsList: string[]):Promise<Sensor[]>  {
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

export async function watchValues(kvm: Kvm, sensorsList: string[], sensorValues: Sensor[]): Promise<Sensor[]> {
  const stopSignal = { stopped: false };

  async function watchSensor(sensor: string, i: number, stopSignal: { stopped: boolean }) {
    const kv = await kvm.open(sensor);
    const watch = await kv.watch({
      include: KvWatchInclude.UpdatesOnly,
    });

    try {
      for await (const e of watch) {
        if (stopSignal.stopped) {
          break; // Stop watching if another watcher has resolved
        }
        console.log(`/nats line 76: watch: ${e.key}: ${e.operation} ${e.value ? e.string() : ""}`);
        console.log(`/nats line 77: past value: ${sensorValues[i][e.key as keyof Sensor]}`);
        sensorValues[i][e.key as keyof Sensor] = e.string();
        stopSignal.stopped = true; // Set the signal to stop all watches
        break;
      }
    } catch (error) {
      console.error("/nats line 86: Error Watching Values: ", error)
    }
  }

  // Create an array of promises for all the sensors
  const sensorPromises = sensorsList.map((sensor, i) => watchSensor(sensor, i, stopSignal));
  await Promise.race(sensorPromises);

  return sensorValues;
} 


/*

data format client -> nats:
road1.labjack1_config
{
  "cabinet_id": "road1",
  "labjack_ip": "111.111.11.1",
  "labjack_name": "cabinet1_t71",
  "sensor_settings": {
    "sampling_rate": 100000,
    "channels_enabled": [0, 2, 3, 4, 5, 7],
    "gains": 10,
    "data_formats": ["float32", "float32", "float32", "float32", "float32", "float32"],
    "measurement_units": ["mV", "mV", "mV", "mV", "mV", "mV", "mV", "mV"],
    “publish_raw_data": [true, true, true, true, true, true, true, true],
     "measure_peaks": [true, true, true, true, true, true, true, true],
     “measure_perm_log” : [true, true, true, true, true, true, true, true],
     “publish_summary_peaks” : true
     "labjack_reset": false
  }
}

*/