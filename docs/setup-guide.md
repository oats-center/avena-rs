# Avena Edge Box Setup Guide

This guide brings up one edge box, such as `i69-mu1` or `i69-mu2`, so it can:

- run a local NATS leaf node on `127.0.0.1:4222`
- run local JetStream in a box-specific domain such as `edge-i69-mu2`
- connect the leaf to central OATS NATS at `nats1.oats:7422` and `nats2.oats:7422`
- run Alloy and a NATS exporter for host/NATS metrics
- run `streamer`, `archiver`, and `exporter` locally
- mirror central webapp KV config into local KV
- publish live LabJack samples from the local leaf to central NATS
- answer CSV export requests sent by the webapp through central NATS

The webapp always connects to central NATS. Laptops do not connect directly to edge boxes.

## 1. Install Host Dependencies

On the edge box:

```bash
sudo dnf install -y podman jq curl git gcc gcc-c++ make openssl-devel pkg-config
```

Install or verify the remaining tools from your normal internal source:

```bash
command -v systemctl
command -v podman
command -v curl
command -v jq
command -v nats
command -v nsc
command -v cargo
command -v rustc
```

Install the LabJack LJM runtime library. `streamer` needs `libLabJackM.so` in a normal library path such as `/usr/local/lib` or `/usr/lib`.

Confirm network and OATS DNS:

```bash
hostname
tailscale status --self
tailscale ip -4
getent hosts nats1.oats nats2.oats prometheus.oats
```

Do not continue until `nats1.oats`, `nats2.oats`, and `prometheus.oats` resolve.

## 2. Clone Or Update The Repo

Use the same path on every edge box unless there is a reason not to:

```bash
cd /home/user
git clone <your-avena-rs-remote> avena-rs
cd /home/user/avena-rs
```

If the repo already exists:

```bash
cd /home/user/avena-rs
git pull
```

## 3. Edit The Box Config

All generated edge config starts from:

```text
shared/edge-box.config.json
```

For each new box, edit these values:

- `box_id`: the edge hostname, for example `i69-mu2`
- `source.id`: the LabJack source id, for example `i69-lj2`
- `nats.leaf_server_name`: normally `<box_id>-leaf`
- `nats.jetstream_domain`: normally `edge-<box_id>`
- `nats.kv_key`: normally `v1.<box_id>.<source_id>.config`
- `labjack.name`: usually the same as `source.id`
- `labjack.asset_number`: the asset number shown in the webapp
- `labjack.ip`: the LabJack Ethernet IP
- `labjack.serial`: the expected LabJack serial number
- `labjack.sensor_settings.channels_enabled`: default enabled channels
- `paths.output_dir`: local streamer output directory
- `paths.parquet_dir`: local parquet archive directory

Recommended path values when `/home/user` is on the SSD:

```json
{
  "paths": {
    "rust_creds_file": "apt.creds",
    "output_dir": "/home/user/avena-rs/rust-ljm/outputs",
    "parquet_dir": "/home/user/avena-rs/rust-ljm/parquet",
    "exporter_http_url": "http://127.0.0.1:9001"
  }
}
```

Recommended NATS values for `i69-mu2` and LabJack `i69-lj2`:

```json
{
  "box_id": "i69-mu2",
  "source": {
    "type": "labjack",
    "id": "i69-lj2"
  },
  "nats": {
    "root_subject": "avenars",
    "local_servers": "nats://127.0.0.1:4222",
    "config_servers": "nats://nats1.oats:4222,nats://nats2.oats:4222",
    "config_jetstream_domain": "",
    "leaf_server_name": "i69-mu2-leaf",
    "leaf_listen": "127.0.0.1:4222",
    "monitor_listen": "127.0.0.1:8222",
    "jetstream_domain": "edge-i69-mu2",
    "jetstream_store_dir": "/var/lib/nats/jetstream",
    "leaf_credentials_path": "/etc/nats/creds/leaf.creds",
    "leaf_remotes": [
      "nats://nats1.oats:7422",
      "nats://nats2.oats:7422"
    ],
    "kv_bucket": "avenabox",
    "kv_key": "v1.i69-mu2.i69-lj2.config",
    "stream_name": "labjacks",
    "stream_max_bytes": 100000000000
  }
}
```

Render generated files:

```bash
cd /home/user/avena-rs
./shared/render-edge-config.py
```

Validate the rendered values:

```bash
jq . shared/edge-box.config.json
jq . shared/labjack-kv.generated.json
jq -r '.env.BOX_ID, .env.JS_DOMAIN, .env.CFG_KEY, .env.LABJACK_IP, .env.LABJACK_SERIAL' rust-ljm/streamer.env.json
grep -n 'server_name:' shared/nats-leaf.conf
grep -n 'domain:' shared/nats-leaf.conf
```

