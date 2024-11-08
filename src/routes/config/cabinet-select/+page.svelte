<script lang="ts">
  import { onMount } from "svelte";
  import { NatsService, connect, getKeys, getKeyValue, type Cabinet } from "$lib/nats";

  let serverName: string | null;
  let nats: NatsService | null;
  let cabinetKeys = $state<string[]>([])
  let cabinets = $state<Cabinet[]>([]);
  let loading = $state<boolean>(true);

  async function getCabinet(nats: NatsService, bucket: string, keys: string[]): Promise<Cabinet>{
    if(!nats) throw new Error("Nats connection is not initialized");
    let cabinet = {
                    id: bucket,
                    labjacks: [],
                    status: ""
                  };
    for await(const key of keys){
      let val = await getKeyValue(nats, bucket, key)
      if(key === "labjacks"){
        cabinet["labjacks"] = JSON.parse(val);
      } else if (key === "status") {
        cabinet["status"] = val;
      } else {
        console.log("Not valid key");
      }
    } 
    return cabinet;
  }

  async function initialize() {
    if(serverName) nats = await connect(serverName)
    if(nats) {
      let keys = await getKeys(nats, "all_cabinets");
      for (const key of keys) {
        cabinetKeys = await getKeys(nats, key);
        let values = await getCabinet(nats, key, cabinetKeys);
        cabinets.push(values);
      }
      loading = false;
    } else {
      console.log('No Nats Connection');
    }
  }
  
  function selectConfig(selectedCabinet: string) {
    sessionStorage.setItem("selectedCabinet", selectedCabinet)
    location.href = "/config/lj-config";
  }

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
          {#each cabinetKeys as key}
            {#if key !== "labjacks"}
              <p class="pl-2 mt-2">{key}: {cabinet[key as keyof Cabinet]}</p>
            {/if}
          {/each}
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
