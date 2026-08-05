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

Read [Edge Runtime Services and Processes](runtime-services.md) alongside this
guide. It is the authoritative inventory of systemd units, container child
processes, ports, runtime files, and expected active/inactive states.

## Runtime Service Summary

Every deployed LattePanda is intended to run these project units:

- `nats-leaf.service`: Podman/Quadlet local NATS, JetStream, KV, and central
  leaf connection.
- `nats-exporter.service`: Podman/Quadlet Prometheus exporter for local NATS.
- `alloy.service`: Podman/Quadlet host, NATS, and Avena metric collector.
- `avena-streamer.service`: LabJack acquisition and NATS publisher.
- `avena-archiver.service`: JetStream consumer and safe Parquet writer.
- `avena-exporter.service`: central-NATS CSV export worker.
- `avena-health-metrics.timer`: 15-second scheduler for the Avena health
  textfile.
- `avena-health-metrics.service`: short-lived oneshot invoked by the timer. It
  is normally inactive between successful runs.
- `wattdog.service`: separately installed Podman/Quadlet Thornwave and relay
  process. Keep it in `--dry-run` until the physical power tests pass.

Required supporting host services are `NetworkManager`, `chronyd`, `sshd`,
`tailscaled`, and, for Wattdog, Bluetooth and D-Bus. Podman is daemonless:
`podman.service` is not expected to stay active. The long-running container
children appear as `conmon`, `nats-server`, `prometheus-nats-exporter`, `alloy`,
and `wattdog`; manage their systemd units instead of killing child processes.

The normal boot relationship is:

```text
network/time/remote access
  -> nats-leaf
       -> streamer + archiver + exporter + nats-exporter
  -> Alloy
  -> health timer -> health oneshot -> Alloy textfile scrape
Bluetooth + D-Bus -> Wattdog
```

## Subject And Key Design

Use one consistent structured namespace everywhere:

```text
<root>.<site_id>.<box_id>.<source_id>.<purpose>...
```

For this deployment, `root` is normally `avenars`.

Examples for site `i69`, box `i69-mu2`, and LabJack source `i69-lj2`:

```text
KV bucket:         avenabox
KV key:            i69.i69-mu2.i69-lj2.config
Live wildcard:     avenars.i69.i69-mu2.i69-lj2.live.*
Live channel:      avenars.i69.i69-mu2.i69-lj2.live.ch11
Export request:    avenars.i69.i69-mu2.i69-lj2.export.request
Export reply:      browser-generated NATS inbox
```

There is no `v1` token in the active design. If we need a future breaking
subject change, use a new root or a deliberate migration, not an extra token in
every current subject.

NATS KV has internal subjects under `$KV`, but use the `nats kv` commands for
normal operations:

```bash
nats kv get avenabox i69.i69-mu2.i69-lj2.config
```

Do not subscribe to `$KV...` directly unless you are debugging NATS internals.

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

Confirm the host services used for networking, time, and recovery:

```bash
systemctl is-active NetworkManager chronyd sshd tailscaled
timedatectl
```

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

## 3. Select The Box Profile

Do not edit one shared mutable configuration when moving between boxes. The
committed deployment profiles are:

```text
shared/edge-boxes/i69-mu1.json
shared/edge-boxes/i69-mu2.json
```

Each profile owns these identity values:

- `box_id`: the edge hostname, for example `i69-mu2`
- `source.id`: the LabJack source id, for example `i69-lj2`
- `nats.leaf_server_name`: normally `<box_id>-leaf`
- `nats.jetstream_domain`: normally `edge-<box_id>`
- `nats.kv_key`: normally `<site_id>.<box_id>.<source_id>.config`
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
    "rust_creds_file": "/etc/avena-rs/apt.creds",
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
    "kv_key": "i69.i69-mu2.i69-lj2.config",
    "stream_name": "labjacks",
    "stream_max_bytes": 100000000000
  }
}
```

Render a self-contained bundle for the selected box. This does not overwrite
the generated files for another profile:

```bash
cd /home/user/avena-rs
./shared/render-edge-config.py \
  --config shared/edge-boxes/i69-mu1.json \
  --output-dir target/edge-config/i69-mu1

./shared/render-edge-config.py \
  --config shared/edge-boxes/i69-mu2.json \
  --output-dir target/edge-config/i69-mu2
```

Validate the rendered values:

```bash
jq -r '.env.BOX_ID, .env.JS_DOMAIN, .env.CFG_KEY, .env.LABJACK_IP, .env.LABJACK_SERIAL' \
  target/edge-config/i69-mu1/streamer.env.json
jq -r '.env.BOX_ID, .env.JS_DOMAIN, .env.CFG_KEY, .env.LABJACK_IP, .env.LABJACK_SERIAL' \
  target/edge-config/i69-mu2/streamer.env.json
grep -nE 'server_name:|domain:' target/edge-config/i69-mu1/nats-leaf.conf
grep -nE 'server_name:|domain:' target/edge-config/i69-mu2/nats-leaf.conf
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

