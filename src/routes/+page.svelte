<script lang="ts">
  import { browser } from '$app/environment';
  import { NatsService } from '$lib/nats';
  import { wsconnect } from "@nats-io/nats-core";
  import { setContext } from 'svelte';
 //use session storage to save the connection ID, which will be used when in the config and map pages
  let serverName = $state<string>("");
  let password = $state<string>("");
  let nats = $state<NatsService | null>(null)

  async function connect() {
    sessionStorage.setItem("serverName", serverName)
    location.href = "/cabinet-select";
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

