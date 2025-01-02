<script lang='ts'>
  import { goto } from "$app/navigation";
  import { slide } from "svelte/transition";

  import SensorControls from './SensorControls.svelte';

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

  let {selectedCabinet, sensors, editingSensor, editingIndex, sensorSize, backgroundImage, cancel_modal, delete_modal, save_modal, alert, saveBackgroundChanges} : {
    selectedCabinet: string | null,
    sensors: Sensor[],
    editingSensor: Sensor | null,
    editingIndex: number,
    sensorSize: number,
    backgroundImage: string | null, 
    cancel_modal: HTMLDialogElement,
    delete_modal: HTMLDialogElement,
    save_modal: HTMLDialogElement,
    alert: string | null,
    saveBackgroundChanges: Function,
  } = $props()

  //main: handles adding a sensor, doesn't get updated in nats until updated
  function addSensor(): void {
    if(!selectedCabinet) throw new Error("Page not properly loaded");
    let newSensor: Sensor = {
      "cabinet_id" : selectedCabinet,
      "labjack_serial" : "0",
      "connected_channel": "0",
      "sensor_name" : "New Sensor", 
      "sensor_type" : "Temperature",
      "x_pos" : 0,
      "y_pos" : 0, 
      "color" : "red" 
    }

    sensors.push(newSensor);
    editingSensor = JSON.parse(JSON.stringify(newSensor));
    editingIndex = sensors.length - 1;
  }

  //controls: reads the file from the file input
  function readFile(): void {
    if(!fileInput) return;

    if(fileInput !== null && fileInput.files){
      const file = fileInput.files[0];
      const reader = new FileReader();
    
      reader.addEventListener("load", () => {
        if(typeof reader.result === "string"){
          console.log(reader.result);
          saveBackgroundChanges(reader.result);
          fileInput!.value = "";
        }
      });
      reader.readAsDataURL(file);
    } else {
      console.log("No file input")
    }  
  }

  const sensorColors = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'grey', 'black'];
  const sensorGroups = ['Temperature', 'Pressure'];
  let fileInput = $state<HTMLInputElement>();
</script>

<div class="relative flex flex-col items-center border-l-2 h-screen">
  <!-- NavBar -->
  <h1>Map Configuration</h1>
  <div class="flex mb-8">
    <div class="mx-10 justify-center">
      <button class="btn btn-primary" onclick={() => goto("/config/cabinet-select")}>{"<-- "}Back to Cabinet Select</button>
    </div>
    <div class="mx-10 justify-center">
      <button class="btn btn-primary" onclick={() => goto("lj-config")}>Card View</button>
    </div>
    <div class="mx-10 justify-center">
      <button class="btn btn-primary" onclick={() => addSensor()}>New Sensor</button>
    </div>
  </div>

  <!-- New Sensors -->
  <div class="flex flex-col justify-center card bg-primary items-center z-0 mb-5 w-5/6">
    <div class="card-body flex">
      <h4 class="text-center mb-2">New Sensor</h4>
    </div>
  </div>

  <!-- Background Image Change -->
  <div class="flex flex-col justify-center card bg-primary items-center z-0 w-5/6">
    <!-- <label for="iconHeight" class="block mt-5">Sensor Size: </label>
    <input type="number" bind:value={sensorSize} id="iconHeight" class="input input-bordered max-w-xs w-full" min=30 max=80/>
    <input type="range" min=30 max=80 bind:value={sensorSize} class="w-full max-w-xs mt-2"/> -->
    <div class="card-body flex">
      <h4 class="text-center mb-2">Change Background Image</h4>
      <input type="file" class="file-input file-input-bordered modal_input" accept="image/png, image/jpg" bind:this={fileInput}/>
      <div class="grid grid-cols-2 gap-4 w-full mt-2">
        <button class="btn btn-outline btn-success" onclick={() => {fileInput!.value = ""}}>Cancel</button>
        <button class="btn btn-outline btn-success" onclick={readFile}>Save</button>
      </div>
    </div>      
  </div>

  <!-- Sensor Controls for Selected Sensor -->
  {#if editingIndex !== -1 && save_modal && cancel_modal && delete_modal}
  <div style="position: absolute; right: 0; top: 0; background-color: #FAF9F6; height: 100vh; width: 100%; z-index: 10;" transition:slide={{duration: 250, axis: "x"}}>
    <SensorControls
      {sensors}
      {editingIndex}
      {editingSensor}
      {alert}
      {sensorColors}
      {sensorGroups}
      {cancel_modal}
      {delete_modal}
      {save_modal}
    />
  </div>
  {/if}
</div>