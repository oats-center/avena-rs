<script lang="ts">
  import { onMount } from "svelte";
  import { NatsService, connect, getKeys, getKeyValue } from "$lib/nats.svelte";

  type Cabinet = {
  "id": string
  "status": string
  };

  let serverName: string | null;
  let nats: NatsService | null;
  let cabinetKeys = $state<string[]>([])
  let cabinets = $state<Cabinet[]>([]);
  let loading = $state<boolean>(true);

  //initializes the nats connection and gets all cabinet vals
  async function initialize() {
    if(serverName) nats = await connect(serverName)
    if(nats) {
      let keys = await getKeys(nats, "all_cabinets");
      for (const key of keys) {
        let values = await getCabinet("all_cabinets", key);
        cabinets.push(values);
        console.log(values);
      }
      loading = false;
    } else {
      console.log('No Nats Connection');
    }
  }

  //gets the value of one cabinet
  async function getCabinet(bucket: string, key: string): Promise<Cabinet>{
    if(!nats) throw new Error("Nats connection is not initialized");
    let cabinet = {
      "id": key,
      "status": ""
    };
    let status = await getKeyValue(nats, bucket, key);
    cabinet.status = JSON.parse(status).status
    return cabinet;
  }

  //once selected, sets the selectedCabinet to session storage and redirects to labjack config page
  function selectConfig(selectedCabinet: string) {
    sessionStorage.setItem("selectedCabinet", selectedCabinet)
    location.href = "/config/lj-config";
  }

  //gets the server name from session storage & initalizes
  onMount(() => {
    serverName = sessionStorage.getItem("serverName")
    initialize();
  })
</script>

{#if loading}
  <div class="loading-overlay">
    <span class="loading loading-spinner loading-lg"></span>  
  </div>
{:else if cabinets !== null && cabinetKeys !== null}
  <div class="flex flex-col justify-center items-center">
    <h1 class="my-10 text-4xl">Select Roadside Cabinet</h1>
    <div class="flex space-x-5">
      {#each cabinets as cabinet}
      <div class="card bg-primary shadow-xl text-neutral w-[15vw] min-w-60">
        <div class="card-body">
          <div class="flex justify-center">
            <h2 class="card-title">{cabinet["id"]}</h2>
          </div>
              <p class="pl-2 mt-2">Status: {cabinet.status}</p>
          <div class="mt-3 flex justify-center">
            <button class="btn btn-outline btn-success" onclick={() =>selectConfig(cabinet.id)}>
              Select Cabinet
            </button>
          </div>
        </div>
      </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .loading-overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    background-color: rgba(0, 0, 0, 0.5);
    display: flex;
    justify-content: center;
    align-items: center;
  }
</style>
