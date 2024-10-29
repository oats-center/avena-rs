import { Kvm } from '@nats-io/kv';
import { connect, type NatsConnection } from "@nats-io/transport-node";
import type { RequestEvent } from "@sveltejs/kit"
import { getKeysList, getKeyValues, watchValues, setupNats } from "$lib/nats/+server.js"
import type { Sensor, NatsService } from "$lib/nats/+server.js"

type PutRequestData = {
  connectionId: string,
  serverName: string,
  bucket: string;
  key: string;
  newValue: string;
}
type NatsConnections = [string, NatsService]

const connections: NatsConnections[] = [];

const RESPONSE_MESSAGES = {
  connectionSuccess: JSON.stringify({ body: "Connection Successful" }),
  closingSuccess: JSON.stringify({body: "Connection Closed Successfully"}),
  connectionError: JSON.stringify({ body: "Error Connecting to NATS" }),
  missingValues: JSON.stringify({ error: "Not sent all necessary information" }),
  noServerMatch: JSON.stringify({ error: "No Matching Server"})
};

function getConnection(id: string): NatsService | undefined {
  if(!connections){
    return undefined;
  }
  return connections.find(([connectionId]) => connectionId === id)?.[1];
}

export async function GET({ url } : RequestEvent): Promise<Response> {
  const type: string | null = url.searchParams.get("type");
  const connectionId: string | null = url.searchParams.get("connectionId");
  const serverName: string | null = url.searchParams.get("serverName");
  
  if(!connectionId || !serverName){
    return new Response(RESPONSE_MESSAGES.missingValues, { status: 500 });
  }

  let nats = getConnection(connectionId) || await setupNats(serverName)
  if(!getConnection(connectionId)) {
    console.log("/kv line 44: New Connection")
    connections.push([connectionId, nats]);
  } else {
    console.log("/kv line 47: Already Connected");
  }

  if(type === "initConnection") {
    if(nats){
      return new Response(RESPONSE_MESSAGES.connectionSuccess, { status: 200 });
    } else {
      return new Response(RESPONSE_MESSAGES.connectionError, { status: 500 });
    }
  }

  try {
    if(nats.currentValues === null) {
      nats.currentValues = await getKeyValues(nats.kvm, nats.bucketList);
    }
    nats.currentValues = type === "watchVals" ? await watchValues(nats.kvm, nats.bucketList, nats.currentValues) : await getKeyValues(nats.kvm, nats.bucketList);  
    return new Response(JSON.stringify({ sensorList: nats.bucketList, sensorValues: nats.currentValues }), { status: 200 });
  } catch (error: unknown) {
    console.error("/kv line 63: NATS Error: ", error);
    return new Response(JSON.stringify({ error: (error as Error).message }), { status: 500 });
  }
}

export async function PUT({request}: RequestEvent): Promise<Response> {
  const { connectionId, serverName, bucket, key, newValue }: PutRequestData = await request.json()
  
  if(!connectionId || !serverName || !bucket || !key || !newValue){
    return new Response(RESPONSE_MESSAGES.missingValues, { status: 500 });
  }

  let nats = getConnection(connectionId) || await setupNats(serverName)
  if(!getConnection(connectionId)) {
    connections.push([connectionId, nats]);
  }

  try {
    const kv = await nats.kvm.open(bucket);
    await kv.put(key, newValue);

    return new Response(JSON.stringify({ success: true }), {
      headers: { "Content-Type": "application/json"},
      status: 200,
    });   
  } catch (error: unknown) {
    console.error("/kv line 94: NATS Error: ", error);
    return new Response(JSON.stringify({ error: (error as Error).message} ), { status : 500 });
  }
}

export async function POST({url}: RequestEvent): Promise<Response> {
  const connectionId: string | null = url.searchParams.get("connectionId");
  const serverName: string | null = url.searchParams.get("serverName");
  if(!connectionId || !serverName){
    return new Response(RESPONSE_MESSAGES.missingValues, { status: 500 });
  }

  const index = connections.findIndex(([id]) => id === connectionId);
  if(index === -1) {
    return new Response(RESPONSE_MESSAGES.noServerMatch, {status: 500});
  }

  try {
    const nats = connections[index][1]
    await nats.connection.drain();
    console.log("the server is disconnected");
    connections.splice(index, 1);
    return new Response(RESPONSE_MESSAGES.closingSuccess, {status: 200} );
  } catch (error: unknown) {
    return new Response(JSON.stringify({error: (error as Error).message}), { status: 500 })
  }
    
}
