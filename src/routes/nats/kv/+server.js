import { Kvm, KvWatchInclude } from '@nats-io/kv';
import { connect } from "@nats-io/transport-node";

export async function GET({ url }) {
  const type = url.searchParams.get("type");
  try {
    const nc = await connect({ servers: "demo.nats.io:4222" })
    
    const kvm = new Kvm(nc);
    const kv = await kvm.open("sensorList");
  
    const keyNames = [];
    const keyValues = [];
    const keys = await kv.keys();
    for await (const k of keys) {
      keyNames.push(k);
    }

    for await (const key of keyNames) {
      let val = await kv.get(key);
      keyValues.push(val.string());
    }
    
    if(type === "watchVals"){
      const watch = await kv.watch({
        include: KvWatchInclude.UpdatesOnly,
      });
      for await (const e of watch) {
        console.log(`watch: ${e.key}: ${e.operation} ${e.value ? e.string() : ""}`)
        let index = keyNames.findIndex(key => key === e.key);
        if(index !== -1){
          keyValues[index] = e.string();
        }
        break;
      }
      console.log("a value was changed");
    }

    await nc.close();
    return new Response( JSON.stringify({ keyNames, keyValues }), {
      headers: { "Content-Type": "application/json"},
      status: 200,
    });
  } catch (error) {
    console.error("NATS Error: ", error);
    return new Response(`Error: ${error.message}`, {status : 500});
  }
}

export async function PUT({request}) {
  try {
    const nc = await connect({ servers: "demo.nats.io:4222" });
    
    const { key, newValue } = await request.json()

    const kvm = new Kvm(nc);
    const kv = await kvm.open("sensorList");
    
    await kv.put(key, newValue);
    
  return new Response({
    headers: { "Content-Type": "application/json"},
    status: 200,
  })   
  } catch (error) {
    console.error("NATS error:", error);
    return new Response(`Error: ${error.message} `, {status: 500});
  }
}