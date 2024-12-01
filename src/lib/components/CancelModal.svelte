<script lang='ts'>
  export let cancel_modal;
  export let cancelSensorChanges;
</script>

<dialog id="cancel_modal" class='modal' bind:this={cancel_modal}>
  <div class="modal-box bg-primary">
    <h3 class="text-lg font-bold">Cancel Changes?</h3>
    <h6>Pressing 'Yes' will delete all changes made without saving</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-outline btn-success">No</button>
        <button class="btn btn-outline btn-error ml-5" onclick={cancelSensorChanges}>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>