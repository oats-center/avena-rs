<script lang='ts'>
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  import { connect, getKeyValue, NatsService, putKeyValue } from '$lib/nats.svelte';
  import SensorControls from '$lib/components/SensorControls.svelte';
  import SaveModal from "$lib/components/SaveModal.svelte";
  import CancelModal from "$lib/components/CancelModal.svelte";
  import DeleteModal from "$lib/components/DeleteModal.svelte";
  import Alert from "$lib/components/Alert.svelte";
  
  interface Sensor {
    "cabinet_id" : string;
    "labjack_serial" : string;
    "connected_channel": string; 
    "sensor_name" : string; 
    "sensor_type" : string; 
    "x_pos" : number; 
    "y_pos" : number; 
    "color" : string; 
  }

  interface MapConfig {
    "backgroundImage": string;
    [key: `labjackd.${string}.ch${string}`]: Sensor;
  }

  //varianles that shouldn't change within this file
  let serverName: string | null;
  let selectedCabinet: string | null;
  let nats: NatsService | null;
  const sensorColors = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'grey', 'black'];
  const sensorGroups = ['Temperature', 'Pressure']
  let cancel_modal = $state<HTMLDialogElement>();
  let delete_modal = $state<HTMLDialogElement>();
  let save_modal = $state<HTMLDialogElement>();

  //variables that will change within the file  
  let sensors = $state<Sensor[]>([]);
  let loading = $state<boolean>(true);
  let mapconfig: MapConfig;
  let editingSensor = $state<Sensor | null>(null);
  let backgroundImage = $state<string | null>(null);
  let editingIndex= $state<number>(-1);
  let queuedIndex = -1;
  let sensorSize= $state<number>(40);
  let alert = $state<string | null>(null);
  
  //gets values from nats and parses
  async function initialize(): Promise<void> {
    if(serverName) nats = await connect(serverName);
    if(nats && selectedCabinet) {
      /* putKeyValue(nats, "road1_cabinet1", "mapconfig", JSON.stringify({
         "backgroundImage" : "",
         "labjackd.1.ch1" : {
          "cabinet_id" : "road1_cabinet1",
          "color" :  "red", 
          "connected_channel" : "1",
          "labjack_serial" : "1",
          "sensor_name" : "Sensor 1",
          "sensor_type" : "Temperature",
          "x_pos" : 0.5,
          "y_pos" : 0.5
        },
        "labjackd.1.ch2" : {
          "cabinet_id" : "road1_cabinet1",
          "color" :  "red", 
          "connected_channel" : "2",
          "labjack_serial" : "1",
          "sensor_name" : "Sensor 2",
          "sensor_type" : "Temperature",
          "x_pos" : 0.75,
          "y_pos" : 0.75
        }
      })) */
      
      //gets the values from NATS   
      let tempMapConfig = await getKeyValue(nats, selectedCabinet, "mapconfig");
      mapconfig = JSON.parse(tempMapConfig) as MapConfig
      sensors = Object.entries(mapconfig)
        .filter(([key]) => key !== "backgroundImage")
        .map(([, value]) => value as Sensor);
      backgroundImage = mapconfig.backgroundImage
      loading = false;
    }
  }

  //saves sensor changes to nats
  function saveSensorChanges(): void {
    if(!nats || !selectedCabinet || !editingSensor || editingIndex === -1) throw new Error("Something went wrong with saving changes");
    
    sensors[editingIndex] = editingSensor;
    mapconfig[`labjackd.${editingSensor.labjack_serial}.ch${editingSensor.connected_channel}`] = editingSensor;
    console.log(mapconfig);
    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
    
    editingSensor = null;
    editingIndex = -1;
    save_modal?.close();
  }
  
  //cancels changes depending on the state of the editing sensor
  function handleSensorChanges(sensor?: Sensor, index?: number): void {
    // option: used cancel button
    if((index === undefined || sensor === undefined) && queuedIndex === -1) { 
      editingSensor = null;
      editingIndex = -1;
      console.log("Used Cancel Button");
    
      // option: initial selection
    } else if(index !== undefined && editingSensor === null && sensor !== null && index !== -1) {
      editingSensor = JSON.parse(JSON.stringify(sensor));
      editingIndex = index;
      console.log("Initial Selection");
    
      //option: selected new sensor with no change to currently editing sensor
    } else if (index !== undefined  && editingIndex !== index && JSON.stringify(editingSensor) === JSON.stringify(sensors[editingIndex])){
      editingSensor = JSON.parse(JSON.stringify(sensor));
      editingIndex = index;
      console.log("Selected new sensor with no change to currently editing sensor");
    
      //option: selected new sensor with changes to currently editing sensor
    } else if (index !== undefined  && editingSensor !== null && editingIndex !== index) {
      cancel_modal?.showModal();
      queuedIndex = index;
      console.log(queuedIndex);
      console.log("New sensor clicked on")
    //option: cancel modal confirm from new sensor selection 
    } else if (queuedIndex !== -1) {
      editingSensor = JSON.parse(JSON.stringify(sensors[queuedIndex]));
      editingIndex = queuedIndex;
      queuedIndex = -1;
    }
  }

  //handles adding a sensor, doesn't get updated in nats until updated
  function addSensor(): void {
    if(!selectedCabinet || !nats) throw new Error("Page not properly loaded");
    let newSensor: Sensor = {
      "cabinet_id" : selectedCabinet,
      "labjack_serial" : "0",
      "connected_channel": "0",
      "sensor_name" : "New Sensor", 
      "sensor_type" : "Temperature",
      "x_pos" : 0,
      "y_pos" : 0, 
      "color" : "red" 
    }
    sensors.push(newSensor);
    editingSensor = JSON.parse(JSON.stringify(newSensor));
    editingIndex = sensors.length - 1;
  }

  //deletes a sensor from the sensors array and nats
  function deleteSensor(): void {
    if(!nats || !selectedCabinet || !editingSensor || editingIndex === -1) throw new Error("Something went wrong with saving changes");
    sensors.splice(editingIndex, 1);
    delete mapconfig[`labjackd.${editingSensor.labjack_serial}.ch${editingSensor.connected_channel}`];
    editingIndex = -1;
    editingSensor = null;
    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
  }

  //saves changes to the background to nats 
  function saveBackgroundChanges(background: string): void {
    if (!nats || !selectedCabinet) throw new Error("NATS is not initialized");
    mapconfig.backgroundImage = background;
    backgroundImage = background;
    putKeyValue(nats, selectedCabinet, "mapconfig", JSON.stringify(mapconfig));
  }

  //sets up the page
  onMount(() => {
    serverName = sessionStorage.getItem("serverName");
    selectedCabinet = sessionStorage.getItem("selectedCabinet");
    initialize();
  })

  //formats backgroundImage and sensor to how it should go into nats
  function formatToNats(): MapConfig {
    if(!backgroundImage || !sensors) throw new Error("Something went wrong with formatting");
    let formattedData: MapConfig = {"backgroundImage" : backgroundImage};
    sensors.forEach((sensor) => {
      formattedData[`labjackd.${sensor.labjack_serial}.ch${sensor.connected_channel}`] = sensor
    })
    return formattedData;
  }
