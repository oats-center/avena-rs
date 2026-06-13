# Edge Box Setup: NATS Leaf, Alloy, Rust Services, and Remote Webapp

This guide is for bringing up a new MU / edge box that:

- runs a local NATS leaf node with local JetStream
- connects that leaf to central OATS NATS
- stores JetStream and parquet data on `/extstore`
- runs `streamer`, `archiver`, and optional `exporter` locally
- sends host and NATS metrics to central Prometheus through Alloy
- is viewed from a laptop webapp connected to central NATS

The current repo example is `i69-mu1` with LabJack `i69-lj2`.

## Architecture

```text
LabJack T7
  -> streamer
  -> local NATS leaf + local JetStream
  -> archiver reads local JetStream and writes local parquet
  -> leaf forwards live subjects to central OATS NATS
  -> laptop webapp connects to central NATS over WebSocket
  -> exporter receives export requests over central NATS and streams CSV back

Alloy
  -> scrapes node-exporter-style host metrics
  -> scrapes local NATS exporter on 127.0.0.1:7777
  -> remote_write to http://prometheus.oats:9090/api/v1/write
```

Current subject and key layout:

```text
KV key:                  v1.<box_id>.<source_id>.config
Live subject wildcard:   avenars.v1.<box_id>.<source_id>.*
Live channel subject:    avenars.v1.<box_id>.<source_id>.ch11
Export request subject:  avenars.export.request.<box_id>
Export reply subject:    avenars.export.reply.<job_id>
```

For `i69-mu1` and `i69-lj2`:

```text
v1.i69-mu1.i69-lj2.config
avenars.v1.i69-mu1.i69-lj2.*
avenars.v1.i69-mu1.i69-lj2.ch11
```

## What Runs Where

On the edge box:

- `nats-leaf` Quadlet container
- `nats-exporter` Quadlet container
- `alloy` Quadlet container
- `rust-ljm/streamer`
- `rust-ljm/archiver`
- `rust-ljm/exporter` in `worker` mode

On the laptop:

- `webapp`
- browser session connected to `ws://nats1.oats:8080`

Not on central:

- no extra services need to be installed on central for this flow

## 1. Prerequisites

On the edge box, install or verify:

- `podman`
- `systemctl`
- `curl`
- `jq`
- `nats`
- `nsc`
- `cargo` and `rustc`
- LabJack LJM runtime library

Helpful checks:

```bash
command -v podman
command -v systemctl
command -v curl
command -v jq
command -v nats
command -v nsc
command -v cargo
command -v rustc
```

If you are on Fedora, the common base packages are:

```bash
sudo dnf install -y podman jq curl git gcc gcc-c++ make openssl-devel pkg-config
```

`nats`, `nsc`, the Rust toolchain, and the LabJack LJM library may come from your
normal internal install path rather than Fedora packages. `streamer` needs
`libLabJackM.so` available in a standard library path such as `/usr/local/lib`
or `/usr/lib`.

On the laptop, install or verify:

- `node`
- `pnpm` or `corepack`

Helpful checks:

```bash
command -v node
command -v pnpm || command -v corepack
```

## 2. Clone Repo and Confirm Network

On the edge box:

```bash
cd /extstore/home/user || cd /home/user
git clone <your-avena-rs-remote> avena-rs
cd avena-rs
```

Confirm Tailscale and OATS DNS:

```bash
hostname
tailscale status --self
tailscale ip -4
getent hosts nats1.oats nats2.oats prometheus.oats
```

Do not continue until `nats1.oats`, `nats2.oats`, and `prometheus.oats` resolve.

## 3. Edit the Box Config

All generated config comes from one file:

```text
shared/edge-box.config.json
```

Edit this first on every new box.

Current example:

```json
{
  "site_id": "i69",
  "box_id": "i69-mu1",
  "source": {
    "type": "labjack",
    "id": "i69-lj2"
  },
  "nats": {
    "root_subject": "avenars",
    "local_servers": "nats://127.0.0.1:4222",
    "leaf_server_name": "i69-mu1-leaf",
    "jetstream_domain": "edge-i69-mu1",
    "kv_bucket": "avenabox",
    "kv_key": "v1.i69-mu1.i69-lj2.config",
    "stream_name": "labjacks",
    "stream_max_bytes": 100000000000
  },
  "labjack": {
    "name": "i69-lj2",
    "asset_number": 1001,
    "ip": "192.168.1.111",
    "serial": "470036312",
    "sensor_settings": {
      "channels_enabled": [11, 13]
    }
  }
}
```

Fields that must be unique per box:

- `box_id`
- `source.id`
- `nats.leaf_server_name`
- `nats.jetstream_domain`
- `nats.kv_key`

Fields that must match the physical LabJack:

- `labjack.name`
- `labjack.asset_number`
- `labjack.ip`
- `labjack.serial`
- `labjack.sensor_settings.channels_enabled`

Recommended naming:

```text
box_id      = i69-mu1
source.id   = i69-lj2
kv_key      = v1.i69-mu1.i69-lj2.config
js_domain   = edge-i69-mu1
leaf user   = i69-mu1-leaf
```

