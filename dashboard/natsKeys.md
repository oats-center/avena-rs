# NATS Keys

All keys listed in this document are stored within the `localhost` server.

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
Each cabinet then has its own bucket, containing:
- `labjackd.config.SERIAL`: LabJack Config Settings (where SERIAL is the labjack serial number)

#### Example:
```json
// bucket: road1_cabinet1
"labjackd.config.TEST001": {
  "cabinet_id": "avenabox_001",
  "labjack_name": "Main Sensor Hub",
  "serial": "TEST001",
  "sensor_settings": {
    "sampling_rate": 7000,
    "scan_rate": 500,
    "channels_enabled": [1, 2, 4],
    "gains": 1,
    "data_formats": ["voltage", "temperature", "current"],
    "measurement_units": ["V", "°C", "A"],
    "labjack_reset": false
  }
}
```

## Data Structure Notes

### LabJack Configuration
- **sampling_rate**: Data sampling frequency in Hz
- **scan_rate**: Channel scanning frequency in Hz  
- **channels_enabled**: Array of active channel numbers (1-8)
- **gains**: Amplification factor for signal processing
- **data_formats**: Data type for each enabled channel
- **measurement_units**: Units of measurement for each enabled channel
- **labjack_reset**: Boolean flag for device reset status

### Cabinet Status Values
- `"online"`: Cabinet is operational and accepting configurations
- `"offline"`: Cabinet is disconnected or unreachable
- `"maintenance"`: Cabinet is in maintenance mode (read-only access)