## 4. Prepare Local Storage And Credentials

Create local storage:

```bash
mkdir -p /home/user/nats
mkdir -p /home/user/avena-rs/rust-ljm/parquet
mkdir -p /home/user/avena-rs/rust-ljm/outputs
df -hT /
du -sh /home/user/nats /home/user/avena-rs/rust-ljm/parquet 2>/dev/null || true
```

Place the client creds used by Rust services at:

```text
rust-ljm/apt.creds
```

Place the leaf connection creds at:

```text
/etc/containers/systemd/creds/leaf.creds
```

Install the leaf creds:

```bash
sudo mkdir -p /etc/containers/systemd/creds
sudo install -m 0600 /path/to/leaf.creds /etc/containers/systemd/creds/leaf.creds
```

The leaf user on central should match the box, for example `i69-mu2-leaf`.

## 5. Install NATS Leaf, NATS Exporter, And Alloy

Install Quadlet files:

```bash
cd /home/user/avena-rs
sudo install -m 0644 shared/nats-leaf.conf /etc/containers/systemd/nats-leaf.conf
sudo install -m 0644 shared/nats-leaf.container /etc/containers/systemd/nats-leaf.container
sudo install -m 0644 shared/nats-exporter.container /etc/containers/systemd/nats-exporter.container
sudo install -m 0644 shared/config.alloy /etc/containers/systemd/config.alloy
sudo install -m 0644 shared/alloy.container /etc/containers/systemd/alloy.container
```

Start services:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now nats-leaf nats-exporter alloy
```

Validate local NATS:

```bash
systemctl is-active nats-leaf nats-exporter alloy
curl -fsS http://127.0.0.1:8222/varz | jq '{server_name, jetstream}'
curl -fsS http://127.0.0.1:8222/leafz | jq '{leafnodes, leafs}'
```

Expected:

- `server_name` is `<box_id>-leaf`, for example `i69-mu2-leaf`
- `jetstream` exists
- `leafnodes` is greater than `0`

Validate local and central NATS auth:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds rtt
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 account info
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds rtt
```

If local auth works but central auth reports `Authorization Violation`, fix the OATS account/user JWT on central.

## 6. Seed Central And Local KV

The webapp edits central KV. The edge Rust services mirror central KV into local KV, then run from local KV.

Create or update the central key:

```bash
cd /home/user/avena-rs
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds kv add avenabox --history=5 || true
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv put avenabox v1.i69-mu2.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv get avenabox v1.i69-mu2.i69-lj2.config --raw | jq .
```

Seed local KV too. This lets the edge start even if central is temporarily unavailable:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv add avenabox --history=5 || true
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 \
  kv put avenabox v1.i69-mu2.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 \
  kv get avenabox v1.i69-mu2.i69-lj2.config --raw | jq .
```

The underlying NATS KV subject is:

```text
$KV.avenabox.v1.<box_id>.<source_id>.config
```

Use `nats kv` for normal access instead of subscribing to `$KV...` directly.

## 7. Build Rust Binaries

Build the edge binaries:

```bash
cd /home/user/avena-rs/rust-ljm
cargo build --release
```

The generated env files already point Rust at the local leaf and at central config sync:

```bash
jq . streamer.env.json
jq . archiver.env.json
jq . exporter.env.json
```

Important env behavior:

- `NATS_SERVERS=nats://127.0.0.1:4222`: publish/consume through the local leaf
- `JS_DOMAIN=edge-<box_id>`: use local JetStream
- `CFG_NATS_SERVERS=nats://nats1.oats:4222,nats://nats2.oats:4222`: mirror central KV
- `CFG_KEY=v1.<box_id>.<source_id>.config`: central and local KV key
- `EXPORTER_MODE=worker`: exporter receives requests through NATS and sends CSV chunks back through NATS

## 8. Start Rust Services

