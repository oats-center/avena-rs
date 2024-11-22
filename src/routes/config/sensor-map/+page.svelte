<script lang='ts'>
  import { onMount } from 'svelte';
  import { useLocalStorage, getLocalImage } from '$lib/localStorage.svelte';
  import { paintBackground, paintSensors } from '$lib/paintCanvases';
  import SensorControls from '$lib/SensorControls.svelte'
  import background from "$lib/images/background.png";
  import temperature_sensor from "$lib/images/temperature_sensor.png";
  import pressure_sensor from "$lib/images/pressure_sensor.png";

  type Sensor = {
    id: string;
    x_pos: number;
    y_pos: number;
    color: string;
    layer: number;
    name: string;
    group: string;
  }

  let sensors: Sensor[] = useLocalStorage<Sensor[]>('sensors', []).value;
  let sensorColors = ['red', 'orange', 'yellow', 'green', 'blue', 'purple', 'grey', 'black'];
  let sensorGroups = ['', 'temperature', 'pressure']
  let currSensor = $state<Sensor | null>(null);
  let queuedSensor: Sensor | null = null;
  let bgCanvas: HTMLCanvasElement;
  let fgCanvas: HTMLCanvasElement; 
  let bgContext: CanvasRenderingContext2D| null, fgContext: CanvasRenderingContext2D| null;
  let mouseX = 0;
  let mouseY = 0;
  let xMax = $state(0);
  let yMax = $state(0);
  let resizeTimeout: number;
  let animationFrameId: number;
  let backgroundImage: HTMLImageElement;
  let sensorImages: HTMLImageElement[] = [];
  let cancel_modal = $state<HTMLDialogElement>();
  let delete_modal = $state<HTMLDialogElement>();
  let save_modal = $state<HTMLDialogElement>();
  

  onMount(() => {
    let tempBg = getLocalImage('background', "").value;
    const tempImage = new Image();
    tempImage.src = temperature_sensor;
    tempImage.onload = function() {
      sensorImages.push(tempImage);
    }
    const pressImage = new Image();
    pressImage.src = pressure_sensor;
    pressImage.onload = function() { 
      sensorImages.push(pressImage);
    }
    let fgImgs = [tempImage, pressImage];
    const img = new Image();
    img.src = tempBg ? tempBg : background;
    img.onload = function () {
      backgroundImage = img;
      setupCanvases(fgImgs, img);
    }
//
    
    // Initial render
    window.addEventListener('resize', function(e) {
      resizeCanvas();
    })
  });
  
  function animateForeground(): void {
    if (currSensor) {
      renderForeground(null);
      // Keep requesting animation frames as long as currSensor is not null
      animationFrameId = requestAnimationFrame(animateForeground);
    } else {
      // Stop the animation when currSensor is null
      cancelAnimationFrame(animationFrameId);
      renderForeground(null);
    }
  }

  function setupCanvases(fgImages: HTMLImageElement[], bgImage: HTMLImageElement): void {
    xMax = window.innerWidth * 0.60;
    yMax = window.innerHeight * 0.80;
    
    let width = bgImage ? bgImage.width : backgroundImage.width;
    let height = bgImage ? bgImage.height : backgroundImage.height;


    let maxTopHeight = yMax * 0.75;
    let width_ratio = xMax / width;
    let height_ratio = maxTopHeight / height;

    if(width_ratio < height_ratio){
      xMax = width * width_ratio;
      yMax = height * width_ratio;
    } else {
      xMax = width * height_ratio;
      yMax = height * height_ratio;
    }

    bgContext = bgCanvas.getContext('2d');
    bgCanvas.width = xMax;
    bgCanvas.height = yMax;

    fgContext = fgCanvas.getContext('2d');
    fgCanvas.width = xMax;
    fgCanvas.height = yMax;

    renderBackground(bgImage ?? backgroundImage);
    renderForeground(fgImages);
  }

  function renderBackground(img: HTMLImageElement | null): void  {
    if(bgContext !== null) {
      paintBackground(bgContext, img ?? backgroundImage);
    }
    
  }

  function renderForeground(img: HTMLImageElement[] | null): void {
    if(fgContext !== null) {
      paintSensors(fgContext, sensors, currSensor, img ?? sensorImages);
    }
  }

  function resizeCanvas(): void {
    clearTimeout(resizeTimeout);
    resizeTimeout = setTimeout(() => {
      setupCanvases(sensorImages, backgroundImage);
    }, 200);
  }
  
  function handleMouseMove(event : MouseEvent): void {
    const rect = fgCanvas.getBoundingClientRect();
    mouseX = event.clientX - rect.left;
    mouseY = event.clientY - rect.top;
  }

  function generateRandomId(): string{
    const length = 5;
    const characters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    let result = '';
    for (let i = 0; i < length; i++) {
      result += characters.charAt(Math.floor(Math.random() * characters.length));
    }
    return result;
  }

  function addSensor(): void {
    const newSensor: Sensor = {
      id: generateRandomId(),
      x_pos: 0,
      y_pos: 0,
      color: "red",
      layer: 1,
      name: "New Sensor",
      group: ""
    };
    sensors = [...sensors, newSensor];
    currSensor = JSON.parse(JSON.stringify(sensors[sensors.length - 1]));
    animateForeground();
  }
  
  function updateSensors(): void {
    if(currSensor !== null){
      const index = sensors.findIndex(sensor => sensor.id === currSensor!.id);
      if (index !== -1) {
        sensors = [
          ...sensors.slice(0, index),
          currSensor,
          ...sensors.slice(index + 1)
        ];
      }
      currSensor = null;
      renderForeground(null);
    } 
  }

  function deleteSensor(): void {
    if(currSensor !== null){
      const index = sensors.findIndex(sensor => sensor.id === currSensor!.id);
      if (index !== -1) {
        sensors = [
          ...sensors.slice(0, index),
          ...sensors.slice(index + 1)
        ];
        renderForeground(null);
      }
      currSensor = null;
    }
    
  }

  function handleClick(): void {
    const clickedSensor = sensors.find(sensor =>
      (sensor.group === "" && mouseX > sensor.x_pos * xMax - 5 && mouseX < sensor.x_pos * xMax + 5  &&
       mouseY > sensor.y_pos * yMax - 5 && mouseY < sensor.y_pos * yMax + 5) ||
       (sensor.group !== "" && mouseX > sensor.x_pos * xMax - 25 && mouseX < sensor.x_pos * xMax + 25  &&
       mouseY > sensor.y_pos * yMax - 25 && mouseY < sensor.y_pos * yMax + 25)
    ) || null;
    if (clickedSensor) {
      if (currSensor) {
        queuedSensor = { ...clickedSensor };
        cancel_modal?.showModal();
      } else {
        queuedSensor = null;
        currSensor = { ...clickedSensor };
      }
      if(currSensor){
        animateForeground();
      }
    }
  }
