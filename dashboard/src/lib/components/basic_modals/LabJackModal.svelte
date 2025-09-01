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

  let { labjackEdit, newLabjack, saveEditChanges, saveNewChanges, delete_modal, edit_modal, readOnly = false} : 
    {
      labjackEdit: FormattedLabJack | null,
      newLabjack: boolean,
      saveEditChanges: Function,
      saveNewChanges: Function,
      delete_modal: HTMLDialogElement | null | undefined,
      edit_modal: HTMLDialogElement | null | undefined,
      readOnly?: boolean
    } = $props()

  // Function to show delete modal
  function showDeleteModal() {
    if (delete_modal) {
      delete_modal.showModal();
    }
  }
</script>

{#if labjackEdit}
  <div class="modal-box w-full max-w-6xl bg-gray-900 border border-white/20 rounded-2xl shadow-2xl">
    <!-- Header -->
    <div class="flex items-center justify-between mb-6 pb-4 border-b border-white/10">
      <div class="flex items-center space-x-3">
        <div class="w-10 h-10 bg-gradient-to-br from-blue-500 to-purple-600 rounded-xl flex items-center justify-center">
          <svg class="w-6 h-6 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"/>
          </svg>
        </div>
        <div>
          <h3 class="text-xl font-semibold text-white">
            {#if newLabjack}
              Add New LabJack Device
            {:else if readOnly}
              View {labjackEdit.labjack_name}
            {:else}
              Edit {labjackEdit.labjack_name}
            {/if}
          </h3>
          <p class="text-sm text-gray-400">
            {#if readOnly}
              View device settings and channel parameters (read-only)
            {:else}
              Configure device settings and channel parameters
            {/if}
          </p>
        </div>
      </div>
      
      <form method="dialog">
        <button class="btn btn-sm btn-circle bg-gray-700 hover:bg-gray-600 border-0 text-white hover:text-white">
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </form>
    </div>

    <!-- Basic Configuration -->
    <div class="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
      <div class="space-y-2">
        <label for="serialNumber" class="block text-sm font-medium text-gray-300">Serial Number</label>
        <input 
          id="serialNumber"
          type="text" 
          name="serialNumber" 
          disabled={!newLabjack || readOnly} 
          class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed" 
          bind:value={labjackEdit.serial} 
          required
          placeholder="Enter serial number"
        />
      </div>
      
      <div class="space-y-2">
        <label for="samplingRate" class="block text-sm font-medium text-gray-300">Sampling Rate (Hz)</label>
        <input 
          id="samplingRate"
          type="number" 
          class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all duration-200" 
          bind:value={labjackEdit.sensor_settings.sampling_rate}
          placeholder="0"
          disabled={readOnly}
        />
      </div>
      
      <div class="space-y-2">
        <label for="gain" class="block text-sm font-medium text-gray-300">Gain</label>
        <input 
          id="gain"
          type="number" 
          class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all duration-200" 
          bind:value={labjackEdit.sensor_settings.gains}
          placeholder="1"
          disabled={readOnly}
        />
      </div>
    </div>

    <!-- Channel Configuration Table -->
    <div class="mb-8">
      <h4 class="text-lg font-semibold text-white mb-4 flex items-center space-x-2">
        <svg class="w-5 h-5 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5H7a2 2 0 00-2 2v10a2 2 0 002 2h8a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2"/>
        </svg>
        <span>Channel Configuration</span>
      </h4>
      
      <div class="overflow-x-auto">
        <table class="w-full border-collapse">
          <thead>
            <tr class="border-b border-gray-700">
              <th class="text-left py-3 px-4 text-sm font-medium text-gray-300 bg-gray-800/50 rounded-tl-lg">Channel #</th>
              <th class="text-center py-3 px-4 text-sm font-medium text-gray-300 bg-gray-800/50">Enabled</th>
              <th class="text-left py-3 px-4 text-sm font-medium text-gray-300 bg-gray-800/50">Data Format</th>
              <th class="text-left py-3 px-4 text-sm font-medium text-gray-300 bg-gray-800/50">Units</th>
              <th class="text-center py-3 px-4 text-sm font-medium text-gray-300 bg-gray-800/50">Publish Raw Data</th>
              <th class="text-center py-3 px-4 text-sm font-medium text-gray-300 bg-gray-800/50 rounded-tr-lg">Measure Peaks</th>
            </tr>
          </thead>
          <tbody>
            {#each Array.from({length: 8}, (_, i) => i + 1) as index}
              {@const isEnabled = labjackEdit.sensor_settings.channels_enabled[index - 1]}
              <tr class="border-b border-gray-700/50 hover:bg-gray-800/30 transition-colors duration-200">
                <td class="py-3 px-4 text-sm font-medium text-white">
                  <div class="flex items-center space-x-2">
                    <div class="w-6 h-6 bg-gradient-to-br from-blue-500/20 to-purple-500/20 rounded-lg border border-blue-500/30 flex items-center justify-center">
                      <span class="text-xs text-blue-400">{index}</span>
                    </div>
                    <span>Channel {index}</span>
                  </div>
                </td>
                
                <td class="py-3 px-4 text-center">
                  <label class="relative inline-flex items-center cursor-pointer">
                    <input 
                      type="checkbox" 
                      bind:checked={labjackEdit.sensor_settings.channels_enabled[index - 1]} 
                      class="sr-only peer"
                      disabled={readOnly}
                    />
                    <div class="w-11 h-6 bg-gray-600 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-800 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-blue-600"></div>
                  </label>
                </td>
                
                <td class="py-3 px-4">
                  <input 
                    type="text" 
                    class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all duration-200 text-sm {!isEnabled ? 'opacity-50 cursor-not-allowed' : ''}" 
                    bind:value={labjackEdit.sensor_settings.data_formats[index - 1]} 
                    disabled={!isEnabled || readOnly}
                    placeholder="e.g., Voltage, Current"
                  />
                </td>
                
                <td class="py-3 px-4">
                  <input 
                    type="text" 
                    class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all duration-200 text-sm {!isEnabled ? 'opacity-50 cursor-not-allowed' : ''}" 
                    bind:value={labjackEdit.sensor_settings.measurement_units[index - 1]} 
                    disabled={!isEnabled || readOnly}
                    placeholder="e.g., V, A, °C"
                  />
                </td>
                
                <td class="py-3 px-4 text-center">
                  <label class="relative inline-flex items-center cursor-pointer {!isEnabled ? 'opacity-50 cursor-not-allowed' : ''}">
                    <input 
                      type="checkbox" 
                      bind:checked={labjackEdit.sensor_settings.publish_raw_data[index - 1]} 
                      class="sr-only peer"
                      disabled={!isEnabled || readOnly}
                    />
                    <div class="w-9 h-5 bg-gray-600 peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-blue-800 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-green-600"></div>
                  </label>
                </td>
                
                <td class="py-3 px-4 text-center">
                  <label class="relative inline-flex items-center cursor-pointer {!isEnabled ? 'opacity-50 cursor-not-allowed' : ''}">
                    <input 
                      type="checkbox" 
                      bind:checked={labjackEdit.sensor_settings.measure_peaks[index - 1]} 
                      class="sr-only peer"
                      disabled={!isEnabled || readOnly}
                    />
                    <div class="w-9 h-5 bg-gray-600 peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-blue-800 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-green-600"></div>
                  </label>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>

    <!-- Action Buttons -->
    <div class="flex flex-col sm:flex-row justify-center space-y-3 sm:space-y-0 sm:space-x-4 pt-6 border-t border-gray-700">
      <button 
        class="px-6 py-3 bg-gray-700 hover:bg-gray-600 text-white font-medium rounded-lg transition-colors duration-200 flex items-center justify-center space-x-2"
        onclick={() => edit_modal?.close()}
      >
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
        </svg>
        <span>{readOnly ? 'Close' : 'Cancel'}</span>
      </button>
      
      {#if !readOnly}
        <button 
          class="px-6 py-3 bg-gradient-to-r from-blue-500 to-blue-600 hover:from-blue-600 hover:to-blue-700 text-white font-medium rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl flex items-center justify-center space-x-2"
          onclick={() => newLabjack ? saveNewChanges() : saveEditChanges()}
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
          </svg>
          <span>Save Changes</span>
        </button>
        
        {#if !newLabjack}
          <button 
            class="px-6 py-3 bg-gradient-to-r from-red-500 to-red-600 hover:from-red-600 hover:to-red-700 text-white font-medium rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl flex items-center justify-center space-x-2"
            onclick={showDeleteModal}
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M3 7h16"/>
            </svg>
            <span>Delete LabJack</span>
          </button>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  /* Custom scrollbar for the table */
  .overflow-x-auto::-webkit-scrollbar {
    height: 8px;
  }
  
  .overflow-x-auto::-webkit-scrollbar-track {
    background: rgba(255, 255, 255, 0.1);
    border-radius: 4px;
  }
  
  .overflow-x-auto::-webkit-scrollbar-thumb {
    background: rgba(206, 184, 136, 0.5);
    border-radius: 4px;
  }
  
  .overflow-x-auto::-webkit-scrollbar-thumb:hover {
    background: rgba(206, 184, 136, 0.7);
  }
  
  /* Smooth transitions */
  * {
    transition-property: color, background-color, border-color, text-decoration-color, fill, stroke, opacity, box-shadow, transform, filter, backdrop-filter;
    transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1);
    transition-duration: 150ms;
  }
</style>