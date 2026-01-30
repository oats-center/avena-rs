# Rust-LJM: LabJack Data Acquisition in Rust

A Rust-based data acquisition system for LabJack devices with NATS integration.
It includes a LabJack streamer, a parquet archiver, and a CSV exporter over WebSocket.

## Prerequisites

- Rust (latest stable)
- LabJack LJM library
- NATS server
- NATS credentials file (for JetStream KV + publish/subscribe)

## Quick Start (single host)

1. Install the LabJack LJM library.
2. Build the project:
   ```bash
   cargo build --release
   ```
3. Run the binaries:
   ```bash
   cargo run --bin streamer
   cargo run --bin archiver
   cargo run --bin exporter
   ```

## Available Binaries

- `streamer` - Stream data from LabJack to NATS
- `archiver` - Subscribe to NATS and write parquet files to disk
- `exporter` - Serve parquet as streamed CSV over WebSocket (`/export`)
- `subscriber` - Subscribe to NATS data streams (diagnostics)

## Configuration (JetStream KV)

Create a KV entry for each LabJack (example key: `labjackd.config.i69-mu1`):

```
{
  "labjack_name": "I69-MU1",
  "asset_number": 1001,
  "max_channels": 8,
  "nats_subject": "avenabox",
  "nats_stream": "labjacks",
  "rotate_secs": 600,
  "sensor_settings": {
    "scan_rate": 200,
    "sampling_rate": 1000,
    "channels_enabled": [0, 1, 2],
    "gains": 1,
    "data_formats": ["voltage", "temperature", "pressure"],
    "measurement_units": ["V", "C", "PSI"],
    "labjack_on_off": true,
    "calibrations": {}
  }
}
```

Set the key/bucket using environment variables (see `env-setup.sh`).

## Environment variables

Sourced by `env-setup.sh`:

- `NATS_CREDS_FILE` (required) - NATS credentials file path
- `CFG_BUCKET` (default: `avenabox`)
- `CFG_KEY` (required) - KV key for the LabJack config
- `LABJACK_IP` (streamer only) - LabJack IP address
- `ROLE` - `edge` or `server` (used by `deploy-binary.sh`)

Used by exporter:

- `PARQUET_DIR` (default: `parquet`)
- `EXPORTER_ADDR` (default: `0.0.0.0:9001`)

## Recommended deployment

- Edge (LabJack connected): `streamer`
- Server (storage + webapp): `archiver` + `exporter`

This keeps the LabJack close to the device and stores parquet data on the server.
`exporter` must run on the same host (or a host with the same parquet directory)
because it reads the local parquet files directly.

## Using the deploy script

```
./env-setup.sh && ROLE=edge ./deploy-binary.sh start
./env-setup.sh && ROLE=server ./deploy-binary.sh start
```

The script prevents multiple copies of the same binary and writes logs to `logs/`.

## Features

- Real-time data streaming
- NATS message publishing
- Parquet archiving
- CSV export over WebSocket
- FlatBuffer serialization
- Multi-channel support
