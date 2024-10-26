import { Kvm } from '@nats-io/kv';
import { connect, type NatsConnection } from "@nats-io/transport-node";
import type { RequestEvent } from "@sveltejs/kit"
import { getKeysList, getKeyValues, watchValues, setupNats } from "$lib/nats/+server.js"
import type { Sensor, NatsService } from "$lib/nats/+server.js"

type SensorDataResponse = {
  sensorsList: string[];
  sensorValues: Sensor[] | null;
}
type PutRequestData = {
  bucket: string;
  key: string;
  newValue: string;
}
type NatsConnections = [string, NatsService]

const connections: NatsConnections[] = [];

function connectionExists(id: string): boolean {
  if(connections){ 
    for(const connection of connections){
      if(connection[0] === id){
        return true;
      }
    }
  }
  return false;
}

export async function GET({ url } : RequestEvent): Promise<Response> {
  const type: string | null = url.searchParams.get("type");
  const connectionId: string | null = url.searchParams.get("connectionId");
  if(connectionId){
    try {
      let nats!: NatsService;
      if(connections == null || !connectionExists(connectionId)){
        nats = await setupNats();
        connections?.push([connectionId, nats])
      } else {
        for(const connection of connections){
          if(connection[0] === connectionId){
            nats = connection[1]
          }
        }
      }
      
      let responseData: SensorDataResponse = { sensorsList: nats.bucketList, sensorValues: null };

      if (type === "watchVals") {
        console.log("Started Watching...");
        nats.currentValues = await watchValues(nats.kvm, nats.bucketList, nats.currentValues || []);
        responseData.sensorValues = nats.currentValues;
      } else {
        // Get sensor values if type is not "watchVals"
        nats.currentValues = await getKeyValues(nats.kvm, nats.bucketList);
        responseData.sensorValues = nats.currentValues;
      }
      
      return new Response(JSON.stringify(responseData), {
        headers: { "Content-Type": "application/json" },
        status: 200,
      });
    } catch (error: unknown) {
      console.error("NATS Error: ", error);
      return new Response(JSON.stringify({ error: (error as Error).message }), {
        headers: { 'Content-Type': 'application/json' },
        status: 500
      });
    }
  } else {
    return new Response(JSON.stringify({error: "No Connection ID"}), {
      headers: { "Content-Type": "application/json" },
      status: 500
    })
  }
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