</script>

<div class="flex justify-center p-10 space-x-16">
  <a href="/sensor-map" class="btn btn-outline btn-primary btn-lg">Sensor Map</a>
  <a href="/lj-config" class="btn btn-outline btn-primary btn-lg">Sensor Config</a>
</div>
<div class="flex">
  <div class="canvas_container">
    <canvas class="background" bind:this={bgCanvas}></canvas>
    <canvas class="foreground" onclick={handleClick} onmousemove={handleMouseMove} bind:this={fgCanvas}></canvas>
  </div>
  <SensorControls
    {currSensor}
    {sensorColors}
    {sensorGroups}
    onAddSensor={addSensor}
    {cancel_modal}
    {delete_modal}
    {save_modal}
  />
</div>
<dialog id="cancel_modal" class='modal' bind:this={cancel_modal}>
  <div class="modal-box">
    <h3 class="text-lg font-bold">Cancel Changes?</h3>
    <h6>Pressing 'Yes' will delete all changes made without saving</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-primary">No</button>
        <button class="btn btn-error ml-5" onclick={ () => {
          queuedSensor ? currSensor = queuedSensor : currSensor = null;
          queuedSensor = null;
        }}>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>

<dialog id="delete_modal" class='modal' bind:this={delete_modal}>
  <div class="modal-box">
    <h3 class="text-lg font-bold">Delete Sensor?</h3>
    <h6>Pressing 'Yes' will delete the currently selected sensor and that data will be unrecoverable</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-primary">No</button>
        <button class="btn btn-error ml-5" onclick={ () => deleteSensor() }>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>

<dialog id="save_modal" class='modal' bind:this={save_modal}>
  <div class="modal-box">
    <h3 class="text-lg font-bold">Save Changes?</h3>
    <h6>Pressing 'Yes' will override the already saved data with the new data.</h6>
    <div class="mt-5 flex">
      <form method="dialog">
        <button class="btn btn-primary">No</button>
        <button class="btn btn-success ml-5" onclick={ () => updateSensors() }>Yes</button>
      </form>
    </div>
  </div>
  <form method="dialog" class="modal-backdrop">
    <button>close</button>
  </form>
</dialog>


<style>
  .canvas_container {
    position: relative;
    width: 75%;
    height: 100%;
  }
  .foreground {
    z-index: 1
  }
  .background {
    z-index: 0;
  }
  canvas {
    position: absolute;
    top: 1;
    left: 10vw;
  }
</style>
