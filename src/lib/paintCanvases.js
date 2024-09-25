export function paintBackground(context, img) {
  const { width, height } = context.canvas;

  context.drawImage(img, 0, 0, width, height);
  console.log('Background Drawn');
}

export function paintSensors(context, sensors, currSensor, img) {
  const { width, height } = context.canvas;
  
  context.clearRect(0, 0, width, height);

  const index = (currSensor ? sensors.findIndex(sensor => sensor.id === currSensor.id) : null);
  for (let i = 0; i < sensors.length; i++) {
    if (i === index && index !== null) {
      let x = currSensor.x_pos;
      let y = currSensor.y_pos;
  
      if (currSensor.group !== 'temperature' && currSensor.group !== 'pressure') {
        context.fillStyle = currSensor.color;
        context.fillRect(x * width - 5, y * height - 5, 10, 10);
        context.beginPath();
        context.arc(x * width, y * height, 15, 0, 2 * Math.PI);
        context.lineWidth = 2;
        context.strokeStyle = currSensor.color;
        context.stroke();
      } else {
        context.drawImage(img, x * width - img.width / 2, y * height - img.height / 2);
      }
      continue;
    }
  
    let x = sensors[i].x_pos;
    let y = sensors[i].y_pos;
  
    if (sensors[i].group !== 'temperature' && sensors[i].group !== 'pressure') {
      context.fillStyle = sensors[i].color;
      context.fillRect(x * width - 5, y * height - 5, 10, 10);
    } else {
      context.drawImage(img, x * width - img.width / 2, y * height - img.height / 2);
    }
    console.log('Sensors Drawn');
  }
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