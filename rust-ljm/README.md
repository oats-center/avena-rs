# Rust-LJM

Rust binaries for LabJack streaming, parquet archiving, and export serving.

## Binaries

- `streamer`: reads from the LabJack and publishes samples to NATS
- `archiver`: subscribes to NATS and writes parquet files
- `exporter`: serves parquet-backed exports over WebSocket, or runs an edge export worker over NATS
- `subscriber`: diagnostic NATS subscriber

## Deployment

- MU / edge host with the LabJack attached: run the local NATS leaf node,
  `streamer`, and `archiver`
- Remote server with webapp support: run the web app against the central OATS
  NATS WebSocket endpoint

`exporter` now supports two modes:

- `direct`: read local parquet and serve `/export` over WebSocket
- `worker`: subscribe for export jobs over core NATS, read local parquet, and
  publish chunked CSV responses back over core NATS

In `direct` mode, `exporter` must run on the same host as the parquet
directory it serves. The standard edge-box setup uses `worker` mode so the
laptop webapp only needs a central NATS WebSocket connection.

For central-webapp exports backed by edge-local parquet:

- run `exporter` in `worker` mode on the MU with access to local parquet
- point the webapp at central OATS NATS as usual
- the browser publishes export requests over its existing NATS WebSocket session
- the webapp includes `box_id` in export requests so the correct edge worker is targeted

The browser-to-worker path uses core NATS subjects, not JetStream, for export chunks:

- request subject: `avenars.<site_id>.<box_id>.<source_id>.export.request`
- reply subject: generated browser inbox

The local LabJack KV config and live sample stream remain on JetStream-backed
subjects as before.

## Runtime Config Sync

`streamer` can treat the central OATS KV entry as the source of truth while
still running against the local leaf node and local JetStream domain.

When `CFG_NATS_SERVERS` or `CENTRAL_NATS_SERVERS` is set in `streamer.env.json`:

- `streamer` connects to the local leaf node as usual
- it bootstraps the local `CFG_BUCKET:CFG_KEY` from the central
  central config bucket/key, defaulting to `CFG_BUCKET:CFG_KEY`
- it keeps watching the central KV key for updates
- each central update is mirrored into local KV
- the existing local KV watcher then restarts the sampler with the new config

This is one-way sync from central to local. Live samples still publish through
the local JetStream domain and leaf connection.

## Service Control

Build the binaries:

```bash
cd /home/user/avena-rs/rust-ljm
cargo build --release
```

Control the user systemd services:

```bash
./avena-service.sh streamer start
./avena-service.sh archiver start
./avena-service.sh exporter start
./avena-service.sh streamer status
./avena-service.sh streamer logs
./avena-service.sh streamer restart
./avena-service.sh streamer stop
```

`archiver` subscribes to NATS and writes parquet files locally under `PARQUET_DIR`.

Set `EXPORTER_ADDR` to an address reachable from the laptop, for example:

```json
{
  "env": {
    "PARQUET_DIR": "parquet",
    "EXPORTER_ADDR": "0.0.0.0:9001"
  }
}
```

`exporter.env.json` fields:

- `EXPORTER_MODE`: `direct` or `worker`
- `EXPORTER_ADDR`: WebSocket listen address for `direct`
- `PARQUET_DIR`: local parquet root for `direct` and `worker`
- `NATS_SERVERS`: NATS URL list for `worker`
- `NATS_CREDS_FILE`: creds file for `worker`
- `BOX_ID` or `EXPORT_BOX_ID`: worker target box id for subject binding

Example `worker` config:

```json
{
  "env": {
    "EXPORTER_MODE": "worker",
    "PARQUET_DIR": "parquet",
    "NATS_SERVERS": "nats://127.0.0.1:4222",
    "NATS_CREDS_FILE": "apt.creds",
    "NATS_SUBJECT": "avenars",
    "SITE_ID": "i69",
    "BOX_ID": "i69-mu1",
    "SOURCE_TYPE": "labjack",
    "SOURCE_ID": "i69-lj2"
  }
}
```

## Streamer Env Config

