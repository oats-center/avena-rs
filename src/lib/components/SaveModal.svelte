<script lang='ts'>
  export let save_modal: HTMLDialogElement | undefined;
  export let saveSensorChanges;
</script>


<dialog id="save_modal" class='modal' bind:this={save_modal}>
  <div class="modal-box">
    <h3 class="text-lg font-bold">Save Changes?</h3>
    <h6>Pressing 'Yes' will override the already saved data with the new data.</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-primary">No</button>
        <button class="btn btn-success ml-5" onclick={saveSensorChanges}>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>