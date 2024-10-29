<script lang="ts">
    import { connect } from "@nats-io/transport-node";
  import { onMount, onDestroy } from "svelte";
    import { formatDiagnosticsWithColorAndContext } from "typescript";

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
  let connectionId: string | null;//replace with code to pull from session storage;
  let serverName: string | null; //replace with code to pull from session storage;

  function generateRandomId(): string{
    const length = 5;
    const characters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    let result = '';
    for (let i = 0; i < length; i++) {
      result += characters.charAt(Math.floor(Math.random() * characters.length));
    }
    return result;
  }

  async function getSensorValues(): Promise<void> {
    try {
      const url = `./nats/kv?type=getKVs&connectionId=${connectionId}&serverName=${serverName}`;
      console.log(url)
      const response = await fetch(url);
      if(response.ok){
        const result: KeyValueResponse = await response.json();
        sensorsList = result.sensorsList;
        sensorValues = result.sensorValues;
        updatedSensors = JSON.parse(JSON.stringify(result.sensorValues)); // Make a deep copy for tracking changes
      } else {
        let result = await response.json()
        console.log(result.error);
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
            body: JSON.stringify({ connectionId, serverName, bucket: updatedSensor.id, key, newValue: updatedSensor[key as keyof Sensor]})
          });
          const result = await response.json();
          if (response.ok) {
            console.log("Value Changed Successfully")
          } else {
            currSaving = -1;
            console.error("Error changing key values: ", result.error)
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
      const url = `./nats/kv?type=watchVals&connectionId=${connectionId}&serverName=${serverName}`;
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

  onMount(() => {
    const initialize = async () => {
      await getSensorValues();
      await watchValues();
    }
    
    connectionId = sessionStorage.getItem("connectionId");
    serverName = sessionStorage.getItem("serverName");
    console.log("ConnectionId" + connectionId)
    console.log(serverName)

    initialize();

    const handleBeforeUnload = () => {
      navigator.sendBeacon(`./nats/kv?connectionId=${connectionId}&serverName=${serverName}`);
    }
    window.addEventListener("beforeunload", handleBeforeUnload)

    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
    }
  });

  
  
</script>
<div class="flex justify-center p-10 space-x-16">
  <a href="/sensor-map" class="btn btn-outline btn-primary btn-lg">Sensor Map</a>
  <a href="/lj-config" class="btn btn-outline btn-primary btn-lg">Sensor Config</a>
</div>

<div class="flex flex-col justify-center items-center">
  {#if sensorValues && updatedSensors}
    <div class="m-[5vw] mt-2">
      <div class="flex flex-wrap gap-x-5 gap-y-5 justify-center">
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