`streamer.env.json` contains the environment variables exported before `streamer`
starts.

Important fields:

- `NATS_CREDS_FILE`: path to the NATS creds file
- `NATS_SERVERS`: local NATS leaf node URL, normally `nats://127.0.0.1:4222`
- `JS_DOMAIN`: local JetStream domain, for example `edge-i69-mu1`
- `CFG_BUCKET`: JetStream KV bucket
- `CFG_KEY`: JetStream KV key for the LabJack config
- `CENTRAL_NATS_SERVERS`: optional central OATS NATS URLs for config sync
- `CENTRAL_NATS_CREDS_FILE`: optional central creds file, defaults to `NATS_CREDS_FILE`
- `CENTRAL_CFG_BUCKET`: optional central KV bucket to mirror from, defaults to `CFG_BUCKET`
- `CENTRAL_CFG_KEY`: optional central KV key to mirror from, defaults to `CFG_KEY`
- `CENTRAL_JS_DOMAIN`: optional central JetStream domain if central KV is domain-scoped
- `LABJACK_IP`: required direct LabJack IP for `streamer`
- `LABJACK_SERIAL`: optional but recommended post-connect serial verification
- `LABJACK_NAME`: optional logical device name for logging
- `STREAMER_MAX_LABJACK_FAILURES`: consecutive sampler failures before the streamer exits cleanly, default `5`
- `STREAMER_LABJACK_RETRY_DELAY_SECS`: delay between sampler retries, default `5`

If `CFG_NATS_SERVERS` or `CENTRAL_NATS_SERVERS` is set, `streamer` bootstraps
the local KV from central KV and keeps watching the central key for updates.
Central changes are mirrored into local KV, and the existing local KV watcher
then restarts the sampler with the new config.

`streamer` now uses a strict Ethernet IP path only:

- no subnet scan
- no indirect serial/name discovery
- no USB fallback

On connect, `streamer`:

- opens the T7 directly via `LABJACK_IP`
- verifies the connected handle is a T7
- verifies `LABJACK_SERIAL` if provided
- runs a minimal read/write self-test using `STREAM_SETTLING_US`

During runtime, `streamer` retries sampler failures up to
`STREAMER_MAX_LABJACK_FAILURES` consecutive times. After that it exits with a
successful status so a `Restart=on-failure` systemd unit does not keep hammering
the LabJack. Fix the hardware/network/config issue, then restart the service
manually.

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

## Browser Decode Note

The browser receives FlatBuffer payloads over the NATS WebSocket connection.
Some payloads arrive as `Uint8Array` slices whose `byteOffset` is not 8-byte
aligned. The generated TypeScript helper `valuesArray()` can throw on these
misaligned payloads when it tries to create a `Float64Array` view directly.

`webapp/src/lib/flatbuffer-parser.ts` now:

- tries the fast `valuesArray()` path first
- falls back to scalar `scan.values(i)` extraction when alignment is invalid

This keeps per-channel point counts stable in the plot UI even when the browser
receives misaligned WebSocket payload slices.

## LabJack KV Config

The JSON stored in JetStream KV should use the newer structure:

```json
{
  "labjack_name": "Macbook",
  "asset_number": 1456,
  "max_channels": 14,
  "site_id": "i69",
  "box_id": "i69-mu1",
  "source_type": "labjack",
  "source_id": "i69-lj2",
  "nats_subject": "avenars",
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

With the structured namespace, channel 11 from this config publishes to:

```text
avenars.i69.i69-mu1.i69-lj2.live.ch11
```

Older configs using `avenabox.<asset>.data.ch##` still parse and publish with
the legacy subject shape. New configs should use the structured `avenars`
fields above.

Legacy KV configs using `scan_rate` and `sampling_rate` are still accepted on
read, but new configs should use `scans_per_read` and `scan_rate_hz` so the
names match the actual LabJack stream semantics.

## Full Edge Setup

For the reproducible NATS leaf, Alloy, Rust service, webapp, validation, and
shutdown workflow, use [../docs/setup-guide.md](../docs/setup-guide.md).
