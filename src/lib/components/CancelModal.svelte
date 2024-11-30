<script lang='ts'>
  export let cancel_modal;
  export let cancelSensorChanges;
</script>

<dialog id="cancel_modal" class='modal' bind:this={cancel_modal}>
  <div class="modal-box">
    <h3 class="text-lg font-bold">Cancel Changes?</h3>
    <h6>Pressing 'Yes' will delete all changes made without saving</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-primary">No</button>
        <button class="btn btn-error ml-5" onclick={cancelSensorChanges}>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>