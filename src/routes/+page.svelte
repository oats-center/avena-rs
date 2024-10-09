<script>
  import { onMount } from "svelte"
  let natsMessage = $state("");
  let sentMessage = $state("");
  let keysList = $state(null);
  let keyValues = $state([])
  let selectedKey = $state(null); 
  let changeTo = $state();

  async function getKeyValues() {
    const response = await fetch("/nats/kv");
    if(response.ok){
      const result = await response.json();
      keysList = result.keyNames;
      keyValues = result.keyValues;
    }
  }

  async function changeKV() {
    console.log(selectedKey + " changed to " + changeTo);
    const response = await fetch("/nats/kv", {
      headers: { "Content-Type" : "application/json"},
      method: "PUT",
      body: JSON.stringify({key: selectedKey, newValue: changeTo})
    });
    selectedKey = null;
    changeTo = null;
  }

  async function watchKeyValues() {
    const response = await fetch("/nats/kv?type=watchVals");
    if(response.ok){
      const result = await response.json();
      keysList = result.keyNames;
      keyValues = result.keyValues;
    }
    watchKeyValues();
  }

  onMount(() => {
    getKeyValues();
    watchKeyValues();
  });
</script>

<div class="flex flex-col justify-center items-center">
  {#if keysList}
    <div class="mt-2">
      <div class="flex space-x-2 mt-5">
        {#each keysList as key, index}
          <div class="card bg-secondary shadow-xl">
            <div class="card-body text-base-100">
              <h2 class="card-title ">{key}</h2>
              <p>{keyValues[index]}</p>
            </div>
          </div>
        {/each}
      </div>
      <div class="mt-5">
        <select class="select select-bordered" bind:value={selectedKey}>
          {#each keysList as key}
            <option>{key}</option>
          {/each}
        </select>
        <input type="text" placeholder="Change Value" class="input input-bordered" bind:value={changeTo}/>
        <button class="btn btn-primary" onclick={changeKV}>Save</button>
      </div>
    </div>
  {:else}
    <span class="loading loading-spinner loading-lg"></span>  
  {/if}
</div>

