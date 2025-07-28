# NATS Keys

All keys listed in this document are stored within the `demo.nats.io` server.

In the program, NATS KeyValue stores are accessed using various functions in the `$lib/nats.svelte.ts` file.

Throughout this file, many buckets/keys will include the letter `N`. This is to represent any combination of numbers that could be used to differentiate cabinets while maintaining the same bucket/key structure.

---

## Buckets Currently Available

The following buckets are currently available:

- `all_cabinets`
- `road1_cabinet1`
- `road1_cabinet2`
- `road1_cabinet3`
- `road2_cabinet1`

---

## Keys/Values Within Buckets

### `all_cabinets`
This bucket contains a key for each cabinet with the format `roadN_cabinetN`. Each key holds a JSON object that includes the cabinet's current status (`"on"` or `"off"`). The structure may be expanded in the future to include additional fields.

#### Example:
```json
"road1_cabinet1": { "status": "off" }
```

### `roadN_cabinetN`
Each cabinet then has its own bucket, containing a few different key variations:
- `labjackd.config.N`: LabJack Config Settings (where N is the labjack serial number)
- `mapconfig`: All cabinet information for map view
- `sensor_types`: Sensor type definitions for map view

#### Example:
```json
//bucket: road1_cabinet1
"labjackd.config.1": { 
  "cabinet_id": "road1_cabinet1",
  "labjack_name": "Labjack 2",
  "serial": "2",
  "sensor_settings": {
    //subject to change
  }
}
"mapconfig": {
  "backgroundImage": "data:image/png;base64...",
  "labjackd.1.ch1": {
    "cabinet_id":"road1_cabinet1",
    "labjack_serial":"1",
    "connected_channel":"1",
    "sensor_name":"New Sensor",
    "sensor_type":"Temp",
    "x_pos":22,
    "y_pos":64,
    "color":"red"
  },
  "labjackd.1.ch2": {
    ...
  }
}
"sensor_types": {
  "Press": {
    "icon":"data:image/svg+xml;base64...",
    "size_px": 50
  }, 
  "Temp": {
    ...
  }
}
```

