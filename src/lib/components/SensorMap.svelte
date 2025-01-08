<script lang='ts'>
  import pressure_sensor from '$lib/images/pressure_sensor.svg';
  import { onMount } from 'svelte';
  
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

  let {sensors, editingSensor, editingIndex, sensorSize, backgroundImage, handleSensorChanges} : {
    sensors: Sensor[],
    editingSensor: Sensor | null,
    editingIndex: number,
    sensorSize: number,
    backgroundImage: string | null, 
    handleSensorChanges: Function,
  } = $props()

  let background = $state<HTMLImageElement | null>();
  let dragging = $state<boolean>(false);
    
  //map: when mouse down on a sensor, start dragging
  function handleDragStart(e: MouseEvent): void {
    console.log("dragging")
    dragging = true;
  }

  //map: when the mouse moves, continue dragging
  function continueDrag(e: MouseEvent): void {
    if (!background || !editingSensor || !dragging) return;

    const scaleX = 100 / background.width;
    const scaleY = 100 / background.height;
    
    editingSensor.x_pos = (e.clientX - background.x) * scaleX;
    editingSensor.y_pos = (e.clientY - background.y) * scaleY;

    editingSensor.x_pos = Math.min(100, Math.max(0, editingSensor.x_pos));
    editingSensor.y_pos = Math.min(100, Math.max(0, editingSensor.y_pos));
  }

  //map: when the mouse button is back up, stop dragging and round values
  function stopDrag(): void {
    dragging = false;
    if (!editingSensor) return;

    editingSensor.x_pos = Math.round(editingSensor.x_pos)
    editingSensor.y_pos = Math.round(editingSensor.y_pos)
    
    editingSensor.x_pos = Math.min(100, Math.max(0, editingSensor.x_pos));
    editingSensor.y_pos = Math.min(100, Math.max(0, editingSensor.y_pos));
  }
</script>

<div role="none" class="relative" onmousemove={continueDrag}>
  <img 
    alt="Background for Map" 
    src={backgroundImage}
    bind:this={background}
    style="z-index: -1; height: 90vh; position: relative;"
  />
  {#each sensors as sensor, index}
    <div
      role="button"
      tabindex=0
      style={`
        position: absolute; 
        top: calc(${(index === editingIndex && editingSensor ? editingSensor.y_pos : sensor.y_pos)}% - ${sensorSize / 2}px);
        left: calc(${(index === editingIndex && editingSensor ? editingSensor.x_pos : sensor.x_pos)}% - ${sensorSize / 2}px);
        min-width: ${sensorSize}px
        min-height: ${sensorSize}px
        border-radius: 8px; 
        outline: ${index === editingIndex ? "2px solid black" : "none"}; 
      `}
      onmousedown={(event) => {if(editingIndex === index) handleDragStart(event)}}
      onmouseup={() => {if(editingIndex === index) stopDrag()}}
      onmousemove={(event) => {if(editingIndex != -1) continueDrag(event)}}
      onclick={() => handleSensorChanges(sensor, index)}
      onkeydown={() => handleSensorChanges(sensor, index)}
    >
      <img 
        src={pressure_sensor}
        width={`${sensorSize}px`}
        height={`${sensorSize}px`}
        draggable={false}
        alt="sensor icon"
        style={`min-width: ${sensorSize}px; min-height: ${sensorSize}px;`}
        />
    </div>
  {/each}
</div>