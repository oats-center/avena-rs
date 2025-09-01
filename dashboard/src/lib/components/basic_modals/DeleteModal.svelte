<script lang='ts'>

  export let delete_modal : HTMLDialogElement | null | undefined; //modal variable
  export let deleteFunction: Function; //function that will do the deleting
  export let delete_string: string; //type of thing being deleted: "LabJack", "Sensor"
  export let confirmation_string: string | undefined; //name of the item to be deleted "LabJack 1", "Sensor 2"

  let confirmation_input: string; //string typed into the modal box for confirmation

  
  function handleDelete(): void {
    if(confirmation_input == confirmation_string) {
      deleteFunction();
      delete_modal?.close();
      confirmation_input = "";
    }
  }
</script>

{#if delete_modal}
  <div class="modal-box w-full max-w-md">
    <div class="text-center">
      <div class="w-16 h-16 bg-red-500/20 rounded-full flex items-center justify-center mx-auto mb-4">
        <svg class="w-8 h-8 text-red-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.34 16.5c-.77.833.192 2.5 1.732 2.5z"/>
        </svg>
      </div>
      <h3 class="text-xl font-bold text-white mb-2">Delete {delete_string}?</h3>
      <p class="text-gray-400 mb-6">To confirm the deletion, enter the {delete_string} name below:</p>
      <p class="text-lg font-semibold text-white mb-4">{confirmation_string}</p>
      <input 
        type="text" 
        placeholder={confirmation_string} 
        class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-red-500/50 focus:border-red-500/50 transition-all duration-200 mb-6" 
        bind:value={confirmation_input}
      />
      <div class="flex space-x-3">
        <button 
          class="flex-1 px-4 py-2 bg-gray-700 hover:bg-gray-600 text-white font-medium rounded-lg transition-colors duration-200"
          onclick={() => {confirmation_input = ""; delete_modal?.close()}}
        >
          Cancel
        </button>
        {#if confirmation_input === confirmation_string}
          <button 
            class="flex-1 px-4 py-2 bg-gradient-to-r from-red-500 to-red-600 hover:from-red-600 hover:to-red-700 text-white font-medium rounded-lg transition-all duration-200"
            onclick={handleDelete}
          >
            Delete
          </button>
        {:else}
          <button 
            class="flex-1 px-4 py-2 bg-gray-600 text-gray-400 font-medium rounded-lg cursor-not-allowed"
            disabled
          >
            Delete
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}