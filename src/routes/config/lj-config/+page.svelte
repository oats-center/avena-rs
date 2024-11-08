<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { NatsService, connect, getKeys, getKeyValues, putKeyValue, getKeyValue} from "$lib/nats";
    import SensorControls from "$lib/SensorControls.svelte";
  
  type LabJack = {
    cabinet_id: string;
    labjack_name: string;
    sensor_settings: SensorSettings
  }
  type SensorSettings = {
    sampling_rate: number;
    channels_enabled: number[];
    gains: number;
    data_formats: string[];
    measurement_units: string[];
    publish_raw_data: boolean[];
    measure_peaks: boolean[];
    publish_summary_peaks: boolean;
    labjack_reset: boolean;
  }
  let defaultSensorSettings = {
    sampling_rate: 0,
    channels_enabled: [0],
    gains: 0,
    data_formats: [""],
    measurement_units: [""],
    publish_raw_data: [false],
    measure_peaks: [false],
    publish_summary_peaks: false,
    labjack_reset: false,
  }
  let defaultLabjack: LabJack = {
    cabinet_id: "",
    labjack_name: "",
    sensor_settings: defaultSensorSettings
  }

  type FormattedLabJack = {
    cabinet_id: string;
    labjack_name: string;
    sensor_settings: FormattedSensorSettings;
  }
  type FormattedSensorSettings = {
    sampling_rate: number;
    channels_enabled: boolean[];
    gains: number;
    data_formats: string[];
    measurement_units: string[];
    publish_raw_data: boolean[];
    measure_peaks: boolean[];
    publish_summary_peaks: boolean;
    labjack_reset: boolean;
  }
  let defaultFormatted: FormattedSensorSettings = {
      sampling_rate: 0,
      channels_enabled: [false],
      gains: 0,
      data_formats: [""],
      measurement_units: [""],
      publish_raw_data: [false],
      measure_peaks: [false],
      publish_summary_peaks: false,
      labjack_reset: false,
  }

  const natsKey = "config";
  const singleKeys = ["cabinet_id", "labjack_name", "sampling_rate", "gains", "channels_enabled"]
  const arraydKeys = ["data_formats", "measurement_units", "publish_raw_data", "measure_peaks", "publish_summary_peaks", "labjack_reset"];

  let serverName: string | null = null;
  let nats: NatsService | null = null;
  let selectedCabinet: string | null = null;
  let labjacks = $state<LabJack[]>([]);
  let loading = $state<boolean>(true);
  let labjackEdit = $state<FormattedLabJack | null>(null);
  let editingIndex = -1;
  let edit_modal = $state<HTMLDialogElement>();
  
  
  function formatData(labjack: LabJack): FormattedLabJack {
    let formattedLJ = {
      cabinet_id: labjack.cabinet_id,
      labjack_name: labjack.labjack_name,
      sensor_settings: defaultFormatted
    }
    let formattedSettings: FormattedSensorSettings = defaultFormatted;
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

  function unformatData(formattedLJ: FormattedLabJack): LabJack {
    let labjack: LabJack = {
      cabinet_id: formattedLJ.cabinet_id,
      labjack_name: formattedLJ.labjack_name,
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
    console.log(labjack)
    return labjack;
  }

  async function getLabjack(bucket: string, key: string): Promise<LabJack> {
    if(!nats) throw new Error("Nats connection is not initialized");
    let labjack = defaultLabjack;
    let val = await getKeyValue(nats, bucket, key);
    let ljVal = JSON.parse(val) as LabJack;
    labjack = ljVal;
    return labjack;
  }

  async function initialize() {
    if(serverName) nats = await connect(serverName)
    if(nats && selectedCabinet) {
      let labjackBuckets = await getKeyValue(nats, selectedCabinet, "labjacks");
      labjackBuckets = JSON.parse(labjackBuckets)
      for(let labjack of labjackBuckets){
        let values = await getLabjack(`${selectedCabinet}_${labjack}`, natsKey);
        labjacks.push(values);
      }
      loading = false;
    } else {
      console.log('No Nats Connection');
    }
  }

  function putLabjackVals(bucket: string, key: string, newValue: string) {
    if(!nats) throw new Error("Nats connection is not initialized");
    putKeyValue(nats, bucket, key, newValue);
  }

  function openEdit(labjack: LabJack, index: number) {
    labjackEdit = formatData(labjack);
    editingIndex = index;
    console.log(labjacks[editingIndex])
    edit_modal?.showModal();
  }

  function saveChanges() {
    if(labjackEdit){
      labjacks[editingIndex] = unformatData(labjackEdit)
    }
    console.log(labjacks[editingIndex]);
    console.log(labjackEdit)
    if(nats) putKeyValue(nats, `${labjacks[editingIndex].cabinet_id}_${labjacks[editingIndex].labjack_name}`, natsKey, JSON.stringify(labjacks[editingIndex]));
    editingIndex = -1;
    labjackEdit = null;
  }

  onMount(() => {
    serverName = sessionStorage.getItem("serverName");
    selectedCabinet = sessionStorage.getItem("selectedCabinet");
    console.log(`Server Name: ${serverName}, Selected Cabinet: ${selectedCabinet}`);
    initialize();
    console.log(labjacks)
  });

</script>

{#if loading}
  <div class="loading-overlay">
    <span class="loading loading-spinner loading-lg"></span>  
  </div>
{:else if labjacks !== null} 
  <div class="flex flex-col justify-center items-center">
    <h1 class="my-10 text-4xl">Select Labjack to Edit</h1>
    <div class="flex space-x-5">
      {#each labjacks as labjack, index}
      <div class="card bg-stone-200 shadow-xl text-neutral w-[20vw] min-w-60">
        <div class="card-body">
          <div class="flex justify-center">
            <h2 class="card-title">{labjack["labjack_name"]}</h2>
          </div>
          {#each singleKeys as key}
            {#if key !== "cabinet_id" && key !== "labjack_name"}
              <p class="pl-2 mt-2"><strong>{key}:</strong> {labjack.sensor_settings[key as keyof SensorSettings]}</p>
            {/if}
          {/each}
          <div class="mt-3 flex justify-center">
            <button class="btn btn-outline btn-success" onclick={() => openEdit(labjack, index)}>
              Edit Config
            </button>
          </div>
        </div>
      </div>
      {/each}
    </div>
  </div>
{/if}

<dialog id="edit_modal" class="modal" bind:this={edit_modal}>
  <div class="modal-box bg-stone-200 max-w-[75vw] p-6 rounded-lg shadow-lg relative">
    <form method="dialog">
      <button class="btn btn-sm btn-circle absolute right-2 top-2">✕</button>
    </form>
    
    <h3 class="text-lg font-semibold text-black text-center mb-6">
      Edit {labjackEdit?.labjack_name}
    </h3>
    
    {#if labjackEdit}
      <div class="flex items-center my-4">
        <p class="text-black font-medium mr-10" >Sampling Rate:</p>
        <input type="text" class="input modal_input mr-auto" bind:value={labjackEdit.sensor_settings.sampling_rate}/>
        <p class=" text-black font-medium mr-10">Gain:</p>
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
        <button class="btn btn-primary w-1/4 mr-5">Cancel</button>
        <button class="btn btn-primary w-1/4 ml-5" onclick={saveChanges}>Save Changes</button>
      </div>
    </form>
  </div>
</dialog>




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
