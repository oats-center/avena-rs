<script lang='ts'>
  import type { Sensor, SensorType } from "$lib/MapTypes"

  let {sensors, editingSensor, editingIndex, sensor_types, backgroundImage, background = $bindable(), handleSensorChanges} : {
    sensors: Sensor[] | null,
    editingSensor: Sensor | null,
    editingIndex: number,
    backgroundImage: string | null, 
    sensor_types: SensorType[] | null,
    background: HTMLImageElement | null,
    handleSensorChanges: Function,
  } = $props()
  
  let dragging = $state<boolean>(false);
    
  //when mouse down on a sensor, start dragging
  function handleDragStart(e: MouseEvent): void {
    if(e.which === 1) dragging = true;
  }

  //when the mouse moves, continue dragging
  function continueDrag(e: MouseEvent): void {
    if (!background || !editingSensor || !dragging) return;

    const scaleX = 100 / background.width;
    const scaleY = 100 / background.height;
    
    const back_loc = background.getBoundingClientRect()

    editingSensor.x_pos = (e.clientX - back_loc.x) * scaleX;
    editingSensor.y_pos = (e.clientY - back_loc.y) * scaleY;
    
    editingSensor.x_pos = Math.min(100, Math.max(0, editingSensor.x_pos));
    editingSensor.y_pos = Math.min(100, Math.max(0, editingSensor.y_pos));
  }

  //when the mouse button is back up, stop dragging and round values
  function stopDrag(): void {
    dragging = false;
    if (!editingSensor) return;

    editingSensor.x_pos = Math.round(editingSensor.x_pos)
    editingSensor.y_pos = Math.round(editingSensor.y_pos)
    
    editingSensor.x_pos = Math.min(100, Math.max(0, editingSensor.x_pos));
    editingSensor.y_pos = Math.min(100, Math.max(0, editingSensor.y_pos));
  }

  //gets any sensor type property for use on map
  function findSensorProperty<T>(curr_type: string, index: number, property: keyof SensorType): T | undefined {
    if (!sensor_types) throw new Error("Sensor Types Not Initialized")
    if (!sensors) throw new Error("Sensors Not Initialized")

    const targetSensorType = editingIndex === index && editingSensor
      ? editingSensor.sensor_type.toLowerCase()
      : curr_type.toLowerCase();

    const sensor = sensor_types.find(sensor_type => sensor_type.name.toLowerCase() === targetSensorType);
    return sensor ? sensor[property] as T : undefined;
  }

</script>

<div role="none" class="fixed" onmousemove={continueDrag}>
  <img 
    alt="Background for Map" 
    src={backgroundImage}
    bind:this={background}
    style="z-index: -1; height: 90vh; position: relative;"
  />
  {#if sensors}
    {#each sensors as sensor, index}
      <div
        role="button"
        tabindex=0
        style={`
          position: absolute; 
          top: calc(${(index === editingIndex && editingSensor ? editingSensor.y_pos : sensor.y_pos)}% - ${(findSensorProperty(sensor.sensor_type, index, "size_px") as number) / 2}px);
          left: calc(${(index === editingIndex && editingSensor ? editingSensor.x_pos : sensor.x_pos)}% - ${(findSensorProperty(sensor.sensor_type, index, "size_px") as number) / 2}px);
          min-width: ${(findSensorProperty(sensor.sensor_type, index, "size_px") as number)}px
          min-height: ${(findSensorProperty(sensor.sensor_type, index, "size_px") as number)}px
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
          src={findSensorProperty(sensor.sensor_type, index, "icon")}
          width={`${findSensorProperty(sensor.sensor_type, index, "size_px")}px`}
          height={`${findSensorProperty(sensor.sensor_type, index, "size_px")}px`}
          draggable={false}
          alt="sensor icon"
          style={`min-width: ${findSensorProperty(sensor.sensor_type, index, "size_px")}px; min-height: ${findSensorProperty(sensor.sensor_type, index, "size_px")}px;`}
          />
      </div>
    {/each}
  {/if}
</div>