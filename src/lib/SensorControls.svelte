<script lang='ts'>
  export let currSensor;
  export let sensorColors;
  export let sensorGroups;
  export let onAddSensor;
  export let cancel_modal;
  export let delete_modal;
  export let save_modal;

  let fileInput: HTMLInputElement;

  function readFile() {
    if(fileInput !== null && fileInput.files){
      const file = fileInput.files[0];
      const reader = new FileReader();
    
      reader.addEventListener("load", () => {
        if(typeof reader.result === "string"){
          localStorage.setItem("background", reader.result);
        }
      });
      reader.readAsDataURL(file);
      reader.onloadend = () => {
        window.location.reload();
      }

    } else {
      console.log("No file input")
    }  
  }
</script>

<div id="sensorControls" class="w-2/5 h-3/5">
  <h1 class="text-center text-2xl">Sensor Control Area</h1>
  {#if currSensor}
    <div class="mt-5">
      <div class="justify-center flex flex-col items-center">
        <label for="nameInput">Name:</label>
        <input type="text" bind:value={currSensor.name} id="nameInput" class="input input-bordered w-full max-w-xs mt-2"/>
        <label for="xPosInput" class="block mt-5">Sensor X Position:</label>
        <input type="text" bind:value={currSensor.x_pos} id="xPosInput" class="input input-bordered w-full max-w-xs"/>
        <input type="range" step="0.01" min="0" max='1' bind:value={currSensor.x_pos} class="w-full  max-w-xs mt-2"/>

        <label for="yPosInput" class="block mt-5">Sensor Y Position:</label>
        <input type="text" bind:value={currSensor.y_pos} id="yPosInput" class="input input-bordered max-w-xs w-full"/>
        <input type="range" step="0.01" min="0.0" max='1.0' bind:value={currSensor.y_pos} class="w-full max-w-xs mt-2"/>
        
        <label for="groupSelect" class="block mt-5">Sensor Group:</label>
        <select id="groupSelect" class="select select-bordered w-full max-w-xs mt-2" bind:value={currSensor.group}>
          {#each sensorGroups as group}
            <option value={group}>{group}</option>
          {/each}
        </select>
        <label for="colorSelect" class="block mt-5">Sensor Color:</label>
        <select id="colorSelect" class="select select-bordered w-full max-w-xs mt-2" bind:value={currSensor.color}>
          {#each sensorColors as color}
            <option value={color}>{color}</option>
          {/each}
        </select>

        <label for="layerSelect" class="block mt-5">Sensor Layer:</label>
        <select id="layerSelect" class="select select-bordered w-full max-w-xs mt-2" bind:value={currSensor.layer}>
          {#each [1, 2, 3] as num}
            <option value={num}>{num}</option>
          {/each}
        </select>
      </div>
      <div class="mt-5 flex justify-between">
        <button class="btn btn-primary w-1/4" onclick={ () => cancel_modal.showModal() }>Cancel</button>
        <button class="btn btn-error w-1/4" onclick={ () => delete_modal.showModal() }>Delete</button>
        <button class="btn btn-primary w-1/4" onclick={ () => save_modal.showModal() }>Save</button>
      </div>
    </div>
  
    {:else}
    <div class="flex flex-col justify-center items-center mt-5">
      <button class="btn btn-neutral w-full max-w-xs" onclick={onAddSensor}>Add Sensor</button>
      <h2 class="mt-5">Change Background</h2>
      <input type="file" class="file-input file-input-bordered mt-2" accept="image/png, image/jpg" bind:this={fileInput}/>
      <div class="flex">
        <button class="btn btn-primary mt-2 mr-2" onclick={() => fileInput.value = ""}>Cancel</button>
        <button class="btn btn-primary mt-2" onclick={readFile}>Save</button>
        
      </div>
      
    </div>
  {/if}    
</div>

<style>
  #sensorControls {
    margin-right: 5vw;
    height: 60vh;
    width: 20vw;
  }
</style>
