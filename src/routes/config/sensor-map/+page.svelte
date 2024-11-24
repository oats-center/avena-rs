<script lang='ts'>
  import { onMount } from 'svelte';
  import SensorControls from '$lib/SensorControls.svelte';
  import { paintBackground, paintSensors } from '$lib/paintCanvases';
  import { connect, getKeys, getKeyValues, NatsService, putKeyValue } from '$lib/nats.svelte';

  import temperature_sensor from "$lib/images/temperature_sensor.svg";
  import pressure_sensor from "$lib/images/pressure_sensor.svg";

  type Sensor = {
    "cabinet_id" : string; //cabinet connected to sensor
    "labjack_serial" : string; //labjack serial number
    "connected_channel": number //need to know when accessing config / data
    "sensor_name" : string; //for visuals
    "sensor_type" : string; //for visuals
    "x_pos" : number; //(0 <= x_pos <= 1)
    "y_pos" : number; //(0 <= y_pos <= 1)
    "color" : string //for visuals
  }
  let serverName: string | null;
  let selectedCabinet: string | null;
  let nats: NatsService | null;
  const sensorColors = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'grey', 'black'];
  const sensorGroups = ['', 'temperature', 'pressure']
  let cancel_modal = $state<HTMLDialogElement>();
  let delete_modal = $state<HTMLDialogElement>();
  let save_modal = $state<HTMLDialogElement>();

  let sensors = $state<Sensor>();
  let editingSensor = $state<Sensor | null>(null);
  let bgCanvas: HTMLCanvasElement;
  let fgCanvas: HTMLCanvasElement; 
  let bgContext: CanvasRenderingContext2D| null, fgContext: CanvasRenderingContext2D| null;

  async function initialize() {
    if(serverName) nats = await connect(serverName);
    if(nats && selectedCabinet) {
      let sensorList = await getKeys(nats, selectedCabinet, "sensors.mapconfig.*")
    }
  }

  onMount(() => {
    serverName = sessionStorage.getItem("serverName");
    selectedCabinet = sessionStorage.getItem("selectedCabinet")
    initialize();
  })


</script>

<!--
labjackd.mapconfig.<serial> = {

}
-->