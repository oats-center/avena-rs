import { connect } from "@nats-io/transport-node";

export async function GET() {
  try {
    const nc = await connect({ servers: "demo.nats.io:4222" });

    const messagePromise = new Promise((resolve) => {
      const sub = nc.subscribe("hello");

      (async () => {
        for await (const m of sub) {
          resolve(m.string());
          break;
        }
      })();

      nc.publish("hello", "This is a cool message");
    });

    const message = await messagePromise;

    return new Response(JSON.stringify({ message }), {
      headers: { "Content-Type": "application/json" },
      status: 200,
    });
  } catch (error) {
    console.error("NATS Error: ", error);
    return new Response(`Error: ${error.message}`, { status: 500 });
  }
}

export async function POST({request}) {
  try {
    const nc = await connect({servers: "demo.nats.io:4222"});
    
    const { message } = await request.json();
    nc.publish("hello", message);


    return new Response(JSON.stringify({status: "Message sent"}, message), {
      headers: {"Content-Type": "application/json"},
      status: 200
    })
  } catch (error) {
    console.error("NATS Error:", error);
    return new Response(`Error: ${error.message}`, {status: 500});
  }

}