## 4. Render Generated Files

From the repo root:

```bash
cd /extstore/home/user/avena-rs
./shared/render-edge-config.py
```

This writes:

```text
shared/nats-leaf.conf
shared/alloy.container
rust-ljm/streamer.env.json
rust-ljm/archiver.env.json
rust-ljm/exporter.env.json
shared/labjack-kv.generated.json
```

Validate:

```bash
jq . shared/edge-box.config.json
jq . shared/labjack-kv.generated.json
jq -r '.env.BOX_ID, .env.JS_DOMAIN, .env.CFG_KEY, .env.LABJACK_IP, .env.LABJACK_SERIAL' rust-ljm/streamer.env.json
grep -n 'server_name:' shared/nats-leaf.conf
grep -n 'domain:' shared/nats-leaf.conf
```

## 5. Prepare Local Storage

Create host storage locations used by the containers and Rust services:

```bash
sudo mkdir -p /extstore/nats
mkdir -p /extstore/home/user/avena-rs/rust-ljm/parquet
mkdir -p /extstore/home/user/avena-rs/rust-ljm/outputs
```

Check capacity:

```bash
df -hT /extstore
du -sh /extstore/nats 2>/dev/null || true
du -sh /extstore/home/user/avena-rs/rust-ljm/parquet 2>/dev/null || true
```

Current JetStream behavior from this repo:

- stream cap: `100000000000` bytes per stream
- discard policy: `old`
- effect: oldest JetStream messages are evicted when the stream reaches 100 GB

## 6. Get Leaf Credentials

You need one NATS creds file for the box leaf user, normally named:

```text
<box_id>-leaf.creds
```

There are two ways to get it.

If a central admin already gave you a creds file, use that and skip the NSC
generation commands.

If you are allowed to generate it from the material in `shared/`:

```bash
cd /extstore/home/user/avena-rs/shared
nsc add operator --url OATS.jwt --force
nsc env -o OATS
nsc import account --file avena-rs.jwt || true
nsc env -a avena-rs
ls *.nk
```

Pick the account signing seed for `avena-rs` and export it:

```bash
export ACCOUNT_SEED=./<avena-rs-account-signing-seed>.nk
export LEAF_USER=i69-mu1-leaf
```

Create or refresh the user and generate creds:

```bash
nsc add user --account avena-rs --name "$LEAF_USER" -K "$ACCOUNT_SEED" || true
nsc generate creds --account avena-rs --name "$LEAF_USER" --output-file /tmp/"$LEAF_USER".creds -K "$ACCOUNT_SEED"
```

Validate:

```bash
test -s /tmp/"$LEAF_USER".creds && echo "creds file exists"
grep -q "BEGIN NATS USER JWT" /tmp/"$LEAF_USER".creds && echo "creds file looks valid"
```

Install the creds in both places:

```bash
sudo install -d -m 0755 /etc/containers/systemd/creds
sudo install -m 0600 /tmp/"$LEAF_USER".creds /etc/containers/systemd/creds/leaf.creds
install -m 0600 /tmp/"$LEAF_USER".creds rust-ljm/apt.creds
```

## 7. Install and Start Local NATS Leaf

Install the Quadlet files:

```bash
sudo install -m 0644 shared/nats-leaf.conf /etc/containers/systemd/nats-leaf.conf
sudo install -m 0644 shared/nats-leaf.container /etc/containers/systemd/nats-leaf.container
sudo install -m 0644 shared/nats-exporter.container /etc/containers/systemd/nats-exporter.container
```

Start the local leaf:

```bash
sudo systemctl daemon-reload
sudo systemctl restart nats-leaf
```

Validate local NATS:

```bash
curl -fsS http://127.0.0.1:8222/varz | jq '{server_name, jetstream}'
curl -fsS http://127.0.0.1:8222/leafz | jq '{leafnodes, leafs}'
```

Expected:

- `server_name` matches `<box_id>-leaf`
- JetStream exists
- `leafnodes` is greater than `0`

Validate CLI access:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds rtt
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 account info
```

If local leaf auth works, confirm central auth also works:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds rtt
```

If this reports `Authorization Violation`, the OATS account resolver has not
loaded the account/user JWT yet. That is a central-side issue.

## 8. Create the Local KV Entry

Add the KV bucket locally if it does not exist:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 kv add avenabox --history=5 || true
```

Write the generated config locally:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  kv put avenabox v1.i69-mu1.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
```

Validate:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  kv get avenabox v1.i69-mu1.i69-lj2.config | sed -n '/^{/,$p' | jq .
```

If the webapp should discover this box through central OATS KV, also put the
same config into central:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv put avenabox v1.i69-mu1.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
```

That central key is what the webapp reads on `/labjacks`.

## 9. Install and Start Alloy and NATS Exporter

Install the remaining Quadlet files:

```bash
sudo install -m 0644 shared/alloy.container /etc/containers/systemd/alloy.container
sudo install -m 0644 shared/alloy.volume /etc/containers/systemd/alloy.volume
sudo install -m 0644 shared/config.alloy /etc/containers/systemd/config.alloy
```

