<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { NatsService, connect, getKeys, getKeyValues, putKeyValue } from "$lib/nats";
  
  type LabJack = {
    cabinet_id: string;
    labjack_name: string;
    sensor_settings: {
      sampling_rate: number;
      channels_enabled: number[];
      gains: number;
      data_formats: string[];
      measurement_units: string[];
      publish_raw_data: boolean[];
      measure_peaks: boolean[];
      publish_summary_peaks: boolean;
      labjack_reset: boolean;
    }
  }

  let serverName: string | null = null;
  let nats: NatsService | null = null;
  let selectedCabinet: string | null = null;
  
  onMount(() => {
    serverName = sessionStorage.getItem("serverName");
    selectedCabinet = sessionStorage.getItem("selectedCabinet");
    console.log(`Server Name: ${serverName}, Selected Cabinet: ${selectedCabinet}`);
  });
    /*import type { LabJack } from "$lib/nats/+server.js"
 
  type KeyValueResponse = {
    sensorsList: string[];
    sensorValues: LabJack[];
  }

  let sensorsList = $state<string[] | null>(null); 
  let sensorValues = $state<LabJack[] | null>(null);
  let updatedSensors = $state<LabJack[] | null>(null);
  let sensorKeys = ["cabinet_id", "labjack_name", "sensor_settings"];
  let currSaving = $state<number>(-1);
  let connectionId: string | null;
  let serverName: string | null;

  async function getSensorValues(): Promise<void> {
    try {
      const url = `./nats/kv?type=getKVs&connectionId=${connectionId}&serverName=${serverName}`;
      console.log(url)
      const response = await fetch(url);
      if(response.ok){
        const result: KeyValueResponse = await response.json();
        sensorsList = result.sensorsList;
        sensorValues = result.sensorValues;
        console.log(sensorValues);
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
            if(currentSensor[key as keyof LabJack] != updatedSensor[key as keyof LabJack]){
              break;
            }
          }
          const response = await fetch("/nats/kv", {
            headers: { "Content-Type" : "application/json"},
            method: "PUT",
            body: JSON.stringify({ connectionId, serverName, bucket: updatedSensor.labjack_name, key, newValue: updatedSensor[key as keyof LabJack]})
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
      console.log("hello")
      await getSensorValues();
      //await watchValues();
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
  });*/

  
</script>
<!--<div class="flex justify-center p-10 space-x-16">
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
                <h2 class="card-title">{sensorValues[index].labjack_name}</h2>
              </div>
              {#each sensorKeys as key}
                <p class="pl-2">LabJack {key}:</p>
                <input 
                  type="text" 
                  bind:value={sensor[key as keyof LabJack]} 
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
-->