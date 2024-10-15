<script lang="ts">
  import { onMount } from "svelte";

  type KeyValueResponse = {
    sensorsList: string[];
    sensorValues: Sensor[];
  }
  type Sensor = {
    id: string;
    name: string;
    type: string;
    value: string;
  }

  let sensorsList = $state<string[] | null>(null); 
  let sensorValues = $state<Sensor[] | null>(null);
  let updatedSensors = $state<Sensor[] | null>(null);
  let sensorKeys = ["name", "type", "value"];
  let currSaving = $state<number>(-1);
  
  async function getSensorsList(): Promise<void> {
    try {
      const response = await fetch("/nats/kv");
      if(response.ok){
        const result: KeyValueResponse = await response.json();
        sensorsList = result.sensorsList;
      }
    } catch (error) {
      console.error("Error fetching key values list: ", error);
    }
  }

  async function getSensorValues(): Promise<void> {
    try {
      const url = "/nats/kv?type=getKVs";
      const response = await fetch(url);
      if(response.ok){
        const result: KeyValueResponse = await response.json();
        sensorsList = result.sensorsList;
        sensorValues = result.sensorValues;
        updatedSensors = JSON.parse(JSON.stringify(result.sensorValues)); // Make a deep copy for tracking changes
      }
    } catch(error) {
      console.error("Error fetching key value: ", error);
    }
  }

  async function changeKV(index: number): Promise<void> {
    currSaving = index;
    if(updatedSensors && sensorValues){
      const updatedSensor = updatedSensors[index];
      const currentSensor = sensorValues[index];
      if(JSON.stringify(currentSensor) !== JSON.stringify(updatedSensor)){
        try {
          let key;
          for(key in currentSensor){
            if(currentSensor[key as keyof Sensor] != updatedSensor[key as keyof Sensor]){
              break;
            }
          }
          const response = await fetch("/nats/kv", {
            headers: { "Content-Type" : "application/json"},
            method: "PUT",
            body: JSON.stringify({ bucket: updatedSensor.id, key, newValue: updatedSensor[key as keyof Sensor]})
          });
          if (response.ok) {
            console.log("Value Changed Successfully")
          }
        } catch (error) {
          console.error("Error changing key values: ", error);
        }
      } else {
        console.log("No change")
        currSaving = -1;
      }
    }
  }

  async function watchValues(): Promise<void> {
    try {
      const url = "/nats/kv?type=watchVals";
      const response = await fetch(url);
      if(response.ok){
        const result: KeyValueResponse = await response.json();
        sensorsList = result.sensorsList;
        sensorValues = result.sensorValues;
        updatedSensors = JSON.parse(JSON.stringify(result.sensorValues)); // Make a deep copy for tracking changes
        currSaving = -1;
        watchValues();
      }
    } catch(error) {
      console.error("Error fetching key value: ", error);
    }
    currSaving = -1;
  }

  onMount(async () => {
    await getSensorsList();
    await getSensorValues();
    await watchValues();

  });
</script>

<div class="flex flex-col justify-center items-center">
  {#if sensorValues && updatedSensors}
    <div class="mt-2 space-y-5">
      <div class="flex space-x-5">
        {#each updatedSensors as sensor, index}
          <div class="card bg-primary shadow-xl text-neutral w-[15vw] min-w-60">
            <div class="card-body">
              <div class="flex justify-center">
                <h2 class="card-title">{sensorValues[index].name}</h2>
              </div>
              {#each sensorKeys as key}
              <p class="pl-2">Sensor {key}:</p>
              <input 
                type="text" 
                bind:value={sensor[key as keyof Sensor]} 
                class="input input-bordered input-accent w-full bg-primary max-w-xs text-accent placeholder-accent"
              />  
              {/each}
              <div class="mt-3 flex justify-center">
                {#if currSaving !== index}
                  <button class="btn btn-accent w-1/2" onclick={() => updatedSensors && sensorValues ? updatedSensors[index] = JSON.parse(JSON.stringify(sensorValues[index])) : null}>Undo</button>
                  <button class="btn btn-accent w-1/2 ml-2" onclick={() => changeKV(index)}>Save</button>
                {:else}
                  <button class="btn btn-accent w-1/2"><span class="loading loading-spinner"></span></button>
                  <button class="btn btn-accent w-1/2 ml-2"><span class="loading loading-spinner"></span></button>
                {/if}
              </div>
            </div>
          </div>
        {/each}
      </div>
    </div>
  {:else}
    <div class="loading-overlay">
      <span class="loading loading-spinner loading-lg"></span>  
    </div>
  {/if}
</div>



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