Use the systemd user wrapper:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh archiver start
./avena-service.sh exporter start
./avena-service.sh streamer start
```

Check status:

```bash
./avena-service.sh archiver status
./avena-service.sh exporter status
./avena-service.sh streamer status
```

Follow logs:

```bash
./avena-service.sh archiver logs
./avena-service.sh exporter logs
./avena-service.sh streamer logs
```

Only start `streamer` when the LabJack is powered and reachable:

```bash
ping -c 3 192.168.1.111
```

Expected runtime flow:

- `streamer` mirrors central `avenabox:v1.<box>.<source>.config` into local KV
- `streamer` watches local KV and restarts sampling when the local mirrored config changes
- live channel samples publish to subjects such as `avenars.v1.i69-mu2.i69-lj2.ch11`
- `archiver` consumes local JetStream and writes parquet under `rust-ljm/parquet`
- `exporter` listens on `avenars.v1.i69.i69-mu2.archive.labjack.i69-lj2.export.request`

## 9. Laptop Webapp

On the laptop:

```bash
cd /path/to/avena-rs/webapp
corepack enable
pnpm install
pnpm dev --host 0.0.0.0
```

Open the URL printed by Vite, normally:

```text
http://127.0.0.1:5173
```

Connect the webapp to central OATS NATS:

```text
Server URL: ws://nats1.oats:8080
Credentials: a central NATS creds file with access to avenabox KV, live subjects, and export subjects
```

The webapp reads central KV keys such as:

```text
v1.i69-mu1.i69-lj2.config
v1.i69-mu2.i69-lj2.config
```

For live plots, it subscribes through central NATS to subjects such as:

```text
avenars.v1.i69-mu1.i69-lj2.ch11
avenars.v1.i69-mu2.i69-lj2.ch11
```

Those are normal NATS subjects, not KV subjects. The leaf node forwards them from the edge box to central.

For CSV exports, the webapp publishes a request through central NATS to:

```text
avenars.v1.i69.<box_id>.archive.labjack.<source_id>.export.request
```

The edge `exporter` receives that request through the leaf connection and replies with chunked CSV frames to the webapp reply inbox.

## 10. Validation Checklist

After setup, validate this order.

1. Central-to-local config sync:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv put avenabox v1.i69-mu2.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"

nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 \
  kv get avenabox v1.i69-mu2.i69-lj2.config --raw | jq .
```

The local value should update after `streamer` or `archiver` is running.

2. Streamer restart on config change:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh streamer logs
```

Change channels in the webapp. The streamer log should show a KV update and a sampler restart with the new channel list.

3. Live stream through central:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  sub 'avenars.v1.i69-mu2.i69-lj2.ch11' --count=5

nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  sub 'avenars.v1.i69-mu2.i69-lj2.ch7' --count=5
```

If channels 7 and 11 are enabled at the same scan rate, batches should arrive on both subjects at the same cadence. The exact per-batch point count is the FlatBuffer `values` length for each received message.

4. Webapp plots:

Open the plot route for the correct config key:

```text
/labjacks/plots/1001?key=v1.i69-mu2.i69-lj2.config
```

The browser should subscribe to the central live subjects and render the enabled channels.

5. CSV export:

From the webapp export dialog, request a time range that includes closed parquet files. Watch the edge exporter logs:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh exporter logs
```

The exporter should log the request and the browser should receive a CSV download.

## 11. Restart, Stop, And Disable

Restart Rust services:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh archiver restart
./avena-service.sh exporter restart
./avena-service.sh streamer restart
```

Stop Rust services:

```bash
./avena-service.sh streamer stop
./avena-service.sh archiver stop
./avena-service.sh exporter stop
```

These Rust services are transient user systemd units created with `systemd-run --user`; they do not install persistent boot units by default.

Restart containers:

```bash
sudo systemctl restart nats-leaf nats-exporter alloy
```

Stop containers until the next reboot or manual start:

```bash
sudo systemctl stop alloy nats-exporter nats-leaf
```

Disable container autostart on reboot:

```bash
sudo systemctl disable --now alloy nats-exporter nats-leaf
```

Prevent accidental manual or dependency starts:

```bash
sudo systemctl mask alloy nats-exporter nats-leaf
```

Undo the mask and start again:

```bash
sudo systemctl unmask nats-leaf nats-exporter alloy
sudo systemctl enable --now nats-leaf nats-exporter alloy
```

## 12. Troubleshooting

`curl http://127.0.0.1:8222/varz` fails:

- `nats-leaf` is not running
- check `sudo journalctl -u nats-leaf --no-pager -n 120`
- confirm `/home/user/nats` exists
- confirm `/etc/containers/systemd/creds/leaf.creds` exists

`leafz` shows no leaf connection:

- check OATS DNS
- check `nats1.oats:7422` and `nats2.oats:7422`
- check the central leaf user JWT and creds

Local `kv get` fails:

- check `--js-domain edge-<box_id>`
- check local leaf is running
- check the local KV bucket was created

Central-to-local config does not update:

- check `CFG_NATS_SERVERS` in `streamer.env.json` and `archiver.env.json`
- check central auth with `nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds rtt`
- watch `streamer` or `archiver` logs for central KV mirror errors

Live plots do not update:

- confirm the webapp is connected to central WebSocket NATS
- confirm the config key has the correct `box_id` and `source_id`
- subscribe from central to `avenars.v1.<box_id>.<source_id>.chNN`

CSV export does not start:

- confirm `exporter` is running in worker mode
- confirm it listens on `avenars.v1.<site>.<box_id>.archive.labjack.<source_id>.export.request`
- confirm parquet files exist under `rust-ljm/parquet`
