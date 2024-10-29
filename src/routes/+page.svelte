<script lang="ts">
  import { browser } from '$app/environment';
 //use session storage to save the connection ID, which will be used when in the config and map pages
  const connectionId = generateRandomId(); //replace with generateRandomId();
  let serverName = $state<string>("");
  let password = $state<string>("");

  async function connect() {
    console.log(`Connecting to server: ${serverName} of password ${password}`);
    
    try {
      const url = `./nats/kv?type=initConnection&connectionId=${connectionId}&serverName=${serverName}`;
      const response = await fetch(url);
      if(response.ok && browser){
        const result = await response.json();
        sessionStorage.setItem("connectionId", connectionId);
        sessionStorage.setItem("serverName", serverName);
        location.href = "/lj-config"
      } else {
        let result = await response.json()
        console.log(result.error);
      }
    } catch(error) {
      console.error("Error fetching key value: ", error);
    }
  }

  function generateRandomId(): string{
    const length = 5;
    const characters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    let result = '';
    for (let i = 0; i < length; i++) {
      result += characters.charAt(Math.floor(Math.random() * characters.length));
    }
    return result;
  }
</script>

<div class="flex items-center justify-center h-screen ">
  <div class="flex flex-col justify-center text-center items-center">
    <h1 class="text-4xl">Welcome to AvenaOTR!</h1>
    <h2 class="text-2xl pt-5 pb-10">Enter Server Credentials Below:</h2>
    <input type="text" placeholder="Server" bind:value={serverName} class="input input-bordered input-primary  w-72 bg-secondary text-accent placeholder-accent mb-3"/>
    <input type="text" placeholder="Password" bind:value={password} class="input input-bordered input-primary w-72 bg-secondary text-accent placeholder-accent mb-3"/>
    <button class="btn btn-secondary max-w-28" onclick={connect}>Connect</button>
  </div>
</div>

