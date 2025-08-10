<script lang='ts'>
  import { goto } from "$app/navigation";
  import { NatsService, putKeyValue } from "$lib/nats.svelte";

  import TypeModal from "./basic_modals/TypeModal.svelte";
  import type { Sensor, SensorType, SensorTypes } from "$lib/MapTypes"

  let {nats, selectedCabinet, sensors = $bindable(), editingSensor = $bindable(), editingIndex = $bindable(), sensor_types, context_position = $bindable(), type_modal = $bindable(), background, saveBackgroundChanges, handleManualSelect, saveSensorChanges} : 
  {
    nats: NatsService | null,
    selectedCabinet: string | null,
    sensors: Sensor[],
    editingSensor: Sensor | null,
    editingIndex: number,
    sensor_types: SensorType[] | null,
    context_position: number[],
    type_modal: HTMLDialogElement | undefined,
    background: HTMLImageElement | null,
    saveBackgroundChanges: Function,
    handleManualSelect: Function,
    saveSensorChanges: Function,
  } = $props()
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

  let fileInput = $state<HTMLInputElement>();
  let selectedLabjack = $state<string>("Labjack");
  let selectedChannel = $state<string>("Channel");
  
  let editing_type = $state<SensorType>({"name": "", "icon": "", "size_px": 30});
  let editing_type_index = -1;
  let newType = $state<boolean>(true);
  let clickedElement = false;

  let dragging: boolean;
  let newSensor = $state<Sensor | null>(null)
  let newIndex = $state<number>(-1);

  //reads the file from the file input
  function readFile(): void {
    if(!fileInput) return;

    if(fileInput !== null && fileInput.files){
      const file = fileInput.files[0];
      const reader = new FileReader();
    
      reader.addEventListener("load", () => {
        if(typeof reader.result === "string"){
          saveBackgroundChanges(reader.result);
          fileInput!.value = "";
        }
      });
      reader.readAsDataURL(file);
    } else {
      console.log("No file input")
    }  
  }

  //handles adding a new sensor Type
  function addSensorType(sensor_type: SensorType): void {
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

    editing_type = {"name": "", "icon": "", "size_px": 30}
    newType = true;
    type_modal?.close()
  }

  //formats the sensor type for NATS
  function formatSensorTypes(): SensorTypes  {
    let formatted_types: SensorTypes = {}
    sensor_types?.forEach((type) => {
      formatted_types[type.name] = {
        "icon": type.icon,
        "size_px": type.size_px
      }
    })

    return formatted_types;
  }

  //when mouse down on a sensor, start dragging
  function handleTypeDragStart(e: MouseEvent, type: SensorType, index: number): void {
    if (!background) throw new Error("Background Doesn't Work");
    if (!selectedCabinet) throw new Error("No Cabinet Selection")
    
    if(e.which === 1) {
      dragging = true;
      newSensor = {
        "cabinet_id": selectedCabinet,
        "labjack_serial" : "",
        "connected_channel": "",
        "sensor_name" : "", 
        "sensor_type" : type.name, 
        "x_pos" : e.clientX,
        "y_pos" : e.clientY, 
        "color" : "black", 
      }
      newIndex = index
    }
  }
  
  //when the mouse moves, continue dragging
  function continueTypeDrag(e: MouseEvent): void {
    if (!background || !newSensor || !dragging) return;
    newSensor.x_pos = e.clientX;
    newSensor.y_pos = e.clientY;
  }

  //when the mouse button is back up, stop dragging and round values
  function stopTypeDrag(): void {
    dragging = false;
    if (!newSensor || !background) return;
  
    const back_loc = background.getBoundingClientRect()

    if(  newSensor.x_pos >= back_loc.x && newSensor.x_pos <= back_loc.x +  background.width 
      && newSensor.y_pos >= back_loc.y && newSensor.y_pos <= back_loc.y + background.height){
        const scaleX = 100 / background.width;
        const scaleY = 100 / background.height;
        
        const back_loc = background.getBoundingClientRect()

        newSensor.x_pos = (newSensor.x_pos - back_loc.x) * scaleX;
        newSensor.y_pos = (newSensor.y_pos - back_loc.y) * scaleY;

        newSensor.x_pos = Math.round(newSensor.x_pos)
        newSensor.y_pos = Math.round(newSensor.y_pos)

        sensors.push(JSON.parse(JSON.stringify(newSensor)));
        newSensor = null;
        editingIndex = sensors.length - 1
        editingSensor = sensors[editingIndex]
    } else {
      newSensor = null;
    }
  }
  
  //moves te context box out of window if clicked out of
  function handleWindowClick(){
    context_position = [-1000, -1000]; 
  }

  //moves context box into view and handles contents
  function handleWindowContext(event: MouseEvent){
    // Only show context menu if we're actually clicking on a sensor type element
    if (!clickedElement) {
      return; // Don't show context menu for background clicks
    }
    
    event.preventDefault(); 
    context_position = [event.clientX, event.clientY];
    clickedElement = false;
  }

