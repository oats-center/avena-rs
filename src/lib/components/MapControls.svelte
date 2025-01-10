<script lang='ts'>
  import { goto } from "$app/navigation";
  import { NatsService, putKeyValue } from "$lib/nats.svelte";

  import TypeModal from "./basic_modals/TypeModal.svelte";
    import ContextMenu from "./ContextMenu.svelte";

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

  interface SensorType {
    "name": string
    "size_px" : number;
    "icon" : string;
  }
  interface SensorTypes {
    [name: string]: {
      "size_px": number
      "icon": string
    }
  }

  let {nats, selectedCabinet, sensors, editingSensor, editingIndex, sensor_types, saveBackgroundChanges, handleManualSelect, saveSensorChanges} : 
  {
    nats: NatsService | null,
    selectedCabinet: string | null,
    sensors: Sensor[],
    editingSensor: Sensor | null,
    editingIndex: number,
    sensor_types: SensorType[] | null,
    saveBackgroundChanges: Function,
    handleManualSelect: Function,
    saveSensorChanges: Function,
  } = $props()

  let fileInput = $state<HTMLInputElement>();
  let labjackArray = $derived.by(() => {
    const uniqueLabjacks = sensors
    .map(sensor => sensor.labjack_serial)
    .filter((serial, index, self) => self.indexOf(serial) === index);
    return uniqueLabjacks;
  });
  let channelArray = $derived.by(() => {
    let channels = [];
    channels = sensors.filter(sensor => sensor.labjack_serial === selectedLabjack)
    .map(sensor => sensor.connected_channel);
    return channels;
  });
  let selectedLabjack = $state<string>("Labjack");
  let selectedChannel = $state<string>("Channel");
  
  
  let context_position = $state<[number, number]>([-1000, -1000]);
  let type_modal = $state<HTMLDialogElement>();
  let editing_type = $state<SensorType>({"name": "", "icon": "", "size_px": 0});
  let editing_type_index = -1;

  //handles adding a sensor, doesn't get updated in nats until updated
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

  //reads the file from the file input
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

  function addSensorType(sensor_type: SensorType): void { //add code to check for duplicate sensor types and differentiate adding vs editing
    if(!nats || !selectedCabinet) throw new Error("Something went wrong with saving changes");
    if(sensor_type.icon === "") sensor_type.icon = editing_type.icon
    
    if(sensor_types) {
      if(editing_type_index == -1) sensor_types.push(sensor_type)
      else {
        sensors.forEach((sensor) => {
          if(sensor.sensor_type === sensor_types![editing_type_index].name) sensor.sensor_type = sensor_type.name
        })
        sensor_types[editing_type_index] = sensor_type

        //save type changes to nats
        saveSensorChanges()
      } 
    } 
    else sensor_types = [sensor_type] //might have to be bindable @ props()?? 
    putKeyValue(nats, selectedCabinet, "sensor_types", JSON.stringify(formatSensorTypes()));

    editing_type = {"name": "", "icon": "", "size_px": 0}
    type_modal?.close()
  }

  function formatSensorTypes(): SensorTypes  {
    let formatted_types: SensorTypes = {}
    sensor_types?.forEach((type) => {
      formatted_types[type.name] = {
        "icon": type.icon,
        "size_px": type.size_px
      }
    })

    console.log(formatted_types)
    return formatted_types;
  }

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
  {#if sensor_types}
  <div class="flex flex-col justify-center card bg-primary items-center z-0 mb-5 w-5/6">
    <div class="card-body flex">
      <div class='flex justify-center items-center space-x-5'>
        <h4 class="text-center mb-2">New Sensor</h4>
        <button class="btn btn-outline btn-success" onclick={() => type_modal?.showModal()}>New Type</button>
      </div>
      <div class="grid grid-cols-3 gap-10">
        {#each sensor_types as type, index}
        <div class="flex flex-col justify-center items-center w-full" role="button" tabindex=0 oncontextmenu={(e) => {e.preventDefault(); context_position = [e.clientX, e.clientY]; editing_type = JSON.parse(JSON.stringify(type)); editing_type_index = index}}>
          <img src={type.icon} alt="sensor icon" style={`width: ${type.size_px}px; height: ${type.size_px}px;`}/>
          <h6 style="font-weight: 400; text-align: center; width: 100%; margin-right: 0; margin-top: 6px">{type.name}</h6>
        </div>
        {/each}
      </div>
    </div>
  </div>
  {/if}

  <!-- Manual Sensor Select -->
  <div class="flex flex-col justify-center card bg-primary items-center z-0 mb-5 w-5/6">
    <div class="card-body w-full">
      <h4 class="text-center mb-2">Select Sensor</h4>
      <div class="flex justify-center item-center w-full space-x-5">
        <select class="select select-primary modal_input w-1/3" bind:value={selectedLabjack}>
          <option disabled selected>Labjack</option>
          {#each labjackArray as labjack}
          <option>{labjack}</option>
          {/each}
        </select>
        <select disabled={selectedLabjack === "Labjack"} class="select select-primary modal_input w-1/3" bind:value={selectedChannel}>
          <option disabled selected>Channel</option>
          {#each channelArray as channel}
          <option>{channel}</option>
          {/each}
        </select>
      </div>
      <div class="flex justify-center items-center w-full space-x-5">
        <button class="btn btn-outline btn-success" onclick={() => {selectedLabjack = "Labjack"; selectedChannel = "Channel"}}>Reset</button>
        <button class="btn btn-outline btn-success" disabled={selectedLabjack === "Labjack" && selectedChannel === "Channel"} onclick={() => handleManualSelect(selectedLabjack, selectedChannel)}>Select</button>
      </div>
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
</div>

<svelte:window onclick={() => {context_position = [-1000, -1000]}} oncontextmenu={(e) => {e.preventDefault(); context_position = [e.clientX, e.clientY]; console.log("Window Clicked")}} />

<dialog id="cancel_modal" class='modal' bind:this={type_modal}>
  <TypeModal 
    bind:editing_type={editing_type}
    {addSensorType}
  />
</dialog>

{#if type_modal}
  <ContextMenu top={context_position[1]} left={context_position[0]} {type_modal}/>
{/if}