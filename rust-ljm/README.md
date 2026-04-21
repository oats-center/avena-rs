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
For a simple setup, you can also run `archiver` and `exporter` on the MU and
point the laptop webapp at the MU's exporter address.

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

## Archiver Control

Edit `archiver.env.json`, then control the archiver with:

```bash
./archiverctl.sh start
./archiverctl.sh status
./archiverctl.sh restart
./archiverctl.sh stop
```

`archiver` subscribes to NATS and writes parquet files locally under `parquet/`.

## Exporter Control

Edit `exporter.env.json`, then control the exporter with:

```bash
./exporterctl.sh start
./exporterctl.sh status
./exporterctl.sh restart
./exporterctl.sh stop
```

Set `EXPORTER_ADDR` to an address reachable from the laptop, for example:

```json
{
  "env": {
    "PARQUET_DIR": "parquet",
    "EXPORTER_ADDR": "0.0.0.0:9001"
  }
}
```

## Streamer Env Config

`streamer.env.json` contains the environment variables exported before `streamer`
starts.

Important fields:

- `NATS_CREDS_FILE`: path to the NATS creds file
- `CFG_BUCKET`: JetStream KV bucket
- `CFG_KEY`: JetStream KV key for the LabJack config
- `LABJACK_IP`: required direct LabJack IP for `streamer`
- `LABJACK_SERIAL`: optional but recommended post-connect serial verification
- `LABJACK_NAME`: optional logical device name for logging

`streamer` now uses a strict Ethernet IP path only:

- no subnet scan
- no indirect serial/name discovery
- no USB fallback

On connect, `streamer`:

- opens the T7 directly via `LABJACK_IP`
- verifies the connected handle is a T7
- verifies `LABJACK_SERIAL` if provided
- runs a minimal read/write self-test using `STREAM_SETTLING_US`

## FlatBuffer Codegen

The stream payload schema is committed in `src/data.fbs`, and the generated
bindings are also committed:

- Rust: `src/data_generated.rs`
- TypeScript: `../webapp/src/lib/sampler.ts` and `../webapp/src/lib/sampler/scan.ts`

When `src/data.fbs` changes, regenerate both files from the repo root:

```bash
flatc --rust -o rust-ljm/src rust-ljm/src/data.fbs
flatc --ts --gen-object-api -o webapp/src/lib rust-ljm/src/data.fbs
```

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
    "scans_per_read": 100,
    "scan_rate_hz": 500,
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

Legacy KV configs using `scan_rate` and `sampling_rate` are still accepted on
read, but new configs should use `scans_per_read` and `scan_rate_hz` so the
names match the actual LabJack stream semantics.

## MU + Laptop Setup

If `streamer` is already running on the MU, you can also run:

```bash
./archiverctl.sh start
./exporterctl.sh start
```

Then run the webapp on the laptop and connect it to the MU's exporter endpoint.
That works as long as:

- the laptop can reach the MU over the network
- `EXPORTER_ADDR` is bound to a reachable interface
- the firewall allows the exporter port

This is fine for local testing or a small deployment. The main tradeoff is that
parquet storage and export serving both stay on the MU instead of a separate
server.
