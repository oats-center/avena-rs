<script lang='ts'>
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
  //variables that will not change in this file
  export let sensorColors: string[];
  export let sensorGroups: string[];
  export let cancel_modal: HTMLDialogElement;
  export let delete_modal: HTMLDialogElement;
  export let save_modal: HTMLDialogElement;
  export let sensors: Sensor[];
  export let editingIndex: number;

  //variables that will change in the file
  export let editingSensor: Sensor | null;
  export let sensorSize: number;
  export let saveBackgroundChanges: Function;
  export let alert: string | null;
  
  //variables just for in this file
  let fileInput: HTMLInputElement;

  //reads the input background files
  function readFile(): void {
    if(fileInput !== null && fileInput.files){
      const file = fileInput.files[0];
      const reader = new FileReader();
    
      reader.addEventListener("load", () => {
        if(typeof reader.result === "string"){
          saveBackgroundChanges(reader.result);
          console.log(reader.result);
          fileInput.value = "";
        }
      });
      reader.readAsDataURL(file);
    } else {
      console.log("No file input")
    }  
  }

  //form validation for mapconfig controls
  function handleSave(): void {
    if(!editingSensor) throw new Error ("No editing sensor with save confirmation?")
    if(editingSensor.labjack_serial === '0' || editingSensor.connected_channel === '0'){
      alert = "Serial number & connected channel cannot be 0";
      return;
    }
    if(editingSensor.x_pos < 0 || editingSensor.x_pos > 1) {
      alert = "X Position must be between 0 and 1";
      return;
    }
    if (editingSensor.y_pos < 0 || editingSensor.y_pos > 1) {
      alert = "Y Position must be between 0 and 1";
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

</script>

<div class="h-full w-80 flex flex-col justify-center item-center">
  <h3 class="mb-0 text-white">Sensor Control Area</h3>
  {#if editingSensor}
    <div class="m-5">
      <div class="justify-center flex flex-col items-center">
        <label for="nameInput">Name:</label>
        <input id="nameInput" type="text" bind:value={editingSensor.sensor_name} class="input input-bordered w-full max-w-xs mt-2"/>

        <label for="serialNumber">LabJack Serial No.:</label>
        <input id="serialNumber" type="text" bind:value={editingSensor.labjack_serial} class="input input-bordered w-full max-w-xs mt-2" min=1/>

        <label for="channelNumber">Connect Channel:</label>
        <input id="channelNumber" type="text" bind:value={editingSensor.connected_channel} class="input input-bordered w-full max-w-xs mt-2" min=1/>

        <label for="xPosInput" class="block mt-5">Sensor X Position:</label>
        <input type="number" bind:value={editingSensor.x_pos} id="xPosInput" class="input input-bordered w-full max-w-xs" step=0.01/>
        <input type="range" step="0.01" min="0" max='1' bind:value={editingSensor.x_pos} class="w-full  max-w-xs mt-2"/>

        <label for="yPosInput" class="block mt-5">Sensor Y Position:</label>
        <input type="number" bind:value={editingSensor.y_pos} id="yPosInput" class="input input-bordered max-w-xs w-full" step=0.01/>
        <input type="range" step="0.01" min="0.0" max='1.0' bind:value={editingSensor.y_pos} class="w-full max-w-xs mt-2"/>
        
        <label for="groupSelect" class="block mt-5">Sensor Group:</label>
        <select id="groupSelect" class="select select-bordered w-full max-w-xs mt-2" bind:value={editingSensor.sensor_type}>
          {#each sensorGroups as group}
            <option value={group}>{group}</option>
          {/each}
        </select>
        <label for="colorSelect" class="block mt-5">Sensor Color:</label>
        <select id="colorSelect" class="select select-bordered w-full max-w-xs mt-2" bind:value={editingSensor.color}>
          {#each sensorColors as color}
            <option value={color}>{color}</option>
          {/each}
        </select>
      </div>
      <div class="mt-5 flex justify-between">
        <button class="btn btn-primary w-1/4" onclick={ () => cancel_modal?.showModal() }>Cancel</button>
        <button class="btn btn-error w-1/4" onclick={ () => delete_modal?.showModal() }>Delete</button>
        <button class="btn btn-primary w-1/4" onclick={ handleSave }>Save</button>
      </div>
    </div>
  
    {:else}
    <div class="flex flex-col justify-center items-center">
      <label for="iconHeight" class="block mt-5">Sensor Color</label>
      <input type="number" bind:value={sensorSize} id="iconHeight" class="input input-bordered max-w-xs w-full" min=30 max=80/>
      <input type="range" min=30 max=80 bind:value={sensorSize} class="w-full max-w-xs mt-2"/>
      
      <label for="backgroundInput" class="block mt-5">Change Background Image</label>
      <input type="file" class="file-input file-input-bordered mt-2" accept="image/png, image/jpg" bind:this={fileInput}/>
      <div class="flex">
        <button class="btn btn-primary mt-2 mr-2" onclick={() => fileInput.value = ""}>Cancel</button>
        <button class="btn btn-primary mt-2" onclick={readFile}>Save</button>
        
      </div>
      
    </div>
  {/if}    
</div>
