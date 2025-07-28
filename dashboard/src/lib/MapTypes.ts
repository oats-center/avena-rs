//MapConfig and SensorTypes what will be pulled from nats and then parsed to different formatting
export type MapConfig = {
  "backgroundImage": string;
  [key: `labjackd.${string}.ch${string}`]: Sensor;
}

export type SensorTypes = {
  [name: string]: {
    "size_px": number
    "icon": string
  }
}

//Type information parsed from SensorTypes
export type SensorType = {
  "name": string
  "size_px" : number;
  "icon" : string;
}

//Sensor information parsed from MapConfig
export type Sensor = {
  "cabinet_id" : string;
  "labjack_serial" : string;
  "connected_channel": string; 
  "sensor_name" : string; 
  "sensor_type" : string; 
  "x_pos" : number; 
  "y_pos" : number; 
  "color" : string; 
}