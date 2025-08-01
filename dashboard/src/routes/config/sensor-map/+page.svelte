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
  import ContextMenu from "$lib/components/ContextMenu.svelte";

  import type {MapConfig, Sensor, SensorType, SensorTypes} from "$lib/MapTypes"


  // svelte-ignore non_reactive_update
  let selectedCabinet: string | null;
  // svelte-ignore non_reactive_update
  let nats: NatsService | null;   
  let serverName: string | null; 
  let loading = $state<boolean>(true);
  let context_position = $state<[number, number]>([-1000, -1000]);

  let mapconfig: MapConfig;
  let sensors = $state<Sensor[]>([]);
  let backgroundImage = $state<string | null>(null);
  let sensor_types = $state<SensorType[] | null>(null);
  let background = $state<HTMLImageElement | null>(null);

  let editingSensor = $state<Sensor | null>(null);
  let editingIndex= $state<number>(-1);
  let queuedIndex = -1;
          
  let alert = $state<string | null>(null);  
  let cancel_modal = $state<HTMLDialogElement>();
  let delete_modal = $state<HTMLDialogElement>();
  let save_modal = $state<HTMLDialogElement>();
  let type_modal = $state<HTMLDialogElement>();  
  
  //gets values from nats and parses
  async function initialize(): Promise<void> {
    if(serverName) nats = await connect(serverName);
    if(nats && selectedCabinet) {
      //access mapconfig from NATS   
      let tempMapConfig = await getKeyValue(nats, selectedCabinet, "mapconfig");
      if(tempMapConfig !== "Key value does not exist"){
        mapconfig = JSON.parse(tempMapConfig) as MapConfig
        sensors = Object.entries(mapconfig)
          .filter(([key]) => key !== "backgroundImage")
          .map(([, value]) => value as Sensor);
        backgroundImage = mapconfig.backgroundImage
      } else {
        mapconfig = {
          backgroundImage: ""
        }
      }

      //access sensor types from NATS
      let tempSensorTypes = await getKeyValue(nats, selectedCabinet, "sensor_types");
      if(tempSensorTypes !== "Key value does not exist"){
        let types_json = JSON.parse(tempSensorTypes) as SensorTypes
        sensor_types = Object.entries(types_json).map(([name, data]) => ({
            name, 
            ...data, 
        }));
      } else {
        sensor_types = null; 
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
      if(sensors[editingIndex].sensor_name == "" || sensors[editingIndex].labjack_serial === "" || sensors[editingIndex].connected_channel === ""){
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
    {#if backgroundImage} <!-- Checks for valid mapconfig -->
      <SensorMap
        {sensors}
        {editingSensor}
        {editingIndex}
        {sensor_types}
        {backgroundImage}
        bind:background={background}
        {handleSensorChanges}
      />
    {:else} <!-- Only if invalid mapconfig -->
    <div class="flex flex-col">
      <h1>No MapConfig Has Been Created</h1>
      <h3 class="text-primary">Start By Importing a Backgroud Image</h3>
    </div>
    {/if}
  </div>

  <!--Configuration Area-->
  <div class="w-1/4">
    <MapControls
      {nats}
      {selectedCabinet}
      bind:sensors={sensors}
      bind:editingSensor={editingSensor}
      bind:editingIndex={editingIndex}
      {sensor_types}
      bind:context_position={context_position}
      bind:type_modal={type_modal}
      {background}
      {saveBackgroundChanges}
      {handleManualSelect}
      {saveSensorChanges}
    />

    <!-- Sensor Controls for Selected Sensor -->
    {#if editingIndex !== -1 && save_modal && cancel_modal && delete_modal && sensor_types}
    <div>
      <SensorControls
        {sensors}
        {editingIndex}
        {editingSensor}
        bind:alert={alert}
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

{#if type_modal}
  <ContextMenu top={context_position[1]} left={context_position[0]} {type_modal}/>
{/if}
