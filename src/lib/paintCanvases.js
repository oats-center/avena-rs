export function paintBackground(context, img) {
  const { width, height } = context.canvas;

  context.drawImage(img, 0, 0, width, height);
  console.log('Background Drawn');
}

export function paintSensors(context, sensors, currSensor, imgs) {
  const { width, height } = context.canvas;

  context.clearRect(0, 0, width, height);

  const index = currSensor ? sensors.findIndex(sensor => sensor.id === currSensor.id) : null;

  for (let i = 0; i < sensors.length; i++) {
    let sensor = sensors[i];
    let x = sensor.x_pos;
    let y = sensor.y_pos;

    // Handle currSensor separately
    if (i === index && currSensor) {
      sensor = currSensor;
      x = currSensor.x_pos;
      y = currSensor.y_pos;
    }

    if (sensor.group !== 'temperature' && sensor.group !== 'pressure') {
      // Draw non-image sensors (non-temperature/pressure)
      context.fillStyle = sensor.color;
      context.fillRect(x * width - 5, y * height - 5, 10, 10);

      if(i === index){
        context.beginPath();
        context.arc(x * width, y * height, 15, 0, 2 * Math.PI);
        context.lineWidth = 2;
        context.strokeStyle = sensor.color;
        context.stroke();
      }
      
    } else {
      let img = sensor.group === 'temperature' ? imgs[0] : imgs[1]; 
      let scale = 0.5
      context.drawImage(img, x * width - (img.width * scale ) / 2, y * height - (img.height * scale) / 2, img.width * scale, img.height * scale);

      context.globalCompositeOperation = 'source-atop';
      context.fillStyle = sensor.color;
      context.fillRect(x * width - (img.width * scale) / 2, y * height - (img.height * scale) / 2, img.width * scale, img.height * scale);

      context.globalCompositeOperation = 'source-over';
    }

    if (i === index && currSensor) {
      continue;
    }
  }

  console.log('Sensors and currSensor Drawn');
}


// Paint Gap
    /*context.fillStyle = "#1f1f1f";
    context.fillRect((width - img_width) / 2, topViewHeight, width, dividerHeight);

    // Paint side view base
    context.fillStyle = "#000000";
    context.fillRect((width - img_width) / 2, topViewHeight + dividerHeight, img_width, sideViewHeight * (1 / 3));

    context.fillStyle = "#444444";
    context.fillRect((width - img_width) / 2, topViewHeight +dividerHeight + sideViewHeight * (1 / 3), img_width, sideViewHeight * (1 / 3));

    context.fillStyle = "#999999";
    context.fillRect((width - img_width) / 2, topViewHeight + dividerHeight + sideViewHeight * (2 / 3), img_width, sideViewHeight);

    // Paint the sensors locations
    for (let i = 0; i < sensors.length; i++) {
      // if index exists, it paints the temporary sensor instead of the saved information about it
      if(i === index) {
        context.fillStyle = currSensor.color;
        let x = currSensor.x_pos;
        let y = currSensor.layer;
        context.fillRect(x, topViewHeight + dividerHeight + y * sideViewHeight * (1 / 3) - 3, 10, 6);
        continue;
      }
      context.fillStyle = sensors[i].color;
      let x = sensors[i].x_pos;
      let y = sensors[i].layer;
      context.fillRect(x, topViewHeight + dividerHeight + y * sideViewHeight * (1 / 3) - 3, 10, 6);
    }
  };*/