Start them:

```bash
sudo systemctl daemon-reload
sudo systemctl restart nats-exporter
sudo systemctl restart alloy
```

Validate local metrics endpoints:

```bash
curl -fsS http://127.0.0.1:7777/metrics | head
curl -fsS http://127.0.0.1:12345/-/ready
```

What Alloy sends:

- host metrics from the unix exporter
- NATS metrics scraped from `127.0.0.1:7777`
- labels `job="avena-rs"` and `instance="<box_id>"`

Prometheus destination from the checked-in config:

```text
http://prometheus.oats:9090/api/v1/write
```

Expected Grafana usage:

- open the OATS dashboard URL
- choose datasource `efo1upqlm33swc`
- choose `job=avena-rs`
- choose node `i69-mu1` or your new `box_id`

## 10. Build and Start the Rust Services

Build once:

```bash
cd /extstore/home/user/avena-rs/rust-ljm
cargo build --release --bin streamer --bin archiver --bin exporter
```

Start the three local processes:

```bash
./streamerctl.sh start
./archiverctl.sh start
./exporterctl.sh start
```

Check status:

```bash
./streamerctl.sh status
./archiverctl.sh status
./exporterctl.sh status
```

Tail logs:

```bash
tail -f logs/streamer.log
tail -f logs/archiver.log
tail -f logs/exporter.log
```

What each one does:

- `streamer`: reads the LabJack and publishes `avenars.v1.<box_id>.<source_id>.chXX`
- `archiver`: reads local JetStream and writes parquet under `rust-ljm/parquet`
- `exporter`: listens on `avenars.export.request.<box_id>` and sends CSV back over NATS

## 11. Validate the End-to-End Data Flow

Verify live traffic locally:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  stream info labjacks --json | jq '{config:{subjects:.config.subjects,max_bytes:.config.max_bytes,discard:.config.discard},state:{messages:.state.messages,bytes:.state.bytes}}'
```

Verify live traffic centrally:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  sub 'avenars.v1.i69-mu1.i69-lj2.*' --count=4
```

Verify parquet is being written:

```bash
find rust-ljm/parquet -type f | tail -n 10
du -sh rust-ljm/parquet
```

Verify archiver consumers:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  consumer info labjacks archiver-i69-mu1-labjack-i69-lj2-11
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  consumer info labjacks archiver-i69-mu1-labjack-i69-lj2-13
```

## 12. Run the Webapp on a Laptop

On the laptop:

```bash
git clone <your-avena-rs-remote> avena-rs
cd avena-rs/webapp
corepack enable || true
pnpm install
pnpm run dev
```

Open:

```text
http://localhost:5173
```

Login values:

- Server URL: `ws://nats1.oats:8080`
- Credentials file: a NATS creds file with access to central `avenabox`, live subjects, and export subjects

The webapp reads configs from central KV. The current plot route form is:

```text
/labjacks/plots/1001?key=v1.i69-mu1.i69-lj2.config
```

Important behavior:

- the webapp does not connect directly to the leaf node
- live plots subscribe through central NATS
- export requests are published through central NATS to `avenars.export.request.<box_id>`
- the local exporter answers back through central NATS

## 13. Exact Commands You Will Usually Re-Run

On the edge box after config changes:

```bash
cd /extstore/home/user/avena-rs
./shared/render-edge-config.py
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  kv put avenabox v1.i69-mu1.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv put avenabox v1.i69-mu1.i69-lj2.config "$(cat shared/labjack-kv.generated.json)"
cd rust-ljm
./streamerctl.sh restart
./archiverctl.sh restart
./exporterctl.sh restart
```

On the laptop:

```bash
cd avena-rs/webapp
pnpm run dev
```

## 14. Troubleshooting

Leaf node is up but not connected:

```bash
curl -fsS http://127.0.0.1:8222/leafz | jq
sudo journalctl -u nats-leaf --no-pager -n 100
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds rtt
```

No configs in the webapp:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds kv ls
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds kv get avenabox v1.i69-mu1.i69-lj2.config
```

No live points in the webapp:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds sub 'avenars.v1.i69-mu1.i69-lj2.*' --count=4
tail -n 100 /extstore/home/user/avena-rs/rust-ljm/logs/streamer.log
```

No parquet output:

```bash
tail -n 100 /extstore/home/user/avena-rs/rust-ljm/logs/archiver.log
find /extstore/home/user/avena-rs/rust-ljm/parquet -type f | tail
```

Export download fails:

```bash
tail -n 100 /extstore/home/user/avena-rs/rust-ljm/logs/exporter.log
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds sub 'avenars.export.request.i69-mu1'
```

Check current storage usage:

```bash
du -sh /extstore/nats
du -sh /extstore/home/user/avena-rs/rust-ljm/parquet
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  stream info labjacks --json | jq '{messages:.state.messages,bytes:.state.bytes,max_bytes:.config.max_bytes}'
```
