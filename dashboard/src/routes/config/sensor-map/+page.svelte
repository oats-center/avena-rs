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
<div class="min-h-screen bg-gradient-to-br from-gray-900 via-gray-800 to-black">
  <!-- Header -->
  <div class="bg-white/5 backdrop-blur-lg border-b border-white/10">
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
      <div class="flex items-center justify-between h-16">
        <!-- Logo and Title -->
        <div class="flex items-center space-x-4">
          <div class="w-8 h-8 bg-gradient-to-br from-yellow-400 to-yellow-600 rounded-lg flex items-center justify-center">
            <svg class="w-5 h-5 text-white" fill="currentColor" viewBox="0 0 20 20">
              <path d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
          </div>
          <h1 class="text-xl font-semibold text-white">Avena-OTR Dashboard</h1>
        </div>
        
        <!-- Page Title -->
        <div class="flex items-center space-x-3">
          <h2 class="text-lg font-medium text-gray-300">Sensor Map Configuration</h2>
          <div class="w-2 h-2 bg-green-400 rounded-full animate-pulse"></div>
          <span class="text-sm text-gray-300">Connected to NATS</span>
        </div>
      </div>
    </div>
  </div>

  <!-- Main Content -->
  <div class="h-[calc(100vh-4rem)] flex">
    <!--Map Area-->
    <div class="relative w-3/4 h-full flex justify-center items-center p-4">
      {#if backgroundImage} <!-- Checks for valid mapconfig -->
        <div class="w-full h-full bg-white/5 backdrop-blur-lg rounded-2xl border border-white/10 overflow-hidden">
          <SensorMap
            {sensors}
            {editingSensor}
            {editingIndex}
            {sensor_types}
            {backgroundImage}
            bind:background={background}
            {handleSensorChanges}
          />
        </div>
      {:else} <!-- Only if invalid mapconfig -->
        <div class="w-full h-full bg-white/5 backdrop-blur-lg rounded-2xl border border-white/10 flex flex-col items-center justify-center">
          <div class="text-center">
            <div class="inline-flex items-center justify-center w-20 h-20 bg-gray-500/20 rounded-full mb-6">
              <svg class="w-10 h-10 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 20l-5.447-2.724A1 1 0 013 16.382V5.618a1 1 0 011.447-.894L9 7m0 13l6-3m-6 3V7m6 10l4.553 2.276A1 1 0 0021 18.382V7.618a1 1 0 00-1.447-.894L15 4m0 13V4m-6 3l6-3"/>
              </svg>
            </div>
            <h1 class="text-2xl font-bold text-white mb-2">No MapConfig Has Been Created</h1>
            <h3 class="text-gray-300 text-lg">Start By Importing a Background Image</h3>
          </div>
        </div>
      {/if}
    </div>

    <!--Configuration Area-->
    <div class="w-1/4 h-full p-4 space-y-4">
      <div class="bg-white/5 backdrop-blur-lg rounded-2xl border border-white/10 p-4">
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
      </div>

      <!-- Sensor Controls for Selected Sensor -->
      {#if editingIndex !== -1 && save_modal && cancel_modal && delete_modal && sensor_types}
        <div class="bg-white/5 backdrop-blur-lg rounded-2xl border border-white/10 p-4">
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
</div>
{/if}

<SaveModal bind:save_modal={save_modal} {saveSensorChanges}/>
<CancelModal bind:cancel_modal={cancel_modal} {handleSensorChanges}/>
<DeleteModal bind:delete_modal={delete_modal} deleteFunction={deleteSensor} delete_string="sensor" confirmation_string={editingSensor?.sensor_name}/>
<Alert bind:alert={alert}/>

{#if type_modal}
  <ContextMenu top={context_position[1]} left={context_position[0]} {type_modal}/>
{/if}

<style>
  /* Custom scrollbar */
  ::-webkit-scrollbar {
    width: 8px;
  }
  
  ::-webkit-scrollbar-track {
    background: rgba(255, 255, 255, 0.1);
    border-radius: 4px;
  }
  
  ::-webkit-scrollbar-thumb {
    background: rgba(206, 184, 136, 0.5);
    border-radius: 4px;
  }
  
  ::-webkit-scrollbar-thumb:hover {
    background: rgba(206, 184, 136, 0.7);
  }
  
  /* Smooth transitions */
  * {
    transition-property: color, background-color, border-color, text-decoration-color, fill, stroke, opacity, box-shadow, transform, filter, backdrop-filter;
    transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1);
    transition-duration: 150ms;
  }
</style>
