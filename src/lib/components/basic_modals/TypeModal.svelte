<script lang='ts'>
    import { onMount } from "svelte";
    import Page from "../../../routes/+page.svelte";


  interface SensorType {
    "name": string
    "size_px" : number;
    "icon" : string;
  }

  let { editing_type = $bindable(), addSensorType } : { editing_type: SensorType, addSensorType: Function } = $props();
  
  let fileInput = $state<HTMLInputElement>();

  function confirmType(): void {
    if(!editing_type || !fileInput) return;
    
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
    <h3 class="text-lg font-bold">New Sensor Type</h3>
    <div class="grid grid-cols-2">
      <div>
        <h6>Name: </h6>
        <input type="text" class="input modal_input" bind:value={editing_type.name} />
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
        <button class="btn btn-outline btn-success" onclick={() => editing_type = {"name": "", "icon": "", "size_px": 0}}>Cancel</button>
      </form>
      <button class="btn btn-outline btn-error ml-5" onclick={confirmType}>Save</button>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button onclick={() => editing_type = {"name": "", "icon": "", "size_px": 0}}>close</button>
  </form>
  {/if}