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
    .map(sensor => sensor.serial)
    .filter((serial, index, self) => self.indexOf(serial) === index);
    return uniqueLabjacks;
  });
  let channelArray = $derived.by(() => {
    let channels = [];
    channels = sensors.filter(sensor => sensor.serial === selectedLabjack)
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

  //reads file and converts to base64
  async function readFile(): Promise<void> {
    if (!fileInput || !fileInput.files || fileInput.files.length === 0) return;
    
    const file = fileInput.files[0];
    const reader = new FileReader();
    
    reader.onload = (e) => {
      if (e.target?.result) {
        const base64String = e.target.result as string;
        saveBackgroundChanges(base64String);
        fileInput!.value = ""; // Clear the input after upload
      }
    };
    
    reader.readAsDataURL(file);
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
    
    // Only allow drag if labjack is selected
    if (selectedLabjack === "Choose Labjack") {
      console.log("Please select a Labjack first before adding sensors");
      return;
    }
    
    if(e.which === 1) {
      dragging = true;
      
      // Find next available channel for this labjack
      const usedChannels = sensors
        .filter(sensor => sensor.serial === selectedLabjack)
        .map(sensor => parseInt(sensor.connected_channel));
      
      let nextChannel = 1;
      while (usedChannels.includes(nextChannel)) {
        nextChannel++;
      }
      
      newSensor = {
        "cabinet_id": selectedCabinet,
        "labjack_name": `Labjack ${selectedLabjack}`,
        "serial" : selectedLabjack,
        "connected_channel": nextChannel.toString(),
        "sensor_name" : `${type.name} Sensor ${nextChannel}`, 
        "sensor_type" : type.name, 
        "x_pos" : e.clientX,
        "y_pos" : e.clientY, 
        "color" : type.name === "Temperature" ? "red" : "blue", 
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

        newSensor.x_pos = (newSensor.x_pos - back_loc.x) * scaleX;
        newSensor.y_pos = (newSensor.y_pos - back_loc.y) * scaleY;

        newSensor.x_pos = Math.round(newSensor.x_pos)
        newSensor.y_pos = Math.round(newSensor.y_pos)

        // Add sensor to array and save to NATS immediately
        sensors.push(JSON.parse(JSON.stringify(newSensor)));
        editingIndex = sensors.length - 1
        editingSensor = sensors[editingIndex]
        
        // Auto-save the new sensor
        saveSensorChanges();
        
        newSensor = null;
    } else {
      newSensor = null;
    }
  }
  
  //moves te context box out of window if clicked out of
  function handleWindowClick(){
    context_position = [-1000, -1000]; 
  }

  //removes the selected sensor
  function removeSensor(): void {
    if (editingIndex !== -1) {
      sensors.splice(editingIndex, 1);
      editingIndex = -1;
      editingSensor = null;
      saveSensorChanges();
    }
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
<div class="w-full space-y-3">
  <!-- Navigation -->
  <div class="flex flex-col space-y-2">
    <button 
      class="px-3 py-2 bg-white/10 backdrop-blur-lg border border-white/20 rounded-lg text-white hover:bg-white/20 transition-all duration-200 text-sm"
      onclick={() => goto("/config/cabinet-select")}
    >
      ← Back to Cabinet
    </button>
    <button 
      class="px-3 py-2 bg-white/10 backdrop-blur-lg border border-white/20 rounded-lg text-white hover:bg-white/20 transition-all duration-200 text-sm"
      onclick={() => goto("lj-config")}
    >
      Card View
    </button>
  </div>

  <!-- Add Sensors -->
  <div class="bg-white/5 backdrop-blur-lg rounded-lg border border-white/10 p-3">
    <h4 class="text-sm font-medium text-white mb-2">Add Sensors</h4>
    {#if sensor_types && sensor_types.length > 0}
      <div class="grid grid-cols-2 gap-1.5">
        {#each sensor_types.slice(0, 4) as type, index}
        <div 
          class="flex flex-col items-center p-1.5 bg-white/5 rounded border border-white/10 hover:bg-white/10 transition-all cursor-grab active:cursor-grabbing" 
          role="button" 
          tabindex=0 
          onmousedown={(event) => {handleTypeDragStart(event, type, index)}}
          onmouseup={() => {if(newIndex === index) stopTypeDrag()}}
          onmousemove={(event) => {if (newIndex === index) continueTypeDrag(event)}}
          style={(newSensor && index === newIndex) ? `
            position: fixed; 
            top: ${newSensor.y_pos - type.size_px / 2}px;
            left: ${newSensor.x_pos - type.size_px / 2}px;
            width: ${type.size_px}px;
            height: ${type.size_px}px;
            z-index: 1000;
            background: rgba(0,0,0,0.8);
            border: 2px solid #fbbf24;
          ` : ''}
        >
          <img 
            src={type.icon} 
            alt="sensor icon" 
            class="w-5 h-5 mb-0.5" 
            draggable={false}
          />
          <span class="text-xs text-gray-300 text-center truncate w-full">{type.name}</span>
        </div>
        {/each}
      </div>
    {:else}
      <div class="text-center py-2">
        <p class="text-gray-400 text-xs">No sensors available</p>
      </div>
    {/if}
  </div>

  <!-- Select Labjack -->
  <div class="bg-white/5 backdrop-blur-lg rounded-lg border border-white/10 p-3">
    <h4 class="text-sm font-medium text-white mb-2">Select Labjack</h4>
    <div class="space-y-2">
      <select class="w-full px-2 py-1.5 bg-gray-800/50 border border-gray-600/50 rounded text-white text-xs focus:outline-none focus:ring-1 focus:ring-yellow-500/50" bind:value={selectedLabjack}>
        <option disabled selected>Choose Labjack</option>
        {#each labjackArray as labjack}
        <option>{labjack}</option>
        {/each}
      </select>
      <button 
        class="w-full px-2 py-1.5 bg-yellow-500/20 border border-yellow-500/30 rounded text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200 text-xs disabled:opacity-50" 
        disabled={selectedLabjack === "Choose Labjack"}
        onclick={() => saveSensorChanges()}
      >
        Save Location
      </button>
    </div>
  </div>

  <!-- Background Image Upload -->
  <div class="bg-white/5 backdrop-blur-lg rounded-lg border border-white/10 p-3">
    <h4 class="text-sm font-medium text-white mb-2">Background Image</h4>
    <div class="space-y-2">
      <input 
        type="file" 
        class="w-full px-2 py-1.5 bg-gray-800/50 border border-gray-600/50 rounded text-white text-xs file:mr-2 file:py-1 file:px-2 file:rounded file:border-0 file:text-xs file:bg-yellow-500/20 file:text-yellow-300 hover:file:bg-yellow-500/30" 
        accept="image/png, image/jpg, image/svg+xml" 
        bind:this={fileInput}
      />
      <div class="grid grid-cols-2 gap-1.5">
        <button 
          class="px-2 py-1.5 bg-gray-600/50 border border-gray-500/50 rounded text-gray-300 hover:bg-gray-500/50 transition-all duration-200 text-xs" 
          onclick={() => {fileInput!.value = ""}}
        >
          Cancel
        </button>
        <button 
          class="px-2 py-1.5 bg-yellow-500/20 border border-yellow-500/30 rounded text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200 text-xs" 
          onclick={readFile}
        >
          Upload
        </button>
      </div>
    </div>
  </div>

  <!-- Sensor Position Controls -->
  {#if editingIndex !== -1 && editingSensor}
  <div class="bg-white/5 backdrop-blur-lg rounded-lg border border-white/10 p-3">
    <h4 class="text-sm font-medium text-white mb-2">Selected Sensor Position</h4>
    <div class="space-y-2">
      <div class="text-xs text-gray-300 mb-1">{editingSensor.sensor_name || `${editingSensor.sensor_type} ${editingSensor.connected_channel}`}</div>
      <div class="grid grid-cols-2 gap-2">
        <div>
          <label class="text-xs text-gray-400">X Position (%)</label>
          <input 
            type="number" 
            min="0" 
            max="100" 
            step="0.1"
            class="w-full px-2 py-1 bg-gray-800/50 border border-gray-600/50 rounded text-white text-xs focus:outline-none focus:ring-1 focus:ring-yellow-500/50"
            bind:value={editingSensor.x_pos}
            oninput={() => saveSensorChanges()}
          />
        </div>
        <div>
          <label class="text-xs text-gray-400">Y Position (%)</label>
          <input 
            type="number" 
            min="0" 
            max="100" 
            step="0.1"
            class="w-full px-2 py-1 bg-gray-800/50 border border-gray-600/50 rounded text-white text-xs focus:outline-none focus:ring-1 focus:ring-yellow-500/50"
            bind:value={editingSensor.y_pos}
            oninput={() => saveSensorChanges()}
          />
        </div>
      </div>
      <div class="grid grid-cols-2 gap-1.5 mt-2">
        <button 
          class="px-2 py-1.5 bg-red-500/20 border border-red-500/30 rounded text-red-300 hover:bg-red-500/30 transition-all duration-200 text-xs" 
          onclick={removeSensor}
        >
          Remove
        </button>
        <button 
          class="px-2 py-1.5 bg-yellow-500/20 border border-yellow-500/30 rounded text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200 text-xs" 
          onclick={() => saveSensorChanges()}
        >
          Save
        </button>
      </div>
    </div>
  </div>
  {/if}
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