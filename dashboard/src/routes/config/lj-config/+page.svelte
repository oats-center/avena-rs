<script lang="ts">
  import { onMount } from "svelte";
  import { KvWatchInclude } from "@nats-io/kv"
  import { goto } from "$app/navigation";

  import { NatsService, connect,  putKeyValue, getKeyValue, getKeys} from "$lib/nats.svelte";
  import Alert from "$lib/components/Alert.svelte";
  import DeleteModal from "$lib/components/basic_modals/DeleteModal.svelte";
  import LabJackModal from "$lib/components/basic_modals/LabJackModal.svelte";
  
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
  let delete_modal = $state<HTMLDialogElement>();

  let labjacks = $state<LabJack[]>([]);
  let loading = $state<boolean>(true);
  let labjackEdit = $state<FormattedLabJack | null>(null);
  let editingIndex = -1;
  let alert = $state<string | null>(null);
  let newLabjack = $state<boolean>(false);
  let cabinetStatus = $state<string>("unknown");
  let errorMessage = $state<string | null>(null);
  
  // Check cabinet status before attempting to connect
  async function checkCabinetStatus(): Promise<string> {
    if (!nats) return "unknown";
    
    try {
      const status = await getKeyValue(nats, "all_cabinets", selectedCabinet!);
      const cabinetData = JSON.parse(status);
      return cabinetData.status || "unknown";
    } catch (error) {
      console.error("Failed to get cabinet status:", error);
      return "offline";
    }
  }

  //initializes new connection with the serverName given, gets all of the labjacks 
  //for the selected cabinet, and watches those vals also
  async function initialize() {
    try {
      if(serverName) nats = await connect(serverName);
      if(nats && selectedCabinet) {
        // Check cabinet status first
        cabinetStatus = await checkCabinetStatus();
        
        if (cabinetStatus.toLowerCase() === 'offline') {
          // Cabinet is offline, don't try to load devices
          loading = false;
          errorMessage = "This cabinet is currently offline and cannot be configured.";
          return;
        }
        
        if (cabinetStatus.toLowerCase() === 'maintenance') {
          // Cabinet is in maintenance mode - load devices but restrict modifications
          try {
            let labjacksList = await getKeys(nats, selectedCabinet, "labjackd.config.*");
            console.log(labjacksList);
            for(let labjack of labjacksList){
              let values = await getLabjack(selectedCabinet, labjack);
              labjacks.push(values);
            }
            loading = false;
            // Don't set up watchers in maintenance mode to avoid conflicts
          } catch (error) {
            console.error("Failed to load devices in maintenance mode:", error);
            loading = false;
            errorMessage = "Failed to load LabJack devices while cabinet is in maintenance mode.";
            return;
          }
          return;
        }
        
        // Cabinet is online, proceed with full functionality
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
        loading = false;
        errorMessage = "Failed to connect to NATS server.";
      }
    } catch (error) {
      console.error("Initialization failed:", error);
      loading = false;
      errorMessage = "Failed to initialize LabJack configuration.";
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
    if (cabinetStatus === 'offline') {
      alert = "Cannot save changes when cabinet is offline.";
      return;
    }
    
    if (cabinetStatus === 'maintenance') {
      alert = "Cannot save changes when cabinet is in maintenance mode.";
      return;
    }
    
    console.log(labjackEdit)
    if(labjackEdit && editingIndex >= 0 && editingIndex < labjacks.length){
      labjacks[editingIndex] = unformatData(labjackEdit)
      if(nats && selectedCabinet) putKeyValue(nats, selectedCabinet, `labjackd.config.${labjacks[editingIndex].serial}`, JSON.stringify(labjacks[editingIndex]));
    }
    editingIndex = -1;
    labjackEdit = null;
    edit_modal?.close()
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

    labjacks.forEach((labjack, index) => {
      watchVal(selectedCabinet!, `labjackd.config.${labjack.serial}`,index);
    })
  }

  async function watchCabinet() {
    if(!nats || !selectedCabinet) throw new Error("NATS is not initialized");

    const kv = await nats.kvm.open(selectedCabinet);
    const watch = await kv.watch({
      "include": KvWatchInclude.UpdatesOnly
    })
    
    for await(const e of watch) {
      let changedIndex = -1;
      const key = e.key;
      const serialNumber = key.split(".").pop();
      changedIndex = labjacks.findIndex(labjack => labjack.serial === serialNumber);

      if(changedIndex == -1) { //new labjack has been added
        let newVal = await getLabjack(selectedCabinet, e.key)
        labjacks.push(newVal);
        alert = "New LabJack Added";
      } else if(changedIndex >= 0 && (e.operation == "DEL" || e.operation == "PURGE")){ //a labjack was deleted
        labjacks.splice(changedIndex, 1);
        alert = `Labjack Deleted`;
      }
    }
  }

  //handles creating a new labjack
  async function createLabjack(event: Event) {
    if (!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    if (!labjackEdit) return;
    
    if (cabinetStatus === 'offline') {
      alert = "Cannot create LabJack devices when cabinet is offline.";
      edit_modal?.close();
      return;
    }
    
    if (cabinetStatus === 'maintenance') {
      alert = "Cannot create LabJack devices when cabinet is in maintenance mode.";
      edit_modal?.close();
      return;
    }

    for(let labjack of labjacks){
      if(labjack.serial == labjackEdit.serial){
        alert = "Serial Number Already Exists";
        edit_modal?.close();
        return;
      }
    }

    let newVals = unformatData(labjackEdit);
    labjackEdit = defaultFormattedLabjack;
    const kv = await nats.kvm.open(selectedCabinet);
    newVals.cabinet_id = selectedCabinet;
    newVals.labjack_name = `Labjack ${newVals.serial}`;
    labjacks.push(newVals);
    kv.create(`labjackd.config.${newVals.serial}`, JSON.stringify(newVals));
    watchVal(selectedCabinet, `labjackd.config.${newVals.serial}`, labjacks.length - 1);
    edit_modal?.close();
  }

  async function deleteLabjack() {
    if (!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    
    if (cabinetStatus === 'offline') {
      alert = "Cannot delete LabJack devices when cabinet is offline.";
      delete_modal?.close();
      return;
    }
    
    if (cabinetStatus === 'maintenance') {
      alert = "Cannot delete LabJack devices when cabinet is in maintenance mode.";
      delete_modal?.close();
      return;
    }
    
    if (editingIndex >= 0 && editingIndex < labjacks.length) {
      const kv = await nats.kvm.open(selectedCabinet);
      await kv.delete(`labjackd.config.${labjacks[editingIndex].serial}`);
    }
    editingIndex = -1;
    labjackEdit = null;
    delete_modal?.close();
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
    if (cabinetStatus === 'offline') {
      alert = "Cannot edit LabJack devices when cabinet is offline.";
      return;
    }
    
    if (cabinetStatus === 'maintenance') {
      alert = "Cannot edit LabJack devices when cabinet is in maintenance mode.";
      return;
    }
    
    newLabjack = false;
    labjackEdit = formatData(labjack);
    editingIndex = index;
    console.log("opening modal")
    console.log(delete_modal);
    console.log(edit_modal)
    edit_modal?.showModal();
  }

  // Get status color for LabJack cards
  function getStatusColor(samplingRate: number) {
    if (samplingRate > 0) return 'border-green-500/30 bg-green-500/10';
    if (samplingRate === 0) return 'border-yellow-500/30 bg-yellow-500/10';
    return 'border-gray-500/30 bg-gray-500/10';
  }

  // Get display name for cabinet
  function getDisplayName(id: string) {
    return id.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase());
  }
</script>

<div class="min-h-screen bg-gradient-to-br from-gray-900 via-gray-800 to-black">
  <!-- Header -->
  <div class="bg-white/5 backdrop-blur-lg border-b border-white/10">
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
      <div class="flex items-center justify-between h-16">
        <!-- Logo and Title -->
        <div class="flex items-center space-x-4">
          <div class="w-8 h-8 bg-gradient-to-br from-yellow-400 to-yellow-600 rounded-lg flex items-center justify-center">
            <svg class="w-5 h-5 text-white" fill="currentColor" viewBox="0 0 20 20">
              <path d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
          </div>
          <h1 class="text-xl font-semibold text-white">Avena-OTR Dashboard</h1>
        </div>
        
        <!-- Connection Status -->
        <div class="flex items-center space-x-3">
          <div class="flex items-center space-x-2">
            <div class="w-2 h-2 bg-green-400 rounded-full animate-pulse"></div>
            <span class="text-sm text-gray-300">Connected to NATS</span>
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- Main Content -->
  <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
    <!-- Page Header -->
    <div class="text-center mb-8">
      <h1 class="text-4xl font-bold text-white mb-4">LabJack Configuration</h1>
      <p class="text-xl text-gray-300 max-w-3xl mx-auto">
        Configure and manage LabJack devices for {selectedCabinet ? getDisplayName(selectedCabinet) : 'the selected Avena box'}
        {#if cabinetStatus === 'maintenance'}
          <span class="block text-yellow-400 text-lg mt-2">🛠️ Maintenance Mode</span>
        {/if}
        {#if cabinetStatus === 'offline'}
          <span class="block text-red-400 text-lg mt-2">🔴 Offline</span>
        {/if}
      </p>
    </div>

    <!-- Maintenance Mode Banner -->
    {#if cabinetStatus === 'maintenance'}
      <div class="mb-6 p-4 bg-yellow-500/20 border border-yellow-500/30 rounded-lg">
        <div class="flex items-center justify-center space-x-3">
          <svg class="w-6 h-6 text-yellow-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.732 16.5c-.77.833.192 2.5 1.732 2.5z"/>
          </svg>
          <span class="text-yellow-300 font-medium">
            Maintenance Mode: This cabinet is currently under maintenance. View-only access is enabled.
          </span>
        </div>
      </div>
    {/if}

    <!-- Navigation and Actions Bar -->
    <div class="flex flex-col sm:flex-row items-center justify-between mb-8 p-4 bg-white/5 backdrop-blur-lg rounded-xl border border-white/10">
      <div class="flex items-center space-x-4 mb-4 sm:mb-0">
        <button 
          onclick={() => goto("/config/cabinet-select")}
          class="flex items-center space-x-2 px-4 py-2 bg-gray-700 hover:bg-gray-600 text-white rounded-lg transition-colors duration-200"
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
          </svg>
          <span>Back to Avena Box Selection</span>
        </button>
        
        <button 
          onclick={() => goto("/config/sensor-map")}
          disabled={cabinetStatus === 'offline' || cabinetStatus === 'maintenance'}
          class="flex items-center space-x-2 px-4 py-2 {cabinetStatus === 'offline' || cabinetStatus === 'maintenance' ? 'bg-gray-500 cursor-not-allowed' : 'bg-blue-600 hover:bg-blue-700'} text-white rounded-lg transition-colors duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 20l-5.447-2.724A1 1 0 013 16.382V5.618a1 1 0 011.447-.894L9 7m0 13l6-3m-6 3V7m6 10l4.553 2.276A1 1 0 0021 18.382V7.618a1 1 0 00-1.447-.894L15 4m0 13V4m0 0L9 7"/>
          </svg>
          <span>Map View</span>
        </button>
      </div>

      <button 
        onclick={() => {newLabjack = true; labjackEdit = defaultFormattedLabjack; edit_modal?.showModal()}}
        disabled={cabinetStatus === 'offline' || cabinetStatus === 'maintenance'}
        class="flex items-center space-x-2 px-6 py-3 {cabinetStatus === 'offline' || cabinetStatus === 'maintenance' ? 'bg-gray-500 cursor-not-allowed' : 'bg-gradient-to-r from-yellow-500 to-yellow-600 hover:from-yellow-600 hover:to-yellow-700'} text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl disabled:opacity-50 disabled:cursor-not-allowed"
      >
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6m0 0v6m0-6h6m-6 0H6"/>
        </svg>
        <span>{cabinetStatus === 'offline' ? 'Cabinet Offline' : cabinetStatus === 'maintenance' ? 'Maintenance Mode' : 'Add New LabJack'}</span>
      </button>
    </div>

    {#if loading}
      <!-- Loading State -->
      <div class="flex items-center justify-center py-20">
        <div class="text-center">
          <div class="inline-flex items-center justify-center w-16 h-16 bg-gradient-to-br from-yellow-400 to-yellow-600 rounded-full mb-6 animate-pulse">
            <svg class="w-8 h-8 text-white animate-spin" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
            </svg>
          </div>
          <p class="text-gray-400 text-lg">Loading LabJack devices...</p>
        </div>
      </div>
    {:else if errorMessage}
      <!-- Error State -->
      <div class="flex items-center justify-center py-20">
        <div class="text-center">
          <div class="inline-flex items-center justify-center w-20 h-20 bg-red-500/20 rounded-full mb-6 border border-red-500/30">
            <svg class="w-10 h-10 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.732 16.5c-.77.833.192 2.5 1.732 2.5z"/>
            </svg>
          </div>
          <h3 class="text-xl font-semibold text-white mb-2">Configuration Error</h3>
          <p class="text-gray-400 mb-6">{errorMessage}</p>
          <button 
            onclick={() => goto("/config/cabinet-select")}
            class="px-6 py-3 bg-gray-600 hover:bg-gray-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl"
          >
            Back to Cabinet Selection
          </button>
        </div>
      </div>
    {:else if cabinetStatus === 'offline'}
      <!-- Offline Cabinet State -->
      <div class="flex items-center justify-center py-20">
        <div class="text-center">
          <div class="inline-flex items-center justify-center w-20 h-20 bg-red-500/20 rounded-full mb-6 border border-red-500/30">
            <svg class="w-10 h-10 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
          </div>
          <h3 class="text-xl font-semibold text-white mb-2">Cabinet Offline</h3>
          <p class="text-gray-400 mb-6">This cabinet is currently offline and cannot be configured. Please select an online cabinet to proceed.</p>
          <button 
            onclick={() => goto("/config/cabinet-select")}
            class="px-6 py-3 bg-gray-600 hover:bg-gray-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl"
          >
            Back to Cabinet Selection
          </button>
        </div>
      </div>
    {:else if cabinetStatus === 'maintenance'}
      <!-- Maintenance Mode State -->
      <div class="flex items-center justify-center py-20">
        <div class="text-center">
          <div class="inline-flex items-center justify-center w-20 h-20 bg-yellow-500/20 rounded-full mb-6 border border-yellow-500/30">
            <svg class="w-10 h-10 text-yellow-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.732 16.5c-.77.833.192 2.5 1.732 2.5z"/>
            </svg>
          </div>
          <h3 class="text-xl font-semibold text-white mb-2">Cabinet in Maintenance Mode</h3>
          <p class="text-gray-400 mb-6">This cabinet is currently in maintenance mode. You can view LabJack devices but modifications are restricted to prevent conflicts with maintenance operations.</p>
          <button 
            onclick={() => goto("/config/cabinet-select")}
            class="px-6 py-3 bg-gray-600 hover:bg-gray-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl"
          >
            Back to Cabinet Selection
          </button>
        </div>
      </div>
    {:else if labjacks && labjacks.length > 0}
      <!-- LabJack Grid -->
      <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
        {#each labjacks as labjack, index}
          {@const statusColor = getStatusColor(labjack.sensor_settings.sampling_rate)}
          {@const isActive = labjack.sensor_settings.sampling_rate > 0}
          
          <div class="group relative bg-white/5 backdrop-blur-lg rounded-2xl p-6 border border-white/10 hover:border-yellow-500/30 transition-all duration-300 hover:transform hover:scale-[1.02] hover:shadow-2xl hover:shadow-yellow-500/10 {statusColor}">
            <!-- Status Badge -->
            <div class="absolute top-4 right-4">
              <div class="flex items-center space-x-2 px-3 py-1.5 rounded-full {isActive ? 'bg-green-500/20 border-green-500/30' : 'bg-yellow-500/20 border-yellow-500/30'} border">
                <div class="w-2 h-2 {isActive ? 'bg-green-400' : 'bg-yellow-400'} rounded-full {isActive ? 'animate-pulse' : ''}"></div>
                <span class="text-xs font-medium {isActive ? 'text-green-400' : 'text-yellow-400'}">
                  {isActive ? 'Active' : 'Inactive'}
                </span>
              </div>
            </div>
            
            <!-- Maintenance Mode Indicator -->
            {#if cabinetStatus === 'maintenance'}
              <div class="absolute top-4 left-4">
                <div class="flex items-center space-x-2 px-3 py-1.5 rounded-full bg-yellow-500/20 border border-yellow-500/30">
                  <svg class="w-3 h-3 text-yellow-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.732 16.5c-.77.833.192 2.5 1.732 2.5z"/>
                  </svg>
                  <span class="text-xs font-medium text-yellow-400">Maintenance</span>
                </div>
              </div>
            {/if}

            <!-- LabJack Icon -->
            <div class="flex items-center justify-center w-16 h-16 bg-gradient-to-br from-blue-500/20 to-purple-500/20 rounded-2xl mb-6 border border-blue-500/30">
              <svg class="w-8 h-8 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"/>
              </svg>
            </div>

            <!-- LabJack Info -->
            <div class="text-center mb-6">
              <h3 class="text-xl font-semibold text-white mb-2">{labjack.labjack_name}</h3>
              <p class="text-gray-400 text-sm">Serial: {labjack.serial}</p>
            </div>

            <!-- Configuration Details -->
            <div class="mb-6 space-y-3">
              <div class="flex items-center justify-between p-3 bg-gray-800/30 rounded-lg border border-gray-700/50">
                <span class="text-sm text-gray-400">Sampling Rate:</span>
                <span class="text-sm font-medium text-white">{labjack.sensor_settings.sampling_rate} Hz</span>
              </div>
              <div class="flex items-center justify-between p-3 bg-gray-800/30 rounded-lg border border-gray-700/50">
                <span class="text-sm text-gray-400">Gain:</span>
                <span class="text-sm font-medium text-white">{labjack.sensor_settings.gains}x</span>
              </div>
              <div class="flex items-center justify-between p-3 bg-gray-800/30 rounded-lg border border-gray-700/50">
                <span class="text-sm text-gray-400">Active Channels:</span>
                <span class="text-sm font-medium text-white">{labjack.sensor_settings.channels_enabled.length}</span>
              </div>
            </div>

            <!-- Action Button -->
            <button
              onclick={() => cabinetStatus === 'maintenance' ? null : openEdit(labjack, index)}
              disabled={cabinetStatus === 'maintenance'}
              class="w-full py-3 px-4 {cabinetStatus === 'maintenance' ? 'bg-gray-500 cursor-not-allowed' : 'bg-gradient-to-r from-blue-500 to-blue-600 hover:from-blue-600 hover:to-blue-700'} text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <div class="flex items-center justify-center space-x-2">
                {#if cabinetStatus === 'maintenance'}
                  <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>
                  </svg>
                  <span>View Only</span>
                {:else}
                  <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/>
                  </svg>
                  <span>Edit Configuration</span>
                {/if}
              </div>
            </button>
          </div>
        {/each}
      </div>

      <!-- Help Text -->
      <div class="mt-12 text-center">
        <div class="inline-flex items-center space-x-2 px-4 py-2 bg-blue-500/20 border border-blue-500/30 rounded-lg">
          <svg class="w-5 h-5 text-blue-400" fill="currentColor" viewBox="0 0 20 20">
            <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z" clip-rule="evenodd"/>
          </svg>
          <span class="text-blue-300 text-sm">
            Click on any LabJack device to edit its configuration, channels, and sensor settings
          </span>
        </div>
      </div>
    {:else}
      <!-- No LabJacks State -->
      <div class="text-center py-20">
        <div class="inline-flex items-center justify-center w-20 h-20 bg-gray-500/20 rounded-full mb-6">
          <svg class="w-10 h-10 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"/>
          </svg>
        </div>
        <h3 class="text-xl font-semibold text-white mb-2">No LabJack Devices Found</h3>
        <p class="text-gray-400 mb-6">No LabJack devices are currently configured for this Avena box.</p>
        <button 
          onclick={() => {newLabjack = true; labjackEdit = defaultFormattedLabjack; edit_modal?.showModal()}}
          disabled={cabinetStatus === 'offline' || cabinetStatus === 'maintenance'}
          class="px-6 py-3 {cabinetStatus === 'offline' || cabinetStatus === 'maintenance' ? 'bg-gray-500 cursor-not-allowed' : 'bg-gradient-to-r from-yellow-500 to-yellow-600 hover:from-yellow-600 hover:to-yellow-700'} text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {cabinetStatus === 'offline' ? 'Cabinet Offline' : cabinetStatus === 'maintenance' ? 'Maintenance Mode' : 'Add Your First LabJack'}
        </button>
      </div>
    {/if}
  </div>
</div>

<!-- Edit/Add LabJack Modal -->
<dialog id="edit_modal" class="modal" bind:this={edit_modal}>
  <LabJackModal
    {labjackEdit}
    {newLabjack}
    saveEditChanges={saveChanges}
    saveNewChanges={createLabjack}
    {delete_modal}
    {edit_modal}
  />
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>

<!-- Delete Confirmation Modal -->
<dialog id="delete_modal" class="modal" bind:this={delete_modal}>
  <DeleteModal 
    {delete_modal} 
    deleteFunction={deleteLabjack} 
    delete_string="labjack" 
    confirmation_string={labjackEdit?.labjack_name}
  />
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>



<Alert bind:alert={alert}/>

<style>
  /* Custom scrollbar */
  ::-webkit-scrollbar {
    width: 8px;
  }
  
  ::-webkit-scrollbar-track {
    background: rgba(255, 255, 255, 0.1);
    border-radius: 4px;
  }
  
  ::-webkit-scrollbar-thumb {
    background: rgba(206, 184, 136, 0.5);
    border-radius: 4px;
  }
  
  ::-webkit-scrollbar-thumb:hover {
    background: rgba(206, 184, 136, 0.7);
  }
  
  /* Smooth transitions */
  * {
    transition-property: color, background-color, border-color, text-decoration-color, fill, stroke, opacity, box-shadow, transform, filter, backdrop-filter;
    transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1);
    transition-duration: 150ms;
  }
</style>

