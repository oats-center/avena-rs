<script lang='ts'>
  interface SensorType {
    "name": string
    "size_px" : number;
    "icon" : string;
  }

  let { sensor_types, editing_type = $bindable(), newType, addSensorType } : { sensor_types: SensorType[] | null, editing_type: SensorType, newType: boolean, addSensorType: Function } = $props();
  
  let fileInput = $state<HTMLInputElement>();
  let alert = $state<string>("")
  function confirmType(): void {
    if(!editing_type || !fileInput) return;

    if(editing_type.name.trim() == "") {
      alert = "Sensor Type Must Have a Name"
      return;
    } 
    if(editing_type.size_px < 30 || editing_type.size_px > 80){
      alert = "Sensor Type Size Must Be Between 30 and 80 pixels"
      return;
    }
    if(newType && (fileInput === null || (fileInput.files && fileInput.files.length === 0))){
      alert = "Sensor Type Must have an SVG File Added"
      return;
    }
    if(sensor_types && newType){
      sensor_types.forEach((type) => {
        if(type.name.toLowerCase() == editing_type.name.toLowerCase()){
          alert = "Sensor Type already Exists"
          return;
        }
      })
      if(alert !== "") return;

    }

    if(fileInput !== null && fileInput.files && fileInput.files[0]){
      const file = fileInput.files[0];
      const reader = new FileReader();

      reader.addEventListener("load", () => {
        if(typeof reader.result === "string"){
          fileInput!.value = "";
          editing_type.icon = reader.result
          addSensorType(editing_type)
        }
      });
      reader.readAsDataURL(file);
      
    } else {
      console.log("No file input")
      addSensorType(editing_type)
    }
  }

</script>

  {#if editing_type}
  <div class="modal-box bg-primary">
    <h3 class="text-lg font-bold mb-1">New Sensor Type</h3>
    <h6 class="text-red-600 mb-2">{alert}</h6>
    <div class="grid grid-cols-2">
      <div>
        <h6>Name: </h6>
        <input type="text" class="input modal_input" bind:value={editing_type.name} placeholder="New Type Name"/>
      </div>
      <div>
        <h6>Size: </h6>
        <input type="number" min="30" max="80" class="input modal_input w-full" bind:value={editing_type.size_px} />
      </div>
    </div>
    <div class="flex items-center mt-5">
      <h6 class="mr-5">Icon: </h6>
      <input type="file" class="file-input file-input-bordered modal_input w-full" accept="image/svg" bind:this={fileInput}/>
    </div>
    
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-outline btn-success" onclick={() => editing_type = {"name": "", "icon": "", "size_px": 30}}>Cancel</button>
      </form>
      <button class="btn btn-outline btn-error ml-5" onclick={confirmType}>Save</button>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button onclick={() => editing_type = {"name": "", "icon": "", "size_px": 0}}>close</button>
  </form>
  {/if}