# Rust-LJM

Rust binaries for LabJack streaming, parquet archiving, and export serving.

## Binaries

- `streamer`: reads from the LabJack and publishes samples to NATS
- `archiver`: subscribes to NATS and writes parquet files
- `exporter`: serves parquet-backed exports over WebSocket
- `subscriber`: diagnostic NATS subscriber

## Deployment

- MU / edge host with the LabJack attached: run `streamer`
- Remote server with storage and webapp support: run `archiver` and `exporter`

`exporter` must run on the same host as the parquet directory it serves.

## Streamer Control

Edit `streamer.env.json`, then control the streamer with:

```bash
./streamerctl.sh start
./streamerctl.sh status
./streamerctl.sh restart
./streamerctl.sh stop
```

Behavior:

- `start` builds `target/release/streamer` if needed
- only one streamer process is allowed at a time
- `restart` stops the existing process before starting a new one
- logs go to `logs/streamer.log`
- the PID file is stored in `.runtime/streamer.pid`

To use a different env file:

```bash
CONFIG_FILE=/path/to/streamer.env.json ./streamerctl.sh restart
```

## Streamer Env Config

`streamer.env.json` contains the environment variables exported before `streamer`
starts.

Important fields:

- `NATS_CREDS_FILE`: path to the NATS creds file
- `CFG_BUCKET`: JetStream KV bucket
- `CFG_KEY`: JetStream KV key for the LabJack config
- `LABJACK_IP`: optional direct LabJack IP
- `LABJACK_SERIAL`: optional serial filter when multiple LabJacks are on the subnet
- `LABJACK_OPEN_ORDER`: usually `ethernet,usb`

If `LABJACK_IP` is empty on Linux, the streamer will scan local IPv4 subnets for
hosts with TCP `502` open and then verify which host is a T7.

## LabJack KV Config

The JSON stored in JetStream KV should use the newer structure:

```json
{
  "labjack_name": "Macbook",
  "asset_number": 1456,
  "max_channels": 14,
  "nats_subject": "avenabox",
  "nats_stream": "labjacks",
  "rotate_secs": 300,
  "sensor_settings": {
    "scan_rate": 100,
    "sampling_rate": 500,
    "channels_enabled": [11, 13],
    "gains": 1,
    "data_formats": ["voltage", "voltage"],
    "measurement_units": ["V", "V"],
    "labjack_on_off": true,
    "calibrations": {
      "11": { "type": "identity" },
      "13": { "type": "identity" }
    }
  }
}
```

## Manual Server Binaries

On the remote server:

```bash
cargo run --release --bin archiver
cargo run --release --bin exporter
```

`exporter` uses:

- `PARQUET_DIR` default: `parquet`
- `EXPORTER_ADDR` default: `0.0.0.0:9001`
