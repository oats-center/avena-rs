export class Sensor {
  constructor(id, x_pos, y_pos, color, layer, name, group) {
    this.id = id;
    this.x_pos = x_pos;
    this.y_pos = y_pos;
    this.color = color;
    this.layer = layer;
    this.name = name;
    this.group = group;
  }

  // Method to update sensor position
  updatePosition(newX, newY) {
    this.x_pos = newX;
    this.y_pos = newY;
  }

  // Method to change the sensor group
  changeColor(newColor) {
    this.color = newColor;
  }

  // Method to update the layer/height
  updateLayer(newLayer) {
    this.layer = newLayer;
  }

  // Method to update sensor name
  updateName(newName) {
    this.name = newName;
  }
}