</script>

{#if loading} 
<div class="loading-overlay">
  <span class="loading loading-spinner loading-lg"></span>  
</div>
{:else}
<div class="flex flex-col items-center w-full h-screen mb-10">
  <h1>Map Configuration</h1>
  <div class="flex mb-8">
      <div class="flex mx-10 justify-center">
        <button class="btn btn-primary" onclick={() => goto("/config/cabinet-select")}>{"<--"}Back to Cabinet Select</button>
      </div>
      <div class="flex mx-10 justify-center">
        <button class="btn btn-primary" onclick={() => addSensor()}>New Sensor</button>
      </div>
      <div class="flex mx-10 justify-center">
        <button class="btn btn-primary" onclick={() => goto("lj-config")}>Card View</button>
      </div>
    </div>
  <div class='flex justify-center items-center'>
    <div class="mx-40 relative h-fit">
      <img 
        alt="Background for Map" 
        src={backgroundImage}
        style="z-index: -1; height: 75vh; z-index: -1; position: relative;"
      />
      {#each sensors as sensor, index}
        <svg
          width={`${sensorSize}px`}
          height={`${sensorSize}px`}
          version="1.1"
          viewBox="0 0 100 100"
          xmlns="http://www.w3.org/2000/svg"
          style={`
                  position: absolute; 
                  top: calc(${(index === editingIndex && editingSensor ? editingSensor.x_pos : sensor.x_pos)}% - ${sensorSize / 2}px);
                  left: calc(${(index === editingIndex && editingSensor ? editingSensor.y_pos : sensor.y_pos)}% - ${sensorSize / 2}px);
                  border-radius: 8px; 
                  outline: ${index === editingIndex ? "2px solid black" : "none"}; 
                `}
          onmousedown={() => {handleSensorChanges(sensor, index)}}
          onkeydown={() => {handleSensorChanges(sensor, index)}}
          role="button"
          tabindex=0
        >
          {#if (sensor.sensor_type === "Temperature" && editingIndex !== index) || (editingIndex === index && editingSensor?.sensor_type === "Temperature")}
            <path
              d="m50 22.801c16.695 0 30.227 13.535 30.227 30.227 0 9.8828-4.7422 18.656-12.07 24.172l-5.3086-7.2812 2.1719-1.8672 3.6367 4.9883c5.3594-5 8.7188-12.117 8.7188-20.012 0-15.094-12.281-27.371-27.375-27.371s-27.375 12.277-27.375 27.371c0 7.8789 3.3516 14.988 8.6953 19.984l3.6289-4.9961 2.1641 1.875-5.2891 7.293c-7.3203-5.5195-12.055-14.281-12.055-24.156 0-16.691 13.535-30.227 30.23-30.227zm-7.8555 11.324 8.6719 14.902c1.8633 0.37891 3.2695 2.0273 3.2695 4 0 2.2578-1.8281 4.0859-4.0859 4.0859-2.2539 0-4.0859-1.8281-4.0859-4.0859 0-0.65625 0.17188-1.2695 0.44922-1.8203l-6.9297-15.703z"
              fill-rule="evenodd"
              fill={(index === editingIndex && editingSensor ? editingSensor.color : sensor.color)}
            />
          {:else}
            <path 
              d="m50.195 45.66c2.3984 0 4.3398 1.9453 4.3398 4.3438 0 2.3945-1.9414 4.3398-4.3398 4.3398-1.832 0-3.3984-1.1367-4.0352-2.7383h-22.379v-3.1992h22.379c0.63672-1.6094 2.2031-2.7461 4.0352-2.7461zm-0.19531 52.809c6.543 0 12.891-1.2812 18.867-3.8086 5.7695-2.4414 10.953-5.9336 15.402-10.387 4.4492-4.4492 7.9453-9.6328 10.387-15.406 2.5273-5.9766 3.8086-12.324 3.8086-18.867s-1.2812-12.891-3.8086-18.867c-2.4414-5.7734-5.9375-10.953-10.387-15.406-4.4492-4.4492-9.6328-7.9453-15.402-10.387-5.9766-2.5234-12.328-3.8086-18.867-3.8086-6.543 0-12.891 1.2812-18.867 3.8125-5.7734 2.4414-10.953 5.9336-15.406 10.387-4.4492 4.4492-7.9453 9.6328-10.387 15.406-2.5234 5.9727-3.8086 12.32-3.8086 18.863s1.2812 12.891 3.8086 18.867c2.4414 5.7734 5.9375 10.953 10.387 15.406 4.4492 4.4492 9.6328 7.9453 15.406 10.387 5.9766 2.5234 12.324 3.8086 18.867 3.8086zm0-93.574c24.871 0 45.105 20.234 45.105 45.105s-20.234 45.105-45.105 45.105-45.105-20.234-45.105-45.105 20.234-45.105 45.105-45.105zm-5.3086 5.8125-1.7852 0.28125 1.7109 10.793 1.7852-0.28125zm-6.0859 1.3125-1.7188 0.55859 3.375 10.395 1.7188-0.55859zm-5.7969 2.25-1.6094 0.82031 4.9609 9.7383 1.6094-0.82031zm-5.3789 3.1289-1.4609 1.0625 6.4258 8.8398 1.4609-1.0625zm-4.8203 3.9336-1.2773 1.2773 7.7266 7.7266 1.2773-1.2773zm-4.1484 4.6367-1.0625 1.4609 8.8398 6.4219 1.0625-1.4609zm-3.3711 5.2305-0.82031 1.6094 9.7344 4.9609 0.82031-1.6094zm-2.5117 5.6953-0.55859 1.7148 10.395 3.3789 0.55859-1.7188zm-1.5898 6.0156-0.28125 1.7812 10.793 1.7109 0.28125-1.7852zm-3.2695 5.4922v3.1992h13.57v-3.1992zm70.949 0v3.1992h13.57v-3.1992zm-30.27-27.07h3.1992v-13.57h-3.1992zm40.895 23.359-0.28125-1.7812-10.797 1.707 0.28516 1.7812zm-1.3125-6.082-0.55859-1.7188-10.395 3.375 0.55859 1.7188zm-2.25-5.8008-0.82031-1.6094-9.7344 4.9609 0.82031 1.6094zm-3.1289-5.3789-1.0625-1.4609-8.8398 6.4219 1.0625 1.4609zm-3.9336-4.8203-1.2773-1.2773-7.7266 7.7266 1.2773 1.2773zm-4.6367-4.1484-1.4609-1.0625-6.4258 8.8398 1.4609 1.0625zm-5.2305-3.3711-1.6094-0.82031-4.9609 9.7383 1.6094 0.82031zm-5.6953-2.5117-1.7188-0.55859-3.3789 10.395 1.7188 0.55859zm-6.0117-1.5898-1.7812-0.28125-1.7148 10.793 1.7852 0.28125zm-7.0469 70.5c1.8359 0 3.3242-1.4883 3.3242-3.3242 0-1.1016-0.53906-2.0742-1.3633-2.6797v-2.7891c0.007813 0 0.015626 0.003907 0.023438 0.003907h2.2773c0.19922 0 0.36328-0.15625 0.36328-0.34766v-1.0039c0-0.19141-0.16406-0.34766-0.36328-0.34766h-2.2773c-0.007812 0-0.015625 0.003906-0.023438 0.003906v-2.4102c0.007813 0 0.015626 0.003906 0.023438 0.003906h2.2773c0.19922 0 0.36328-0.15625 0.36328-0.34766v-1.0039c0-0.19141-0.16406-0.34766-0.36328-0.34766h-2.2773c-0.007812 0-0.015625 0-0.023438 0.003906v-2.4102c0.007813 0 0.015626 0.003906 0.023438 0.003906h2.2773c0.19922 0 0.36328-0.15625 0.36328-0.35156v-1c0-0.19141-0.16406-0.34766-0.36328-0.34766h-2.2773c-0.007812 0-0.015625 0.003906-0.023438 0.003906v-0.96094c0-1.082-0.87891-1.9609-1.9648-1.9609-1.082 0-1.9609 0.87891-1.9609 1.9609v13.652c-0.82422 0.60547-1.3633 1.5781-1.3633 2.6797 0.003906 1.8242 1.4922 3.3164 3.3281 3.3164zm3.875-5.1406c0.23828 0.52734 0.37109 1.1133 0.37109 1.7305 0 0.050781-0.003907 0.097656-0.003907 0.14453 0.57422-0.12891 1.2695-0.007812 2.0781 0.74609 1.707 1.5859 3.8398 1.6055 5.8555 0.058594 1.2852-0.98828 1.9609-0.96094 1.9648-0.96094-0.046875-0.003906-0.066406-0.011718-0.066406-0.011718l0.39453-1.875c-0.32031-0.066407-1.5195-0.17188-3.4648 1.3281-1.2852 0.98828-2.3594 1.0078-3.3789 0.058594-1.1719-1.0898-2.4727-1.4961-3.75-1.2188zm-18.109 1.7148c0.007812 0 0.68359-0.027344 1.9688 0.96094 2.0117 1.5508 4.1445 1.5273 5.8516-0.058594 0.875-0.8125 1.6172-0.89453 2.2148-0.71094-0.003906-0.058594-0.003906-0.11719-0.003906-0.17969 0-0.60547 0.12891-1.1836 0.35938-1.7031-1.3164-0.32812-2.6641 0.066406-3.8711 1.1875-1.0234 0.94922-2.0977 0.92969-3.3789-0.058594-1.9492-1.5-3.1445-1.3945-3.4688-1.3281l0.39844 1.875c0 0.007813-0.023437 0.015625-0.070312 0.015625zm25.238 4.3203c-1.2852 0.98828-2.3594 1.0078-3.3789 0.058594-1.7422-1.6172-3.7852-1.7344-5.5977-0.3125-0.79297 0.62109-1.3945 1.0078-1.9453 1.0156-0.042968 0.003907-0.085937 0.003907-0.125 0.011719-0.042968-0.007812-0.085937-0.011719-0.125-0.011719-0.55078-0.007812-1.1523-0.39453-1.9453-1.0156-1.8125-1.4219-3.8516-1.3047-5.5977 0.3125-1.0234 0.94922-2.0977 0.93359-3.3789-0.058594-1.9492-1.5-3.1445-1.3945-3.4688-1.3242l0.39844 1.875s-0.023438 0.007812-0.066407 0.011718c0.007813 0 0.68359-0.03125 1.9688 0.96094 2.0117 1.5508 4.1445 1.5273 5.8516-0.058594 1.3867-1.2891 2.4414-0.73438 3.1133-0.21094 0.90625 0.71094 1.8945 1.4062 3.1016 1.4258h0.015625c0.046875 0 0.09375-0.007812 0.14062-0.015625 0.046875 0.007813 0.09375 0.015625 0.14062 0.015625h0.015625c1.207-0.019531 2.1914-0.71484 3.1016-1.4258 0.67188-0.52734 1.7266-1.0781 3.1133 0.21094 1.707 1.5859 3.8398 1.6055 5.8555 0.058594 1.2852-0.98828 1.9609-0.96094 1.9648-0.96094-0.046875-0.003906-0.066406-0.011718-0.066406-0.011718l0.39453-1.875c-0.33594-0.070313-1.5352-0.17578-3.4805 1.3242z"
              fill={(index === editingIndex && editingSensor ? editingSensor.color : sensor.color)}
            />
          {/if}
        </svg>
      {/each}
    </div>
    {#if save_modal && cancel_modal && delete_modal}
      <SensorControls
        {sensors}
        {editingIndex}
        bind:editingSensor={editingSensor}
        bind:sensorSize={sensorSize}
        bind:alert={alert}
        {sensorColors}
        {sensorGroups}
        {cancel_modal}
        {delete_modal}
        {save_modal}
        {saveBackgroundChanges}
      />
    {/if}
  </div>
</div>
{/if}

<SaveModal bind:save_modal={save_modal} {saveSensorChanges}/>
<CancelModal bind:cancel_modal={cancel_modal} {handleSensorChanges}/>
<DeleteModal bind:delete_modal={delete_modal} {deleteSensor}/>
<Alert bind:alert={alert}/>
