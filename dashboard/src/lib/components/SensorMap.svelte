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
  function continueDrag(event: MouseEvent): void {
    if (!background || !editingSensor || !dragging) return;

    const back_loc = background.getBoundingClientRect()
    const scaleX = 100 / background.width;
    const scaleY = 100 / background.height;

    editingSensor.x_pos = (event.clientX - back_loc.x) * scaleX;
    editingSensor.y_pos = (event.clientY - back_loc.y) * scaleY;

    editingSensor.x_pos = Math.round(editingSensor.x_pos * 10) / 10;
    editingSensor.y_pos = Math.round(editingSensor.y_pos * 10) / 10;
  }

  //when the mouse button is back up, stop dragging and round values
  function stopDrag(): void {
    dragging = false;
    if (!editingSensor) return;

    editingSensor.x_pos = Math.round(editingSensor.x_pos)
    editingSensor.y_pos = Math.round(editingSensor.y_pos)
    
    editingSensor.x_pos = Math.min(100, Math.max(0, editingSensor.x_pos));
    editingSensor.y_pos = Math.min(100, Math.max(0, editingSensor.y_pos));
    
    // Save the new position to NATS
    handleSensorChanges(editingSensor, editingIndex);
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

<div class="relative w-full h-full" onmousemove={continueDrag}>
  <img 
    alt="Background for Map" 
    src={backgroundImage}
    bind:this={background}
    class="w-full h-full object-contain max-h-[85vh]"
    onclick={(e) => {
      // Handle background clicks - deselect sensor if clicking on empty area
      if (e.target === e.currentTarget) {
        handleSensorChanges();
      }
    }}
  />
  {#if sensors && background}
    {#each sensors as sensor, index}
      <div
        role="button"
        tabindex=0
        class="absolute cursor-pointer z-10 hover:scale-110 transition-transform"
        style={`
          top: ${(index === editingIndex && editingSensor ? editingSensor.y_pos : sensor.y_pos)}%;
          left: ${(index === editingIndex && editingSensor ? editingSensor.x_pos : sensor.x_pos)}%;
          transform: translate(-50%, -50%);
          width: 24px;
          height: 24px;
          border: ${index === editingIndex ? "3px solid #fbbf24" : "2px solid rgba(255,255,255,0.3)"};
          border-radius: 8px;
          background: rgba(0,0,0,0.1);
          backdrop-filter: blur(4px);
        `}
        onmousedown={(event) => {if(editingIndex === index) handleDragStart(event)}}
        onmouseup={() => {if(editingIndex === index) stopDrag()}}
        onmousemove={(event) => {if(editingIndex != -1) continueDrag(event)}}
        onclick={() => handleSensorChanges(sensor, index)}
        onkeydown={() => handleSensorChanges(sensor, index)}
      >
        <img 
          src={findSensorProperty(sensor.sensor_type, index, "icon")}
          draggable={false}
          alt="sensor icon"
          class="w-full h-full object-contain p-1"
        />
        <div class="absolute -bottom-6 left-1/2 transform -translate-x-1/2 text-xs text-white bg-black/50 px-2 py-1 rounded whitespace-nowrap">
          {sensor.sensor_name || `${sensor.sensor_type} ${sensor.connected_channel}`}
        </div>
      </div>
    {/each}
  {/if}
</div>