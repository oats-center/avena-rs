<script lang="ts">
  import { onMount, getContext } from "svelte";
  import { NatsService, connect, getKeys, getCabinet, putKeyValue, type Cabinet } from "$lib/nats";

  let serverName: string | null;
  let nats: NatsService | null;
  let cabinetKeys = $state<string[]>([])
  let cabinets = $state<Cabinet[]>([]);
  let loading = $state<number>(-1);

  
  async function initialize() {
    if(serverName) nats = await connect(serverName)
    if(nats) {
      let keys = await getKeys(nats, "all_cabinets");
      for (const key of keys) {
        cabinetKeys = await getKeys(nats, key);
        let values = await getCabinet(nats, key, cabinetKeys);
        cabinets.push(values);
      }
      loading = 0;
    } else {
      console.log('No Nats Connection');
    }
  }
  
  function selectConfig(selectedCabinet: string) {
    sessionStorage.setItem("cabinet", selectedCabinet)
    location.href = "/lj-config";
  }

  onMount(() => {
    serverName = sessionStorage.getItem("serverName")
    initialize();
  })
</script>

{#if loading === -1}
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
