<script lang='ts'>
  import { onMount } from "svelte";
  import { slide } from "svelte/transition";
  import { connect, getKeyValue, NatsService, putKeyValue } from '$lib/nats.svelte';
  
  import SensorMap from "$lib/components/SensorMap.svelte";
  import MapControls from "$lib/components/MapControls.svelte";
  import SensorControls from "$lib/components/SensorControls.svelte";
  import SaveModal from "$lib/components/basic_modals/SaveModal.svelte";
  import CancelModal from "$lib/components/basic_modals/CancelModal.svelte";
  import DeleteModal from "$lib/components/basic_modals/DeleteModal.svelte";
  import Alert from "$lib/components/Alert.svelte";

  interface Sensor {
    "cabinet_id" : string;
    "labjack_serial" : string;
    "connected_channel": string; 
    "sensor_name" : string; 
    "sensor_type" : string; 
    "x_pos" : number; 
    "y_pos" : number; 
    "color" : string; 
  }
  interface MapConfig {
    "backgroundImage": string;
    [key: `labjackd.${string}.ch${string}`]: Sensor;
  }
  interface SensorTypes {
    [name: string]: {
      "size_px": number
      "icon": string
    }
  }
  interface SensorType {
    "name": string
    "size_px" : number;
    "icon" : string;
  }


  // svelte-ignore non_reactive_update
  let selectedCabinet: string | null;
  // svelte-ignore non_reactive_update
  let nats: NatsService | null;   
  
  let serverName: string | null; 
  let loading = $state<boolean>(true);
  
  let mapconfig: MapConfig;
  let sensors = $state<Sensor[]>([]);
  let backgroundImage = $state<string | null>(null);
  let sensor_types = $state<SensorType[] | null>(null);

  let editingSensor = $state<Sensor | null>(null);
  let editingIndex= $state<number>(-1);
  let sensorSize= 50
  let queuedIndex = -1;
          
  let alert = $state<string | null>(null);  
  let cancel_modal = $state<HTMLDialogElement>();
  let delete_modal = $state<HTMLDialogElement>();
  let save_modal = $state<HTMLDialogElement>();  
  //gets values from nats and parses
  async function initialize(): Promise<void> {
    if(serverName) nats = await connect(serverName);
    if(nats && selectedCabinet) {
       /* putKeyValue(nats, "road1_cabinet1", "sensor_types", JSON.stringify({
          "pressure" : {
          "icon" : "data:image/svg+xml;base64,PD94bWwgdmVyc2lvbj0iMS4wIiBlbmNvZGluZz0iVVRGLTgiPz4NCjxzdmcgd2lkdGg9IjEwMHB0IiBoZWlnaHQ9IjEwMHB0IiB2ZXJzaW9uPSIxLjEiIHZpZXdCb3g9IjAgMCAxMDAgMTAwIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPg0KIDxwYXRoIGQ9Im01MCAyMi44MDFjMTYuNjk1IDAgMzAuMjI3IDEzLjUzNSAzMC4yMjcgMzAuMjI3IDAgOS44ODI4LTQuNzQyMiAxOC42NTYtMTIuMDcgMjQuMTcybC01LjMwODYtNy4yODEyIDIuMTcxOS0xLjg2NzIgMy42MzY3IDQuOTg4M2M1LjM1OTQtNSA4LjcxODgtMTIuMTE3IDguNzE4OC0yMC4wMTIgMC0xNS4wOTQtMTIuMjgxLTI3LjM3MS0yNy4zNzUtMjcuMzcxcy0yNy4zNzUgMTIuMjc3LTI3LjM3NSAyNy4zNzFjMCA3Ljg3ODkgMy4zNTE2IDE0Ljk4OCA4LjY5NTMgMTkuOTg0bDMuNjI4OS00Ljk5NjEgMi4xNjQxIDEuODc1LTUuMjg5MSA3LjI5M2MtNy4zMjAzLTUuNTE5NS0xMi4wNTUtMTQuMjgxLTEyLjA1NS0yNC4xNTYgMC0xNi42OTEgMTMuNTM1LTMwLjIyNyAzMC4yMy0zMC4yMjd6bS03Ljg1NTUgMTEuMzI0IDguNjcxOSAxNC45MDJjMS44NjMzIDAuMzc4OTEgMy4yNjk1IDIuMDI3MyAzLjI2OTUgNCAwIDIuMjU3OC0xLjgyODEgNC4wODU5LTQuMDg1OSA0LjA4NTktMi4yNTM5IDAtNC4wODU5LTEuODI4MS00LjA4NTktNC4wODU5IDAtMC42NTYyNSAwLjE3MTg4LTEuMjY5NSAwLjQ0OTIyLTEuODIwM2wtNi45Mjk3LTE1LjcwM3oiIGZpbGwtcnVsZT0iZXZlbm9kZCIvPg0KPC9zdmc+DQo=",
          "size_px" : 50
        }
      })) */
      
      //gets the values from NATS   
      let tempMapConfig = await getKeyValue(nats, selectedCabinet, "mapconfig");
      if(tempMapConfig !== "Key value does not exist"){
        mapconfig = JSON.parse(tempMapConfig) as MapConfig
        sensors = Object.entries(mapconfig)
          .filter(([key]) => key !== "backgroundImage")
          .map(([, value]) => value as Sensor);
        backgroundImage = mapconfig.backgroundImage
      } else {
        console.log("No MapConfig")
        mapconfig = {
          backgroundImage: ""
        }
      }

      let tempSensorTypes = await getKeyValue(nats, selectedCabinet, "sensor_types");
      if(tempSensorTypes !== "Key value does not exist"){
        let types_json = JSON.parse(tempSensorTypes) as SensorTypes
        sensor_types = Object.entries(types_json).map(([name, data]) => ({
            name, 
            ...data, 
        }));
        console.log(types_json)
      } else {
        //figure out what to put into this else statement 
      }
      loading = false
    }
  }

  //saves sensor changes to nats
  function saveSensorChanges(): void {
    if(!nats || !selectedCabinet) throw new Error("Something went wrong with saving changes");
    
    if(editingSensor && editingIndex){
      sensors[editingIndex] = editingSensor;
      mapconfig[`labjackd.${editingSensor.labjack_serial}.ch${editingSensor.connected_channel}`] = editingSensor;
    } else {
      sensors.forEach((sensor) => {
        mapconfig[`labjackd.${sensor.labjack_serial}.ch${sensor.connected_channel}`] = sensor;
      })
    }
    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
    
    editingSensor = null;
    editingIndex = -1;
    save_modal?.close();
  }
  
  //cancels changes depending on the state of the editing sensor
  function handleSensorChanges(sensor?: Sensor, index?: number): void {
    // option: used cancel button
    if((index === undefined || sensor === undefined) && queuedIndex === -1) { 
      if(sensors[editingIndex].labjack_serial === "0"){
        sensors.pop()
      }
      editingSensor = null;
      editingIndex = -1;
      console.log("Used Cancel Button");
    
      // option: initial selection
    } else if(index !== undefined && editingSensor === null && sensor !== null && index !== -1) {
      editingSensor = JSON.parse(JSON.stringify(sensor));
      editingIndex = index;
      console.log("Initial Selection");
    
      //option: selected new sensor with no change to currently editing sensor
    } else if (index !== undefined  && editingIndex !== index && JSON.stringify(editingSensor) === JSON.stringify(sensors[editingIndex])){
      editingSensor = JSON.parse(JSON.stringify(sensor));
      editingIndex = index;
      console.log("Selected new sensor with no change to currently editing sensor");
    
      //option: selected new sensor with changes to currently editing sensor
    } else if (index !== undefined  && editingSensor !== null && editingIndex !== index) {
      cancel_modal?.showModal();
      queuedIndex = index;
      console.log(queuedIndex);
      console.log("New sensor clicked on")

    //option: cancel modal confirm from new sensor selection 
    } else if (queuedIndex !== -1) {
      editingSensor = JSON.parse(JSON.stringify(sensors[queuedIndex]));
      editingIndex = queuedIndex;
      queuedIndex = -1;
    }
  }

  //deletes a sensor from the sensors array and nats
  function deleteSensor(): void {
    if(!nats || !selectedCabinet || !editingSensor || editingIndex === -1) throw new Error("Something went wrong with saving changes");
    sensors.splice(editingIndex, 1);
    delete mapconfig[`labjackd.${editingSensor.labjack_serial}.ch${editingSensor.connected_channel}`];
    editingIndex = -1;
    editingSensor = null;
    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
    
  }

  //saves changes to the background to nats 
  function saveBackgroundChanges(background: string): void {
    if (!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    mapconfig.backgroundImage = background;
    backgroundImage = background;
    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
  }

  //sets up the page
  onMount(() => {
    serverName = sessionStorage.getItem("serverName");
    selectedCabinet = sessionStorage.getItem("selectedCabinet");
    initialize();
  })
  
  //selects the sensor based on its labjack and channel
  function handleManualSelect(selectedLabjack: string, selectedChannel: string): void {
    editingIndex = sensors.findIndex((sensor) => 
      sensor.labjack_serial === selectedLabjack &&
      sensor.connected_channel === selectedChannel
    ) ?? null;
    editingSensor = sensors[editingIndex];
  }
</script>

{#if loading} <!-- While loading nats data -->
<div class="loading-overlay">
  <span class="loading loading-spinner loading-lg"></span>  
</div>
{:else}
<div class='h-screen flex justify-center items-center '>
  <!--Map Area-->
  <div class="relative w-3/4 h-screen flex justify-center items-center">
    {#if backgroundImage && sensors && sensor_types} <!-- Checks for valid mapconfig -->
      <SensorMap
        {sensors}
        {editingSensor}
        {editingIndex}
        {sensorSize}
        {backgroundImage}
        {sensor_types}
        {handleSensorChanges}
      />
    {:else} <!-- Only if invalid mapconfig -->
      <h1>No MapConfig Has Been Created</h1>
      <h3 class="text-primary">Start By Importing a Backgroud Image</h3>
    {/if}
  </div>

  <!--Configuration Area-->
  
  <div class="w-1/4">
    <MapControls
      {nats}
      {selectedCabinet}
      {sensors}
      {editingSensor}
      {editingIndex}
      {sensor_types}
      {saveBackgroundChanges}
      {handleManualSelect}
      {saveSensorChanges}
    />
    <!-- Sensor Controls for Selected Sensor -->
    {#if editingIndex !== -1 && save_modal && cancel_modal && delete_modal && sensor_types}
    <div class="sensor_controls" transition:slide={{duration: 250, axis: "x"}}>
      <SensorControls
        {sensors}
        {editingIndex}
        {editingSensor}
        {alert}
        {sensor_types}
        {cancel_modal}
        {delete_modal}
        {save_modal}
        {handleSensorChanges}
      />
    </div>
    {/if}
  </div>
  
</div>

{/if}

<SaveModal bind:save_modal={save_modal} {saveSensorChanges}/>
<CancelModal bind:cancel_modal={cancel_modal} {handleSensorChanges}/>
<DeleteModal bind:delete_modal={delete_modal} deleteFunction={deleteSensor} delete_string="sensor" confirmation_string={editingSensor?.sensor_name}/>
<Alert bind:alert={alert}/>





<style>
  .sensor_controls {
    position: absolute; 
    right: 0; 
    top: 0; 
    background-color:#FAF9F6; 
    width: 25%; 
    height: 100vh; 
    z-index: 10;
  }
</style>