Install Quadlet files from the selected rendered bundle (MU1 is shown):

```bash
cd /home/user/avena-rs
sudo install -m 0644 target/edge-config/i69-mu1/nats-leaf.conf /etc/containers/systemd/nats-leaf.conf
sudo install -m 0644 shared/nats-leaf.container /etc/containers/systemd/nats-leaf.container
sudo install -m 0644 shared/nats-exporter.container /etc/containers/systemd/nats-exporter.container
sudo install -m 0644 shared/config.alloy /etc/containers/systemd/config.alloy
sudo install -m 0644 target/edge-config/i69-mu1/alloy.container /etc/containers/systemd/alloy.container
```

Start services:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now nats-leaf nats-exporter alloy
```

These names are generated systemd services created from Quadlet files during
`systemctl daemon-reload`. `systemctl is-enabled` can report `generated` rather
than `enabled`; that is normal when the Quadlet contains an `[Install]` section.
No persistent `podman.service` is required.

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
  kv put avenabox i69.i69-mu2.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv get avenabox i69.i69-mu2.i69-lj2.config --raw | jq .
```

Seed local KV too. This lets the edge start even if central is temporarily unavailable:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv add avenabox --history=5 || true
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 \
  kv put avenabox i69.i69-mu2.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 \
  kv get avenabox i69.i69-mu2.i69-lj2.config --raw | jq .
```

The underlying NATS KV subject is:

```text
$KV.avenabox.<site_id>.<box_id>.<source_id>.config
```

Use `nats kv` for normal access instead of subscribing to `$KV...` directly.

## 7. Install Persistent Rust Services

The installer renders the selected profile, builds release binaries, installs
runtime configuration under `/etc/avena-rs`, installs binaries under
`/usr/local/libexec/avena-rs`, and enables persistent system services. It also
installs the health-metric timer and Alloy textfile integration.

```bash
cd /home/user/avena-rs
./scripts/install-edge-services.sh \
  --profile shared/edge-boxes/i69-mu1.json \
  --start
```

Use the MU2 profile on MU2. Before using `--start`, seed and verify that box's
central and local KV key as described above.

The installed env files point Rust at the local leaf and central config sync:

```bash
sudo jq . /etc/avena-rs/streamer.env.json
sudo jq . /etc/avena-rs/archiver.env.json
sudo jq . /etc/avena-rs/exporter.env.json
```

Important env behavior:

- `NATS_SERVERS=nats://127.0.0.1:4222`: publish/consume through the local leaf
- `JS_DOMAIN=edge-<box_id>`: use local JetStream
- `CFG_NATS_SERVERS=nats://nats1.oats:4222,nats://nats2.oats:4222`: mirror central KV
- `CFG_KEY=<site_id>.<box_id>.<source_id>.config`: central and local KV key
- `EXPORTER_MODE=worker`: exporter receives requests through NATS and sends CSV chunks back through NATS
- `STREAMER_MAX_LABJACK_FAILURES=5`: after five consecutive sampler failures, `streamer` exits cleanly and systemd does not restart it
- `STREAMER_LABJACK_RETRY_DELAY_SECS=5`: delay between LabJack sampler retry attempts

## 8. Operate Rust Services

The installed services are system units and survive reboot:

```bash
sudo systemctl start avena-streamer avena-archiver avena-exporter
```

Check status:

```bash
systemctl status \
  nats-leaf nats-exporter alloy \
  avena-streamer avena-archiver avena-exporter \
  avena-health-metrics.timer
systemctl is-enabled avena-streamer avena-archiver avena-exporter
systemctl --failed
```

`avena-health-metrics.service` is a oneshot. Between timer runs, its correct
state is `inactive (dead)`, with a successful previous result:

```bash
systemctl status avena-health-metrics.timer avena-health-metrics.service
journalctl -u avena-health-metrics.service -n 20 --no-pager
cat /var/lib/avena-rs/metrics/avena.prom
```

Follow logs:

```bash
journalctl -u avena-archiver -f
journalctl -u avena-exporter -f
journalctl -u avena-streamer -f
```

Only start `streamer` when the LabJack is powered and reachable:

```bash
ping -c 3 192.168.1.111
```

Expected runtime flow:

