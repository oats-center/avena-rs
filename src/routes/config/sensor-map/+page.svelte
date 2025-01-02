<script lang='ts'>
  import { onMount } from "svelte";
  

  import { connect, getKeyValue, NatsService, putKeyValue } from '$lib/nats.svelte';
  
  import SaveModal from "$lib/components/modals/SaveModal.svelte";
  import CancelModal from "$lib/components/modals/CancelModal.svelte";
  import DeleteModal from "$lib/components/modals/DeleteModal.svelte";
  import Alert from "$lib/components/Alert.svelte";
  import SensorMap from "$lib/components/SensorMap.svelte";
  import MapControls from "$lib/components/MapControls.svelte";

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

  interface SensorType {
    "size" : number;
    "svg" : string;
  }

  interface SensorTypes {
    [name: string]: SensorType
  }

  //generals
  // svelte-ignore non_reactive_update
  let selectedCabinet: string | null;
  let serverName: string | null; 
  let nats: NatsService | null;   
  let loading = $state<boolean>(true);
    
  //from nats
  let mapconfig: MapConfig;
  let sensors = $state<Sensor[]>([]);
  let backgroundImage = $state<string | null>(null);
  
  let editingSensor = $state<Sensor | null>(null);
  let editingIndex= $state<number>(-1);
  let sensorSize= $state<number>(40);
  let queuedIndex = -1;
          
  let alert = $state<string | null>(null);  
  let cancel_modal = $state<HTMLDialogElement>();
  let delete_modal = $state<HTMLDialogElement>();
  let save_modal = $state<HTMLDialogElement>();  
    
  
  
  
  //main: gets values from nats and parses
  async function initialize(): Promise<void> {
    if(serverName) nats = await connect(serverName);
    if(nats && selectedCabinet) {
      /* putKeyValue(nats, "road1_cabinet1", "mapconfig", JSON.stringify({
         "backgroundImage" : "",
         "labjackd.1.ch1" : {
          "cabinet_id" : "road1_cabinet1",
          "color" :  "red", 
          "connected_channel" : "1",
          "labjack_serial" : "1",
          "sensor_name" : "Sensor 1",
          "sensor_type" : "Temperature",
          "x_pos" : 0.5,
          "y_pos" : 0.5
        },
        "labjackd.1.ch2" : {
          "cabinet_id" : "road1_cabinet1",
          "color" :  "red", 
          "connected_channel" : "2",
          "labjack_serial" : "1",
          "sensor_name" : "Sensor 2",
          "sensor_type" : "Temperature",
          "x_pos" : 0.75,
          "y_pos" : 0.75
        }
      })) */
      
      //gets the values from NATS   
      let tempMapConfig = await getKeyValue(nats, selectedCabinet, "mapconfig");
      if(tempMapConfig !== "Key value does not exist"){
        console.log(tempMapConfig)
        console.log("MapConfig Exists")
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
      loading = false
    }
  }

  //main: saves sensor changes to nats
  function saveSensorChanges(): void {
    if(!nats || !selectedCabinet || !editingSensor || editingIndex === -1) throw new Error("Something went wrong with saving changes");
    
    sensors[editingIndex] = editingSensor;
    mapconfig[`labjackd.${editingSensor.labjack_serial}.ch${editingSensor.connected_channel}`] = editingSensor;

    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
    
    editingSensor = null;
    editingIndex = -1;
    save_modal?.close();
  }
  
  //main: cancels changes depending on the state of the editing sensor
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

  //main: deletes a sensor from the sensors array and nats
  function deleteSensor(): void {
    if(!nats || !selectedCabinet || !editingSensor || editingIndex === -1) throw new Error("Something went wrong with saving changes");
    sensors.splice(editingIndex, 1);
    delete mapconfig[`labjackd.${editingSensor.labjack_serial}.ch${editingSensor.connected_channel}`];
    editingIndex = -1;
    editingSensor = null;
    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
    
  }

  //main: saves changes to the background to nats 
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
</script>

{#if loading} <!-- While loading nats data -->
<div class="loading-overlay">
  <span class="loading loading-spinner loading-lg"></span>  
</div>
{:else}
<div class='flex justify-center items-center h-screen'>
  <!--Map Area-->
  <div class="relative flex justify-center items-center w-3/4 h-screen ">
    {#if backgroundImage && sensors} <!-- Checks for valid mapconfig -->
      <SensorMap
        {sensors}
        {editingSensor}
        {editingIndex}
        {sensorSize}
        {backgroundImage}
        {handleSensorChanges}
      />
    {:else} <!-- Only if invalid mapconfig -->
      <h1>No MapConfig Has Been Created</h1>
      <h3 class="text-primary">Start By Importing a Backgroud Image</h3>
    {/if}
  </div>

  <!--Configuration Area-->
  <div class="w-1/4">
    {#if cancel_modal && delete_modal && save_modal} <!-- Typescript checking -->
      <MapControls
        {selectedCabinet}
        {sensors}
        {editingSensor}
        {editingIndex}
        {sensorSize}
        {backgroundImage}
        {cancel_modal}
        {delete_modal}
        {save_modal}
        {alert}
        {saveBackgroundChanges}
      />
    {/if}
  </div>
</div>

{/if}

<SaveModal bind:save_modal={save_modal} {saveSensorChanges}/>
<CancelModal bind:cancel_modal={cancel_modal} {handleSensorChanges}/>
<DeleteModal bind:delete_modal={delete_modal} deleteFunction={deleteSensor} delete_string="sensor" confirmation_string={editingSensor?.sensor_name}/>
<Alert bind:alert={alert}/>
