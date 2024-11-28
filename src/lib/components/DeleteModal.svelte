<script lang='ts'>

  export let delete_modal : HTMLDialogElement | undefined;
  export let deleteSensor;

</script>

<dialog id="delete_modal" class='modal' bind:this={delete_modal}>
  <div class="modal-box">
    <h3 class="text-lg font-bold">Delete Sensor?</h3>
    <h6>Pressing 'Yes' will delete the currently selected sensor and that data will be unrecoverable</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-primary">No</button>
        <button class="btn btn-error ml-5" onclick={deleteSensor}>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>