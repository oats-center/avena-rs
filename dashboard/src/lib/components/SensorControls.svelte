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
<div class="controls" transition:slide={{duration: 250, axis: "x"}}>
  <h3 class="text-accent my-5">Sensor Settings</h3>
  {#if editingSensor}
    <div class="grid grid-cols-2 gap-4 m-5 items-center">
      <label for="nameInput" class="text-accent mr-4">Name:</label>
      <input id="nameInput" type="text" bind:value={editingSensor.sensor_name} class="input modal_input w-full max-w-xs" placeholder="Sensor Name"/>
    
      <label for="serialNumber" class="text-accent">LabJack Serial Number:</label>
      <input id="serialNumber" type="text" bind:value={editingSensor.labjack_serial} class="input modal_input w-full max-w-xs mt-2" placeholder="Labjack Serial"/>
    
      <label for="channelNumber" class="text-accent">Connect Channel:</label>
      <input id="channelNumber" type="text" bind:value={editingSensor.connected_channel} class="input modal_input w-full max-w-xs mt-2" placeholder="Channel Number"/>
    
      <label for="groupSelect" class="text-accent">Sensor Type:</label>
      <select id="groupSelect" class="select modal_input w-full max-w-xs mt-2" bind:value={editingSensor.sensor_type}>
        {#each sensor_types as type}
          <option value={type.name}>{type.name}</option>
        {/each}
      </select>

      <label for="colorSelect" class="text-accent">Sensor Color:</label>
      <select id="colorSelect" class="select modal_input w-full max-w-xs mt-2" bind:value={editingSensor.color}>
        {#each sensorColors as color}
          <option value={color}>{color}</option>
        {/each}
      </select>
    </div>
    <div class="flex justify-between mt-auto mb-5 mx-5">
      <button class="btn btn-outline btn-success w-1/4" onclick={ handleCancel }>Cancel</button>
      <button class="btn btn-outline btn-error w-1/4" onclick={ () => delete_modal?.showModal() }>Delete</button>
      <button class="btn btn-outline btn-success w-1/4" onclick={ handleSave }>Save</button>
    </div>  
  {/if}    
</div>

<style>
  .controls {
    position: fixed;
    top: 0;
    right: 0;
    height: 100vh;
    width: 25vw;
    display: flex;
    flex-direction: column;
    background-color:#FAF9F6; 
  }  
</style>