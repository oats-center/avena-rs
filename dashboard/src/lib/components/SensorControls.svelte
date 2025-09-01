<script lang='ts'>
  import type {Sensor, SensorType} from "$lib/MapTypes"
    import { slide } from "svelte/transition";

  //variables that will not change in this file
  let {cancel_modal, delete_modal, save_modal, sensors, editingIndex, editingSensor, sensor_types, alert = $bindable(), handleSensorChanges}:
      {
        cancel_modal: HTMLDialogElement, 
        delete_modal: HTMLDialogElement, 
        save_modal: HTMLDialogElement, 
        sensors: Sensor[], 
        editingIndex: number,
        editingSensor: Sensor | null,
        sensor_types: SensorType[],
        alert: string | null,
        handleSensorChanges: Function,
      } = $props();
    
  const sensorColors = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'grey', 'black'];

  //form validation for mapconfig controls
  function handleSave(): void {
    if(!editingSensor) throw new Error ("No editing sensor with save confirmation?")
    if(editingSensor.labjack_serial === '0' ||editingSensor.labjack_serial === ""){
      alert = "Serial Number Cannot Be 0 or Empty";
      return;
    }
    if(editingSensor.connected_channel === '0' ||editingSensor.connected_channel === ""){
      alert = "Connected Channel Cannot Be 0 or Empty";
      return;
    }
    if(editingSensor.x_pos < 0 || editingSensor.x_pos > 100) {
      alert = "X Position must be between 0 and 100";
      return;
    }
    if (editingSensor.y_pos < 0 || editingSensor.y_pos > 100) {
      alert = "Y Position must be between 0 and 100";
      return;
    }
    for(let i = 0; i < sensors.length; i++) {
      if(sensors[i].connected_channel == editingSensor.connected_channel && sensors[i].labjack_serial == editingSensor.labjack_serial && editingIndex !== i){
        alert = "Sensor with corresponding channel and labjack already exists"
        return;
      }
    }
    save_modal?.showModal();
  }

  //handles when someone cancels sensor changes
  function handleCancel(): void {
    if(JSON.stringify(editingSensor) !== JSON.stringify(sensors[editingIndex])) cancel_modal?.showModal();
    else handleSensorChanges();
  }

</script>
<div class="w-full" transition:slide={{duration: 250, axis: "x"}}>
  <h3 class="text-xl font-medium text-white text-center mb-6">Sensor Settings</h3>
  {#if editingSensor}
    <div class="grid grid-cols-2 gap-4 mb-6">
      <label for="nameInput" class="text-gray-300 text-sm font-medium">Name:</label>
      <input id="nameInput" type="text" bind:value={editingSensor.sensor_name} class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" placeholder="Sensor Name"/>
    
      <label for="serialNumber" class="text-gray-300 text-sm font-medium">LabJack Serial Number:</label>
      <input id="serialNumber" type="text" bind:value={editingSensor.labjack_serial} class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" placeholder="Labjack Serial"/>
    
      <label for="channelNumber" class="text-gray-300 text-sm font-medium">Connect Channel:</label>
      <input id="channelNumber" type="text" bind:value={editingSensor.connected_channel} class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" placeholder="Channel Number"/>
    
      <label for="groupSelect" class="text-gray-300 text-sm font-medium">Sensor Type:</label>
      <select id="groupSelect" class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" bind:value={editingSensor.sensor_type}>
        {#each sensor_types as type}
          <option value={type.name}>{type.name}</option>
        {/each}
      </select>

      <label for="colorSelect" class="text-gray-300 text-sm font-medium">Sensor Color:</label>
      <select id="colorSelect" class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" bind:value={editingSensor.color}>
        {#each sensorColors as color}
          <option value={color}>{color}</option>
        {/each}
      </select>
    </div>
    <div class="flex justify-between space-x-3">
      <button class="flex-1 px-4 py-2 bg-gray-600/50 border border-gray-500/50 rounded-lg text-gray-300 hover:bg-gray-500/50 transition-all duration-200" onclick={ handleCancel }>Cancel</button>
      <button class="flex-1 px-4 py-2 bg-red-500/20 border border-red-500/30 rounded-lg text-red-300 hover:bg-red-500/30 transition-all duration-200" onclick={ () => delete_modal?.showModal() }>Delete</button>
      <button class="flex-1 px-4 py-2 bg-yellow-500/20 border border-yellow-500/30 rounded-lg text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200" onclick={ handleSave }>Save</button>
    </div>  
  {/if}    
</div>
