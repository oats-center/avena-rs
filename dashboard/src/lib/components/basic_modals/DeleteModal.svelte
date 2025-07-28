<script lang='ts'>

  export let delete_modal : HTMLDialogElement | undefined; //modal variable
  export let deleteFunction: Function; //function that will do the deleting
  export let delete_string: string; //type of thing being deleted: "LabJack", "Sensor"
  export let confirmation_string: string | undefined; //name of the item to be deleted "LabJack 1", "Sensor 2"

  let confirmation_input: string; //string typed into the modal box for confirmation

  
  function handleDelete(): void {
    if(confirmation_input == confirmation_string) deleteFunction();
    delete_modal?.close();
    confirmation_input = "";
  }
</script>

<dialog id="delete_modal" class='modal' bind:this={delete_modal}>
  <div class="modal-box bg-primary">
    <h3 class="text-lg font-bold">Delete {delete_string}?</h3>
    <h6>To confirm the deletion, enter the {delete_string} below</h6>
    <input type="text" placeholder={confirmation_string} class="input modal_input" bind:value={confirmation_input}/>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-outline btn-success" onclick={() => confirmation_input=""}>Cancel</button>
        {#if confirmation_input === confirmation_string}
          <button class="btn btn-outline btn-error ml-5" onclick={handleDelete}>Confirm</button>
        {:else}
          <button class="btn btn-disabled ml-5">Confirm</button>
        {/if}
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>