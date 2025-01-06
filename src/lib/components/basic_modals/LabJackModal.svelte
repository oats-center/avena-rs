<script lang='ts'>
  type FormattedLabJack = {
    "cabinet_id": string;
    "labjack_name": string;
    "serial" : string;
    "sensor_settings": FormattedSensorSettings
  }
  type FormattedSensorSettings = {
    "sampling_rate": number;
    "channels_enabled": boolean[];
    "gains": number;
    "data_formats": string[];
    "measurement_units": string[];
    "publish_raw_data": boolean[];
    "measure_peaks": boolean[];
    "publish_summary_peaks": boolean;
    "labjack_reset": boolean;
  }

  let { labjackEdit, newLabjack, saveEditChanges, saveNewChanges, delete_modal} : 
    {
      labjackEdit: FormattedLabJack | null,
      newLabjack: boolean,
      saveEditChanges: Function,
      saveNewChanges: Function,
      delete_modal: HTMLDialogElement
    } = $props()
</script>


{#if labjackEdit}
  <div class="modal-box edit_modal">
    <form method="dialog">
      <button class="btn btn-sm btn-circle modal_close">✕</button>
    </form>

    <h3>
      {#if newLabjack}
        Add New Labjack
      {:else}
      Edit {labjackEdit.labjack_name}
      {/if}
    </h3>

    <div class="flex items-center my-4">
      <h6 >Serial Number: </h6>
      <input type="text" name="serialNumber" disabled={!newLabjack} class='input modal_input mr-auto' bind:value={labjackEdit.serial} required/>
      <h6>Sampling Rate: </h6>
      <input type="text" class="input modal_input mr-auto" bind:value={labjackEdit.sensor_settings.sampling_rate}/>
      <h6>Gain: </h6>
      <input type="text" class="input modal_input mr-auto" bind:value={labjackEdit.sensor_settings.gains}/>
    </div>

    <table class="table w-full border-collapse">
      <thead class="text-black border-b-2">
        <tr>
          <th>Channel #</th>
          <th>Enabled</th>
          <th>Data Format</th>
          <th>Units</th>
          <th>Publish Raw Data</th>
          <th>Measure Peaks</th>
        </tr>
      </thead>
      <tbody class="text-black">
        {#each Array.from({length: 8}, (_, i) => i + 1) as index}
          <tr class="border-t">
            <td class="text-center">{index}</td>
              <td class="text-center">
                <input type="checkbox" bind:checked={labjackEdit.sensor_settings.channels_enabled[index - 1]} class="checkbox border-black"/>
              </td>
              <td>
                <input type="text" class="input modal_input" bind:value={labjackEdit.sensor_settings.data_formats[index - 1]} disabled={!labjackEdit.sensor_settings.channels_enabled[index - 1]}/>
              </td>
              <td>
                <input type="text" class="input modal_input" bind:value={labjackEdit.sensor_settings.measurement_units[index - 1]} disabled={!labjackEdit.sensor_settings.channels_enabled[index - 1]}/>                      
              </td>
              <td class="text-center">
                <input type="checkbox" bind:checked={labjackEdit.sensor_settings.publish_raw_data[index - 1]} class="checkbox border-black" disabled={!labjackEdit.sensor_settings.channels_enabled[index - 1]}/>                             
              </td>
              <td class="text-center">
                <input type="checkbox" bind:checked={labjackEdit.sensor_settings.measure_peaks[index - 1]} class="checkbox border-black" disabled={!labjackEdit.sensor_settings.channels_enabled[index - 1]}/>                             
              </td>
          </tr>
        {/each}
      </tbody>
    </table>
    <form method="dialog" class="modal-backdrop mt-4">
      <div class="flex justify-center space-x-5">
        <button class="btn btn-outline btn-success w-1/4">Cancel</button>
        <button class="btn btn-outline btn-success w-1/4" onclick={() => newLabjack ? saveNewChanges() : saveEditChanges()}>Save Changes</button>
        <button class="btn btn-outline btn-error  w-1/4" onclick={() => delete_modal?.showModal()}>Delete Labjack</button>
      </div>
    </form>
  </div>
{/if}


<style>
  .edit_modal {
    position: relative;
    max-width: 75vw;
    padding: 1.5rem;
    background-color: #FAF9F6;
    border-radius: 0.5rem;
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.1); 
  }
</style>