<script lang="ts">
  import { onMount } from "svelte";
  import { NatsService, connect,  putKeyValue, getKeyValue, getKeys} from "$lib/nats.svelte";
  import { KvWatchInclude } from "@nats-io/kv"
  import { goto } from "$app/navigation";
  
  type LabJack = {
    "cabinet_id": string;
    "labjack_name": string;
    "serial" : string;
    "sensor_settings": SensorSettings
  }
  type SensorSettings = {
    "sampling_rate": number;
    "channels_enabled": number[];
    "gains": number;
    "data_formats": string[];
    "measurement_units": string[];
    "publish_raw_data": boolean[];
    "measure_peaks": boolean[];
    "publish_summary_peaks": boolean;
    "labjack_reset": boolean;
  }
  type FormattedLabJack = {
    "cabinet_id": string;
    "labjack_name": string;
    "serial": string;
    "sensor_settings": FormattedSensorSettings;
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
  
  //default values for above types
  const defaultSensorSettings = {
    "sampling_rate": 0,
    "channels_enabled": [0],
    "gains": 0,
    "data_formats": [""],
    "measurement_units": [""],
    "publish_raw_data": [false],
    "measure_peaks": [false],
    "publish_summary_peaks": false,
    "labjack_reset": false,
  }
  const defaultFormattedSettings: FormattedSensorSettings = {
    "sampling_rate": 0,
    "channels_enabled": [false],
    "gains": 0,
    "data_formats": [""],
    "measurement_units": [""],
    "publish_raw_data": [false],
    "measure_peaks": [false],
    "publish_summary_peaks": false,
    "labjack_reset": false,
  }
  const defaultFormattedLabjack: FormattedLabJack = {
    "cabinet_id": "",
    "labjack_name": "",
    "serial": "",
    "sensor_settings": defaultFormattedSettings
  }
 
  let serverName: string | null = null;
  let nats: NatsService | null = null;
  let selectedCabinet: string | null = null;
  let edit_modal = $state<HTMLDialogElement>();
  let new_modal = $state<HTMLDialogElement>();
  let verify_modal = $state<HTMLDialogElement>();

  let labjacks = $state<LabJack[]>([]);
  let loading = $state<boolean>(true);
  let labjackEdit = $state<FormattedLabJack | null>(null);
  let editingIndex = -1;
  let alert = $state<string | null>(null);
  let newLabjack = $state<FormattedLabJack>(defaultFormattedLabjack)
  
  //initializes new connection with the serverName given, gets all of the labjacks 
  //for the selected cabinet, and watches those vals also
  async function initialize() {
    if(serverName) nats = await connect(serverName)
    if(nats && selectedCabinet) {
      let labjacksList = await getKeys(nats, selectedCabinet, "labjackd.config.*");
      console.log(labjacksList);
      for(let labjack of labjacksList){
        let values = await getLabjack(selectedCabinet, labjack);
        labjacks.push(values);
      }
      loading = false;
      watchLabJacks();
      watchCabinet();
    } else {
      console.log('No Nats Connection');
    }
  }

  //gets & formats the data for one labjack
  async function getLabjack(bucket: string, key: string): Promise<LabJack> {
    if(!nats) throw new Error("Nats connection is not initialized");
    let val = await getKeyValue(nats, bucket, key);
    let ljVal = JSON.parse(val) as LabJack;
    return ljVal;
  }

  //saves changes made to a labjack
  function saveChanges() {
    if(labjackEdit){
      labjacks[editingIndex] = unformatData(labjackEdit)
    }
    if(nats && selectedCabinet) putKeyValue(nats, selectedCabinet, `labjackd.config.${labjacks[editingIndex].serial}`, JSON.stringify(labjacks[editingIndex]));
    editingIndex = -1;
    labjackEdit = null;
  }

  //watches the values of one key
  async function watchVal(bucket: string, key: string, index: number): Promise<void> {
    if(!nats) throw new Error("NATS is not initialized");
    const kv = await nats.kvm.open(bucket);
    const watch = await kv.watch({
      "include": KvWatchInclude.UpdatesOnly,
      "key": key,
    })
    for await (const e of watch) {
      if(e.operation == "PUT"){
        if(e.value) labjacks[index] = JSON.parse(e.string());
        alert = `Changes were made to ${labjacks[index].labjack_name}`;
      }
    }
  }

  //watches the values of all labjacks
  function watchLabJacks() {
    if (!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    for(let i = 0; i < labjacks.length; i++){
      watchVal(selectedCabinet, `labjackd.config.${labjacks[i].serial}`,i);
    }
  }

  async function watchCabinet() {
    if(!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    const kv = await nats.kvm.open(selectedCabinet);
    const watch = await kv.watch({
      "include": KvWatchInclude.UpdatesOnly
    })
    for await(const e of watch) {
      let exists = -1;
      const key = e.key;
      const serialNumber = key.split(".").pop();
      for(let i = 0; i < labjacks.length; i++){
        if(labjacks[i].serial === serialNumber){
          exists = i;
          break;
        }
      }
      if(exists == -1) {
        let newVal = await getLabjack(selectedCabinet, e.key)
        labjacks.push(newVal);
        alert = "New LabJack Added";
      } else if(exists && (e.operation == "DEL" || e.operation == "PURGE")){
        labjacks.splice(exists, 1);
        alert = `Labjack Deleted`;
      }
    }
  }

  //handles creating a new labjack
  async function createLabjack(event: Event) {
    event.preventDefault();
    if (!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    
    for(let labjack of labjacks){
      if(labjack.serial == newLabjack.serial){
        alert = "Serial Number Already Exists";
        new_modal?.close();
        return;
      }
    }
    let newVals = unformatData(newLabjack);
    newLabjack = defaultFormattedLabjack;
    const kv = await nats.kvm.open(selectedCabinet);
    newVals.cabinet_id = selectedCabinet;
    newVals.labjack_name = `Labjack ${newVals.serial}`;
    labjacks.push(newVals);
    kv.create(`labjackd.config.${newVals.serial}`, JSON.stringify(newVals));
    watchVal(selectedCabinet, `labjackd.config.${newVals.serial}`, labjacks.length - 1);
    new_modal?.close();
  }

  async function deleteLabjack() {
    if (!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    const kv = await nats.kvm.open(selectedCabinet);
    await kv.delete(`labjackd.config.${labjacks[editingIndex].serial}`);
    editingIndex = -1;
    edit_modal?.close();
  }

  //gets the selected cabinet and server name from session storage
  onMount(() => {
    serverName = sessionStorage.getItem("serverName");
    if (!serverName) goto("/")
    selectedCabinet = sessionStorage.getItem("selectedCabinet");
    initialize();
  });

  //formats the data to fit in the table properly
  function formatData(labjack: LabJack): FormattedLabJack {
    let formattedLJ = {
      cabinet_id: labjack.cabinet_id,
      labjack_name: labjack.labjack_name,
      serial: labjack.serial,
      sensor_settings: defaultFormattedSettings
    }
    let formattedSettings: FormattedSensorSettings = defaultFormattedSettings;
    const settings = labjack.sensor_settings;

    
    formattedSettings.sampling_rate = settings.sampling_rate;
    formattedSettings.gains = settings.gains;
    formattedSettings.publish_summary_peaks = settings.publish_summary_peaks;
    formattedSettings.labjack_reset = false;
    for(let i = 0; i < 8; i++) {
      const index = settings.channels_enabled.indexOf(i + 1)
      if(index !== -1){
        formattedSettings.channels_enabled[i] = true;
        formattedSettings.data_formats[i] = settings.data_formats[index];
        formattedSettings.measurement_units[i] = settings.measurement_units[index];
        formattedSettings.publish_raw_data[i] = settings.publish_raw_data[index];
        formattedSettings.measure_peaks[i] = settings.measure_peaks[index];
      } else {
        formattedSettings.channels_enabled[i] = false;
        formattedSettings.data_formats[i] = "";
        formattedSettings.measurement_units[i] = "";
        formattedSettings.publish_raw_data[i] = false;
        formattedSettings.measure_peaks[i] = false;
      }
    }

    return formattedLJ;
  }

  //unformats data from the table to its proper labjack type
  function unformatData(formattedLJ: FormattedLabJack): LabJack {
    let labjack: LabJack = {
      cabinet_id: formattedLJ.cabinet_id,
      labjack_name: formattedLJ.labjack_name,
      serial: formattedLJ.serial,
      sensor_settings: defaultSensorSettings
    }
    const settings = formattedLJ.sensor_settings;
    let activeChannel = -1;

    labjack.sensor_settings.sampling_rate = settings.sampling_rate;
    labjack.sensor_settings.gains = settings.gains;
    labjack.sensor_settings.publish_summary_peaks = settings.publish_summary_peaks;
    labjack.sensor_settings.labjack_reset = false;
    for(let i = 0; i < 8; i++) {
      if(settings.channels_enabled[i]){
        activeChannel++;
        labjack.sensor_settings.channels_enabled[activeChannel] = i + 1;
        labjack.sensor_settings.data_formats[activeChannel] = settings.data_formats[i];
        labjack.sensor_settings.measurement_units[activeChannel] = settings.measurement_units[i];
        labjack.sensor_settings.publish_raw_data[activeChannel] = settings.publish_raw_data[i];
        labjack.sensor_settings.measure_peaks[activeChannel] = settings.measure_peaks[i];
      }
    }
    return labjack;
  }

  //opens the edit modal
  function openEdit(labjack: LabJack, index: number) {
    labjackEdit = formatData(labjack);
    editingIndex = index;
    edit_modal?.showModal();
  }
</script>

{#if loading}
  <div class="loading-overlay">
    <span class="loading loading-spinner loading-lg"></span>  
  </div>
{:else}
  <div class="flex flex-col items-center w-full px-4">
    <h1 class="my-8 text-3xl sm:text-4xl text-center">Labjack Configuration</h1>
    <div class="flex mb-8">
      <div class="flex w-64 mx-10 justify-center">
        <button class="btn btn-primary" onclick={() => goto("/config/cabinet-select")}>{"<--"}Back to Cabinet Select</button>
      </div>
      <div class="flex w-64 mx-10 justify-center">
        <button class="btn btn-primary" onclick={() => new_modal?.showModal()}>New LabJack</button>
      </div>
      <div class="flex w-64 mx-10 justify-center">
        <button class="btn btn-primary" onclick={() => goto("sensor-map")}>Map View</button>
      </div>
    </div>
    {#if labjacks !== null} 
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-5 w-full max-w-7xl">
        {#each labjacks as labjack, index}
          <div class="card bg-primary shadow-lg text-neutral p-4">
            <div class="card-body space-y-4">
              <h2 class="card-title text-center">{labjack["labjack_name"]}</h2>
              <p><strong>Sampling Rate:</strong> {labjack.sensor_settings["sampling_rate"]}</p>
              <p><strong>Gain:</strong> {labjack.sensor_settings["gains"]}</p>
              <p><strong>Serial:</strong> {labjack.serial}</p>
              <div class="flex justify-center">
                <button class="btn btn-outline btn-success" onclick={() => openEdit(labjack, index)}>
                  Edit Config
                </button>
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<dialog id="edit_modal" class="modal" bind:this={edit_modal}>
  <div class="modal-box bg-primary max-w-[75vw] p-6 rounded-lg shadow-lg relative">
    <form method="dialog">
      <button class="btn btn-sm btn-circle absolute right-2 top-2">✕</button>
    </form>
    
    <h3>
      Edit {labjackEdit?.labjack_name}
    </h3>
    
    {#if labjackEdit}
      <div class="flex items-center my-4">
        <h6>Sampling Rate:</h6>
        <input type="text" class="input modal_input mr-auto" bind:value={labjackEdit.sensor_settings.sampling_rate}/>
        <h6>Gain:</h6>
        <input type="text" class="input modal_input mr-auto" bind:value={labjackEdit.sensor_settings.gains}/>
      </div>

      <div class="overflow-x-auto my-6">
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
      </div>
    {/if}
    <form method="dialog" class="modal-backdrop mt-4">
      <div class="flex justify-center">
        <button class="btn btn-outline btn-success w-1/4 mr-5">Cancel</button>
        <button class="btn btn-outline btn-success w-1/4 ml-5" onclick={saveChanges}>Save Changes</button>
        <button class="btn btn-outline btn-error  w-1/4 ml-5" onclick={() => verify_modal?.showModal()}>Delete Labjack</button>
      </div>
    </form>
  </div>
</dialog>

<dialog id="new_modal" class="modal" bind:this={new_modal}>
  <div class="modal-box bg-primary max-w-[75vw] p-6 rounded-lg shadow-lg relative">
    <form method="dialog">
      <button class="btn btn-sm btn-circle absolute right-2 top-2">✕</button>
    </form>
    
    <h3>
      Add New LabJack
    </h3>
    <form onsubmit={createLabjack}>
      <div class="flex items-center my-4">
        <h6>Serial Number:</h6>
        <input type="text" name="serialNumber" class='input modal_input mr-auto' bind:value={newLabjack.serial} required/>
        <h6>Sampling Rate:</h6>
        <input type="text" class="input modal_input mr-auto" bind:value={newLabjack.sensor_settings.sampling_rate}/>
        <h6>Gain: </h6>
        <input type="text" class="input modal_input mr-auto" bind:value={newLabjack.sensor_settings.gains}/>
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
                  <input type="checkbox" bind:checked={newLabjack.sensor_settings.channels_enabled[index - 1]} class="checkbox border-black"/>
                </td>
                <td>
                  <input type="text" class="input modal_input" bind:value={newLabjack.sensor_settings.data_formats[index - 1]} disabled={!newLabjack.sensor_settings.channels_enabled[index - 1]}/>
                </td>
                <td>
                  <input type="text" class="input modal_input" bind:value={newLabjack.sensor_settings.measurement_units[index - 1]} disabled={!newLabjack.sensor_settings.channels_enabled[index - 1]}/>                      
                </td>
                <td class="text-center">
                  <input type="checkbox" bind:checked={newLabjack.sensor_settings.publish_raw_data[index - 1]} class="checkbox border-black" disabled={!newLabjack.sensor_settings.channels_enabled[index - 1]}/>                             
                </td>
                <td class="text-center">
                  <input type="checkbox" bind:checked={newLabjack.sensor_settings.measure_peaks[index - 1]} class="checkbox border-black" disabled={!newLabjack.sensor_settings.channels_enabled[index - 1]}/>                             
                </td>
            </tr>
          {/each}
        </tbody>
      </table>
      <button type="submit" class="btn btn-outline btn-success">Add LabJack</button>
    </form>
  </div>
</dialog>

<dialog id="verify_modal" class='modal' bind:this={verify_modal}>
  <div class="modal-box bg-primary">
    <h3>Save Changes?</h3>
    <h6>Pressing 'Yes' will delete the current selected LabJack.</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-outline btn-success" onclick={() => edit_modal?.showModal()}>No</button>
        <button class="btn btn-outline btn-error ml-5" onclick={ () => deleteLabjack() }>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>

{#if alert}
  <div class="toast toast-top toast-center">
    <div role="alert" class="alert">
      <svg
        xmlns="http://www.w3.org/2000/svg"
        fill="none"
        viewBox="0 0 24 24"
        class="stroke-info h-6 w-6 shrink-0">
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          stroke-width="2"
          d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
      </svg>
      <span>{alert}</span>
      <div>
        <button class="btn btn-sm btn-primary" onclick={() => {alert=null;}}>Close</button>
      </div>
    </div>
  </div>
{/if}


<style>
  .loading-overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    background-color: rgba(0, 0, 0, 0.5);
    display: flex;
    justify-content: center;
    align-items: center;
  }
  
  .modal_input {
    width: 1/2;
    max-width: "xs";
    background-color: rgb(231 229 228);
    border-color: black;
    color: black; 
  }
</style>