- `streamer` mirrors central `avenabox:<site>.<box>.<source>.config` into local KV
- `streamer` watches local KV and restarts sampling when the local mirrored config changes
- `streamer` stops after five consecutive LabJack sampler failures; inspect logs and restart it manually after fixing the hardware/network issue
- live channel samples publish to subjects such as `avenars.i69.i69-mu2.i69-lj2.live.ch11`
- `archiver` consumes local JetStream and writes parquet under `rust-ljm/parquet`
- active archive parts end in `.parquet.inprogress` and become `.parquet` only after a successful close
- startup preserves incomplete or unreadable parts with a `.quarantined-<timestamp>-<index>` suffix
- `exporter` listens on `avenars.i69.i69-mu2.i69-lj2.export.request`
- `avena-health-metrics.timer` publishes service state, last stream/archive time, quarantine count, and leaf connection count through Alloy

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
i69.i69-mu1.i69-lj2.config
i69.i69-mu2.i69-lj2.config
```

For live plots, it subscribes through central NATS to subjects such as:

```text
avenars.i69.i69-mu1.i69-lj2.live.ch11
avenars.i69.i69-mu2.i69-lj2.live.ch11
```

Those are normal NATS subjects, not KV subjects. The leaf node forwards them from the edge box to central.

For CSV exports, the webapp publishes a request through central NATS to:

```text
avenars.i69.<box_id>.<source_id>.export.request
```

The edge `exporter` receives that request through the leaf connection and replies with chunked CSV frames to the webapp reply inbox.

## 10. Validation Checklist

After setup, validate this order.

1. Central-to-local config sync:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv put avenabox i69.i69-mu2.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"

nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 \
  kv get avenabox i69.i69-mu2.i69-lj2.config --raw | jq .
```

The local value should update after `streamer` or `archiver` is running.

2. Streamer restart on config change:

```bash
journalctl -u avena-streamer -f
```

Change channels in the webapp. The streamer log should show a KV update and a sampler restart with the new channel list.

3. Live stream through central:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  sub 'avenars.i69.i69-mu2.i69-lj2.live.ch11' --count=5

nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  sub 'avenars.i69.i69-mu2.i69-lj2.live.ch7' --count=5
```

If channels 7 and 11 are enabled at the same scan rate, batches should arrive on both subjects at the same cadence. The exact per-batch point count is the FlatBuffer `values` length for each received message.

4. Webapp plots:

Open the plot route for the correct config key:

```text
/labjacks/plots/1001?key=i69.i69-mu2.i69-lj2.config
```

The browser should subscribe to the central live subjects and render the enabled channels.

5. CSV export:

From the webapp export dialog, request a time range that includes closed parquet files. Watch the edge exporter logs:

```bash
journalctl -u avena-exporter -f
```

The exporter should log the request and the browser should receive a CSV download.

The same NATS worker path can be tested without the webapp:

```bash
cd /home/user/avena-rs
./scripts/request-nats-export.mjs \
  --subject avenars.i69.i69-mu1.i69-lj2.export.request \
  --asset 1001 \
  --channels 11,13 \
  --start 2026-08-05T15:25:45Z \
  --end 2026-08-05T15:35:45Z \
  --output target/evidence/mu1-10min.csv \
  --report target/evidence/mu1-10min.json
```

The report records the byte, row, chunk, and missing-channel counts.

## 11. Restart, Stop, And Disable

Restart only the Rust data services:

```bash
sudo systemctl restart avena-streamer
sudo systemctl restart avena-archiver avena-exporter
```

For an orderly data-stack shutdown, stop acquisition first, then flush the
archiver, then stop the exporter:

```bash
sudo systemctl stop avena-streamer
sudo systemctl stop avena-archiver
sudo systemctl stop avena-exporter
```

Stopping the archiver sends `SIGINT`, waits for every channel writer, and closes
the current Parquet parts before systemd reports the service stopped.

If `avena-streamer` is inactive after a LabJack fault, check why it stopped:

```bash
journalctl -u avena-streamer -n 120 --no-pager
```

Fix the LabJack power/network/config issue, then restart it manually:

```bash
sudo systemctl restart avena-streamer
```

Restart containers:

```bash
sudo systemctl restart nats-leaf nats-exporter alloy
```

Do not restart the local NATS leaf while the Rust services are actively writing
unless testing recovery. A full ordered restart is:

```bash
sudo systemctl stop avena-streamer
sudo systemctl stop avena-archiver avena-exporter
sudo systemctl restart nats-leaf
sudo systemctl restart nats-exporter alloy
sudo systemctl start avena-archiver avena-exporter
sudo systemctl start avena-streamer
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

Wattdog is operated separately from the data stack:

```bash
systemctl status bluetooth wattdog --no-pager
journalctl -u wattdog -n 60 --no-pager
```

Do not remove Wattdog `--dry-run` as part of normal software installation. That
change is gated by the documented wiring, voltage-threshold, power-cut, and
recovery tests in `todo.md`.

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
- subscribe from central to `avenars.<site_id>.<box_id>.<source_id>.live.chNN`

CSV export does not start:

- confirm `exporter` is running in worker mode
- confirm it listens on `avenars.<site_id>.<box_id>.<source_id>.export.request`
- confirm parquet files exist under `rust-ljm/parquet`

`avena-health-metrics.service` shows inactive:

- this is normal between timer invocations because it is `Type=oneshot`
- confirm `avena-health-metrics.timer` is `active (waiting)`
- use `systemctl --failed` to distinguish a failed invocation from normal
  inactivity
- check that `/var/lib/avena-rs/metrics/avena.prom` has a recent modification
  time

The service is active but no child process appears under the unit name:

- Quadlet units use `conmon` as the systemd main process
- use `systemctl status <unit>` or `podman ps`, not only `ps` by unit name
- `podman.service` being inactive is normal because Podman is daemonless