</script>
<div class="flex flex-col items-center space-y-4 w-full">
  <!-- NavBar -->
  <div class="w-full text-center -mt-8">
    <h1 class="text-xl font-semibold text-white mb-4">Map Configuration</h1>
    <div class="flex justify-center space-x-4">
      <button 
        class="px-4 py-2 bg-white/10 backdrop-blur-lg border border-white/20 rounded-lg text-white hover:bg-white/20 transition-all duration-200"
        onclick={() => goto("/config/cabinet-select")}
      >
        ← Back to Cabinet
      </button>
      <button 
        class="px-4 py-2 bg-white/10 backdrop-blur-lg border border-white/20 rounded-lg text-white hover:bg-white/20 transition-all duration-200"
        onclick={() => goto("lj-config")}
      >
        Card View
      </button>
    </div>
  </div>

  <!-- New Sensors -->
  <div class="w-full bg-white/5 backdrop-blur-lg rounded-2xl border border-white/10 p-4">
    <div class="text-center mb-4">
      <h4 class="text-lg font-medium text-white mb-3">New Sensor</h4>
      <button 
        class="px-4 py-2 bg-yellow-500/20 border border-yellow-500/30 rounded-lg text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200"
        onclick={() => type_modal?.showModal()}
      >
        New Type
      </button>
    </div>
    <div class="grid grid-cols-3 gap-4">
      {#if sensor_types}
        {#each sensor_types as type, index}
        <div class="flex flex-col justify-center items-center h-full" role="button" tabindex=0 oncontextmenu={(e) => {e.preventDefault(); editing_type = JSON.parse(JSON.stringify(type)); editing_type_index = index; newType = false; clickedElement = true}}>
          <div
            role="button"
            tabindex=0
            class="flex flex-col justify-center items-center"
            onmousedown={(event) => {handleTypeDragStart(event, type, index)}}
            onmouseup={() => {if(newIndex === index) stopTypeDrag()}}
            onmousemove={(event) => {if (newIndex === index) continueTypeDrag(event)}}
            style={(newSensor && index === newIndex) ? `
              position: fixed; 
              top: ${newSensor.y_pos - type.size_px / 2}px;
              left: ${newSensor.x_pos - type.size_px / 2}px;
              min-width: ${type.size_px}px;
              min-height: ${type.size_px}px;
            ` : `
              height: 100%;
              width: 100%;
            `}
          >
            <img src={type.icon} alt="sensor icon" style={`width: ${type.size_px}px; height: ${type.size_px}px;`} draggable={false}/>
          </div>
          <div style={(newSensor && index === newIndex) ? `
            height: 100%;
            width: 100%;
            min-width: ${type.size_px}px;
            min-height: ${type.size_px}px;
          ` : `
            height: 0;
            width: 100%;
          `}>

          </div>
          <h6 class="text-sm text-gray-300 text-center w-full mt-2 truncate">{type.name}</h6>
        </div>
        {/each}
      {/if}
    </div>
  </div>
  
  <!-- Manual Sensor Select -->
  <div class="w-full bg-white/5 backdrop-blur-lg rounded-2xl border border-white/10 p-4">
    <div class="text-center mb-4">
      <h4 class="text-lg font-medium text-white mb-3">Select Sensor</h4>
      <div class="grid grid-cols-2 gap-3">
        <select class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" bind:value={selectedLabjack}>
          <option disabled selected>Labjack</option>
          {#each labjackArray as labjack}
          <option>{labjack}</option>
          {/each}
        </select>
        <select disabled={selectedLabjack === "Labjack"} class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 disabled:opacity-50" bind:value={selectedChannel}>
          <option disabled selected>Channel</option>
          {#each channelArray as channel}
          <option>{channel}</option>
          {/each}
        </select>
        <button class="px-4 py-2 bg-gray-600/50 border border-gray-500/50 rounded-lg text-gray-300 hover:bg-gray-500/50 transition-all duration-200" onclick={() => {selectedLabjack = "Labjack"; selectedChannel = "Channel"}}>Reset</button>
        <button class="px-4 py-2 bg-yellow-500/20 border border-yellow-500/30 rounded-lg text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200 disabled:opacity-50" disabled={selectedLabjack === "Labjack" && selectedChannel === "Channel"} onclick={() => handleManualSelect(selectedLabjack, selectedChannel)}>Select</button>
      </div>
    </div>
  </div>

  <!-- Background Image Change -->
  <div class="w-full bg-white/5 backdrop-blur-lg rounded-2xl border border-white/10 p-4">
    <div class="text-center mb-4">
      <h4 class="text-lg font-medium text-white mb-3">Change Background Image</h4>
      <div class="grid grid-cols-1 gap-3">
        <input type="file" class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white file:mr-4 file:py-2 file:px-4 file:rounded-lg file:border-0 file:text-sm file:font-medium file:bg-yellow-500/20 file:text-yellow-300 hover:file:bg-yellow-500/30" accept="image/png, image/jpg" bind:this={fileInput}/>
        <div class="grid grid-cols-2 gap-3">
          <button class="px-4 py-2 bg-gray-600/50 border border-gray-500/50 rounded-lg text-gray-300 hover:bg-gray-500/50 transition-all duration-200" onclick={() => {fileInput!.value = ""}}>Cancel</button>
          <button class="px-4 py-2 bg-yellow-500/20 border border-yellow-500/30 rounded-lg text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200" onclick={readFile}>Save</button>
        </div>
      </div>
    </div>      
  </div>
</div>


<svelte:window onclick={handleWindowClick} oncontextmenu={handleWindowContext} onmousemove={(e) => {if (newSensor) continueTypeDrag(e)}}/>

<dialog id="type_modal" class='modal' bind:this={type_modal}>
  <TypeModal 
    {sensor_types}
    bind:editing_type={editing_type}
    {newType}
    {addSensorType}
  />
</dialog>