<script lang='ts'>
  export let save_modal: HTMLDialogElement | undefined;
  export let saveSensorChanges;
</script>


<dialog id="save_modal" class='modal' bind:this={save_modal}>
  <div class="modal-box bg-gray-800 border border-white/10">
    <h3 class="text-lg font-bold text-white mb-4">Save Changes?</h3>
    <p class="text-gray-300 mb-6">Pressing 'Yes' will override the already saved data with the new data.</p>
    <div class="flex justify-end space-x-3">
      <form method="dialog">
        <button class="px-4 py-2 bg-gray-600/50 border border-gray-500/50 rounded-lg text-gray-300 hover:bg-gray-500/50 transition-all duration-200">No</button>
      </form>
      <button class="px-4 py-2 bg-yellow-500/20 border border-yellow-500/30 rounded-lg text-yellow-300 hover:bg-yellow-500/30 transition-all duration-200" onclick={saveSensorChanges}>Yes</button>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>