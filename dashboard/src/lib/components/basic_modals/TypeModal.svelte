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
  <div class="modal-box bg-gray-800 border border-white/10">
    <h3 class="text-lg font-bold mb-4 text-white">New Sensor Type</h3>
    {#if alert}
      <div class="mb-4 p-3 bg-red-500/20 border border-red-500/30 rounded-lg">
        <h6 class="text-red-300 text-sm">{alert}</h6>
      </div>
    {/if}
    <div class="grid grid-cols-2 gap-4 mb-5">
      <div>
        <h6 class="text-gray-300 text-sm font-medium mb-2">Name: </h6>
        <input type="text" class="w-full px-3 py-2 bg-gray-700/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" bind:value={editing_type.name} placeholder="New Type Name"/>
      </div>
      <div>
        <h6 class="text-gray-300 text-sm font-medium mb-2">Size: </h6>
        <input type="number" min="30" max="80" class="w-full px-3 py-2 bg-gray-700/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50" bind:value={editing_type.size_px} />
      </div>
    </div>
    <div class="flex items-center mb-6">
      <h6 class="mr-4 text-gray-300 text-sm font-medium">Icon: </h6>
      <input type="file" class="flex-1 px-3 py-2 bg-gray-700/50 border border-gray-600/50 rounded-lg text-white file:mr-4 file:py-2 file:px-4 file:rounded-lg file:border-0 file:text-sm file:font-medium file:bg-yellow-500/20 file:text-yellow-300 hover:file:bg-yellow-500/30" accept="image/svg" bind:this={fileInput}/>
    </div>
    
    <div class="flex justify-end space-x-3">
      <form method="dialog">
        <button class="px-4 py-2 bg-gray-600/50 border border-gray-500/50 rounded-lg text-gray-300 hover:bg-gray-500/50 transition-all duration-200" onclick={() => editing_type = {"name": "", "icon": "", "size_px": 30}}>Cancel</button>
      </form>
      <button class="px-4 py-2 bg-yellow-500/20 border border-yellow-500/30 rounded-lg text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200" onclick={confirmType}>Save</button>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button onclick={() => editing_type = {"name": "", "icon": "", "size_px": 0}}>close</button>
  </form>
  {/if}