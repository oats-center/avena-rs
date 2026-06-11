# Edge Box Setup: NATS Leaf, Alloy, Rust Services, and Remote Webapp

This guide is for bringing up a new MU / edge box that:

- runs a local NATS leaf node with local JetStream
- connects that leaf to central OATS NATS
- stores JetStream and parquet data on `/extstore`
- runs `streamer`, `archiver`, and optional `exporter` locally
- sends host and NATS metrics to central Prometheus through Alloy
- is viewed from a laptop webapp connected to central NATS

- local NATS leaf node on `127.0.0.1:4222`
- local JetStream domain `edge-i69-mu2`
- leaf connection from this box to OATS NATS at `nats1.oats:7422` and `nats2.oats:7422`
- Rust `streamer` publishes LabJack data to the local leaf node
- Rust `archiver` writes local Parquet files
- Rust `exporter` reads local Parquet files and streams requested exports back through the local leaf node to central OATS NATS
- Alloy sends host and local NATS metrics to OATS Prometheus
- the web app connects to central OATS NATS, not directly to this box

## Single Config File

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

- `nats-leaf.service`: local NATS leaf node with JetStream
- `nats-exporter.service`: local NATS metrics exporter
- `alloy.service`: host/NATS metrics collector and remote writer
- `rust-ljm/archiver`: local Parquet writer
- `rust-ljm/streamer`: LabJack reader and NATS publisher
- `rust-ljm/exporter`: local Parquet export server that receives central NATS export requests through the leaf node

Normally do not run this on `i69-mu2`:

- `webapp`: the browser-facing app connects to central OATS NATS and can be run from a laptop or central host

## Before You Start

Start from the repo root:

```bash
cd /home/user/avena-rs
```

Confirm the box identity:

```bash
hostname
tailscale status --self
tailscale ip -4
```

Expected:

- Tailscale name is `i69-mu2`
- Linux hostname may still be `localhost.localdomain`; that is not a blocker for this setup
- Tailscale is online
- the device has the `tag:prom-node` tag on the Tailscale admin side

Confirm OATS names resolve:

```bash
getent hosts nats1.oats nats2.oats prometheus.oats
```

If this fails, Tailscale/DNS is the problem. Do not continue until OATS names resolve.

Check required local tools:

```bash
command -v podman
command -v systemctl
command -v curl
command -v nsc
command -v nats
command -v jq
```

If only `jq` is missing, install it or omit the `| jq ...` parts in validation commands. If `nsc` or `nats` is missing, install the NATS tools before continuing.

## Storage Check

Parquet archives, local JetStream storage, Podman container volumes, and repo data
all live under `/` for this deployment. On `i69-mu2`, `/` should be backed by
the 1 TB NVMe SSD, not the small eMMC device.

Check disks and mounted filesystems:

```bash
lsblk -o NAME,MODEL,SERIAL,SIZE,TYPE,FSTYPE,LABEL,UUID,MOUNTPOINTS
df -hT /
```

Expected on `i69-mu2` after storage expansion:

```text
nvme0n1                     CT1000P310SSD2  931.5G disk
fedora_fw--77--231-root                    928.9G lvm  xfs  /
/dev/mapper/fedora_fw--77--231-root xfs    929G        /
```

If `df -hT /` shows only about `115G`, the SSD is present but most of the LVM
space has not been allocated to `/`. Confirm available LVM space:

```bash
sudo pvs
sudo vgs
sudo lvs
```

If the `fedora_fw-77-231` volume group shows a large `VFree` value, grow `/` to
use the rest of the SSD:

```bash
sudo lvextend -r -l +100%FREE /dev/fedora_fw-77-231/root
```

Validate:

```bash
df -hT /
lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS
```

Expected result:

- `/` is about `929G`
- `/home/user/avena-rs/rust-ljm/parquet` is on `/`
- local NATS JetStream storage is on `/` through the Podman volume
- free space is hundreds of GB, not tens of GB

Check current Parquet usage:

```bash
du -sh /home/user/avena-rs/rust-ljm/parquet
```

## Step 0: Render The Config Files

Render all generated configs from `shared/edge-box.config.json`:

```bash
cd /home/user/avena-rs
./shared/render-edge-config.py
```

Expected output:

```text
Rendered edge config files:
  shared/nats-leaf.conf
  shared/alloy.container
  rust-ljm/streamer.env.json
  rust-ljm/archiver.env.json
  rust-ljm/exporter.env.json
  shared/labjack-kv.generated.json
```

Validate:

```bash
test -s shared/nats-leaf.conf && echo "nats-leaf.conf rendered"
test -s rust-ljm/streamer.env.json && echo "streamer env rendered"
test -s rust-ljm/archiver.env.json && echo "archiver env rendered"
test -s rust-ljm/exporter.env.json && echo "exporter env rendered"
test -s shared/labjack-kv.generated.json && echo "KV config rendered"
```

If this fails, fix `shared/edge-box.config.json` before continuing.

## Step 1: Create Or Locate Box Credentials

The repo includes the OATS operator/account files under `shared/`.

Create or refresh the `i69-mu2-leaf` user:

```bash
cd /home/user/avena-rs/shared
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

If old `localhost.localdomain` samples must be deleted immediately from OATS
Prometheus, that has to be done on the OATS Prometheus server with admin API
access. Run this on OATS only if Prometheus was started with
`--web.enable-admin-api`:

```bash
curl -X POST 'http://127.0.0.1:9090/api/v1/admin/tsdb/delete_series?match[]={job="avena-rs",instance="localhost.localdomain"}'
curl -X POST 'http://127.0.0.1:9090/api/v1/admin/tsdb/delete_series?match[]={job="avena-rs",server="localhost.localdomain"}'
curl -X POST 'http://127.0.0.1:9090/api/v1/admin/tsdb/delete_series?match[]={job="avena-rs",nodename="localhost.localdomain"}'
curl -X POST 'http://127.0.0.1:9090/api/v1/admin/tsdb/clean_tombstones'
```

Most of the time, waiting for the dashboard time window to move past the old
samples is enough.

If metrics do not show up in the OATS dashboard after a few minutes, check:

```bash
tailscale status --self
getent hosts prometheus.oats
sudo journalctl -u alloy --no-pager -n 120
```

## Step 6: Confirm Rust Env Files

Validate the edge process configs:

```bash
jq . rust-ljm/streamer.env.json
jq . rust-ljm/archiver.env.json
jq . rust-ljm/exporter.env.json
```

Important values must be:

```text
NATS_SERVERS = nats://127.0.0.1:4222
JS_DOMAIN    = edge-i69-mu2
CFG_NATS_SERVERS = nats://nats1.oats:4222,nats://nats2.oats:4222
CFG_JS_DOMAIN    =
CFG_BUCKET   = avenabox
CFG_KEY      = labjackd.config.i69-mu2
BOX_ID       = i69-mu2
LABJACK_IP   = 192.168.1.111
LABJACK_SERIAL = 470036330
```

Validate with commands:

```bash
jq -r '.env.NATS_SERVERS, .env.JS_DOMAIN, .env.CFG_NATS_SERVERS, .env.CFG_JS_DOMAIN, .env.CFG_KEY, .env.BOX_ID, .env.LABJACK_IP, .env.LABJACK_SERIAL' rust-ljm/streamer.env.json
jq -r '.env.NATS_SERVERS, .env.JS_DOMAIN, .env.CFG_NATS_SERVERS, .env.CFG_JS_DOMAIN, .env.CFG_KEY, .env.BOX_ID' rust-ljm/archiver.env.json
jq -r '.env.NATS_SERVERS, .env.JS_DOMAIN, .env.BOX_ID, .env.PARQUET_DIR, .env.EXPORTER_ADDR' rust-ljm/exporter.env.json
```

## Step 7: Write The Current LabJack KV Config

The LabJack KV payload is generated from `shared/edge-box.config.json` at:

```text
shared/labjack-kv.generated.json
```

Validate it before writing to NATS:

```bash
cd /extstore/home/user/avena-rs/rust-ljm
cargo build --release --bin streamer --bin archiver --bin exporter
```

The web app writes LabJack configuration to the central OATS KV bucket. The streamer and archiver read that central KV via `CFG_NATS_SERVERS`, while sample data stays local on the leaf node via `NATS_SERVERS`.

Create the central KV bucket if needed and write the config:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds kv add avenabox --history=5 || true
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds kv put avenabox labjackd.config.i69-mu2 "$(cat shared/labjack-kv.generated.json)"
```

Validate the central config that the streamer and web app will use:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds kv get avenabox labjackd.config.i69-mu2 --raw | jq .
```

Optional fallback: seed the local edge KV bucket too. This is useful if you later run a local-only test with `CFG_NATS_SERVERS` unset.

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv add avenabox --history=5 || true
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv put avenabox labjackd.config.i69-mu2 "$(cat shared/labjack-kv.generated.json)"
```

Validate:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv get avenabox labjackd.config.i69-mu2 | sed -n '/^{/,$p' | jq .
```

If the central command fails, the OATS NATS credentials/KV setup is the issue. If the local command fails, the local NATS/JetStream/KV setup is the issue. Do not start the Rust processes until the relevant KV read succeeds.

## Step 8: Build Rust Services

Build the three Rust binaries used on the edge box:

```bash
cd /home/user/avena-rs
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc RUSTFLAGS='' \
  cargo build --manifest-path rust-ljm/Cargo.toml --release \
  --bin streamer --bin archiver --bin exporter
```

Validate:

```bash
test -x rust-ljm/target/release/streamer && echo "streamer built"
test -x rust-ljm/target/release/archiver && echo "archiver built"
test -x rust-ljm/target/release/exporter && echo "exporter built"
```

## Step 9: Start Rust Services With User Systemd

Use `avena-service.sh` when you want systemd to own the Rust process lifecycle instead of the older PID-file `streamerctl.sh`/`archiverctl.sh` scripts.

Check status first:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh streamer status
./avena-service.sh archiver status
./avena-service.sh exporter status
pgrep -af '/home/user/avena-rs/rust-ljm/target/release/(streamer|archiver|exporter)' || true
```

If any older PID-file process is running, stop it before using the systemd path:

```bash
./streamerctl.sh stop
./archiverctl.sh stop
./exporterctl.sh stop
```

Start archiver before streamer so local data is recorded as soon as samples begin. Start exporter before opening the web-app download flow so it can answer central export requests.

```bash
./avena-service.sh archiver restart
./avena-service.sh exporter restart
```

Only start streamer when the LabJack is powered and reachable at `192.168.1.111`.

Validate LabJack network first:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  sub 'avenars.v1.i69-mu1.i69-lj2.*' --count=4
```

Verify parquet is being written:

```bash
./avena-service.sh streamer restart
```

Check status and logs:

```bash
./avena-service.sh archiver status
./avena-service.sh exporter status
./avena-service.sh streamer status
journalctl --user -u avena-archiver.service --no-pager -n 80
journalctl --user -u avena-exporter.service --no-pager -n 80
journalctl --user -u avena-streamer.service --no-pager -n 120
```

Expected:

- `avena-archiver.service` is active
- `avena-exporter.service` is active
- `avena-streamer.service` is active after you start it
- archiver says it loaded config for `labjackd.config.i69-mu2`
- archiver subscribes to subjects like `avenars.v1.i69.i69-mu2.live.labjack.i69-lj2.sample.ch11`
- exporter listens on `avenars.v1.i69.i69-mu2.archive.labjack.i69-lj2.export.request`
- streamer says it connected to LabJack serial `470036330`
- streamer says streaming started

If it fails before LabJack connection, check NATS/KV steps.

If it fails at LabJack connection, check `LABJACK_IP`, cabling, power, and serial.

## Step 10: Validate Live Data And Archive Data

Check the local stream exists:

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

## Step 12: Validate Archive Export Path

Archive download is separate from live plotting:

- archiver subscribes to local sample subjects on `127.0.0.1:4222`
- archiver writes Parquet under `/home/user/avena-rs/rust-ljm/parquet`
- exporter subscribes locally to `avenars.v1.i69.i69-mu2.archive.labjack.i69-lj2.export.request`
- the web app sends an export request to central OATS NATS
- the leaf connection routes that request down to the edge exporter
- exporter reads local Parquet and sends CSV chunks back to the web app through NATS replies

Check that exporter is listening:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh exporter status
journalctl --user -u avena-exporter.service --no-pager -n 80
```

Expected log line:

```text
[exporter] Listening for NATS exports on avenars.v1.i69.i69-mu2.archive.labjack.i69-lj2.export.request
```

Check that closed Parquet files exist:

```bash
find /home/user/avena-rs/rust-ljm/parquet/asset1001 -type f -name '*.parquet' -printf '%TY-%Tm-%Td %TH:%TM:%TS %s %p\n' | sort | tail
```

Expected:

- some files have non-zero size
- the newest currently-open file may not be readable until archiver rotates and closes it
- closed files are the ones exporter can reliably read

From the web app, open a LabJack plot page, open the download/export dialog, choose a time range that includes closed Parquet files, and start the download. While testing, watch exporter logs:

```bash
journalctl --user -u avena-exporter.service -f
```

Large CSV exports can be hundreds of MB because each sample row is expanded to text. The web app should continue as long as NATS data is still arriving; its timeout is an idle timeout, not a fixed maximum download duration.

If the browser starts downloading bytes but never finishes, restart exporter after rebuilding and try a smaller time range first:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh exporter restart
```

If exporter logs `failed to create reader`, the file is probably still active or was interrupted before close. Wait for the next archiver rotation or stop streamer, wait five seconds, then stop archiver so it can close files.

If exporter logs `Cannot access Str as Long`, that file is from an older Parquet schema. Use newer files written by this setup.

## Normal Bring-Online Command Set

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
cd /home/user/avena-rs
sudo systemctl unmask nats-leaf nats-exporter alloy || true
sudo systemctl daemon-reload
sudo systemctl start nats-leaf nats-exporter alloy

cd /home/user/avena-rs/rust-ljm
./avena-service.sh archiver restart
./avena-service.sh exporter restart
./avena-service.sh streamer restart
./avena-service.sh archiver status
./avena-service.sh exporter status
./avena-service.sh streamer status
```

`avena-service.sh` uses user systemd units named `avena-archiver.service`, `avena-exporter.service`, and `avena-streamer.service`. This avoids stray background PIDs and makes logs available through `journalctl --user`.

```bash
cd avena-rs/webapp
pnpm run dev
```

## 14. Troubleshooting

Leaf node is up but not connected:

```bash
curl -fsS http://127.0.0.1:8222/leafz | jq
curl -fsS http://127.0.0.1:7777/metrics | head
tail -n 40 /home/user/avena-rs/rust-ljm/logs/streamer.log
tail -n 40 /home/user/avena-rs/rust-ljm/logs/archiver.log
journalctl --user -u avena-exporter.service --no-pager -n 40
find /home/user/avena-rs/rust-ljm/parquet -type f -name '*.parquet' | sort | tail
```

No configs in the webapp:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh streamer stop
```

No live points in the webapp:

```bash
sleep 5
./avena-service.sh archiver stop
./avena-service.sh exporter stop
```

No parquet output:

```bash
tail -n 100 /extstore/home/user/avena-rs/rust-ljm/logs/archiver.log
find /extstore/home/user/avena-rs/rust-ljm/parquet -type f | tail
```

Export download fails:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh streamer status
./avena-service.sh archiver status
./avena-service.sh exporter status
systemctl is-active nats-leaf nats-exporter alloy || true
pgrep -af '/home/user/avena-rs/rust-ljm/target/(debug|release)/(streamer|archiver|exporter)|target/(debug|release)/(streamer|archiver|exporter)' || true
```

Check current storage usage:

```bash
du -sh /extstore/nats
du -sh /extstore/home/user/avena-rs/rust-ljm/parquet
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu1 \
  stream info labjacks --json | jq '{messages:.state.messages,bytes:.state.bytes,max_bytes:.config.max_bytes}'
```

## Reboot Command Set

Use this when you want to reboot the box and have it come back online.

Stop Rust data processes cleanly first:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh streamer stop
sleep 5
./avena-service.sh archiver stop
./avena-service.sh exporter stop
```

Leave the Quadlet services unmasked so they can come back after reboot:

```bash
sudo systemctl unmask nats-leaf nats-exporter alloy || true
sudo systemctl reboot
```

After the reboot, validate:

```bash
systemctl is-active nats-leaf nats-exporter alloy || true
curl -fsS http://127.0.0.1:8222/varz | jq '{server_name, jetstream}'
curl -fsS http://127.0.0.1:8222/leafz | jq
```

Then start the Rust data path:

```bash
cd /home/user/avena-rs/rust-ljm
./avena-service.sh archiver start
./avena-service.sh exporter start
./avena-service.sh streamer start
./avena-service.sh archiver status
./avena-service.sh exporter status
./avena-service.sh streamer status
```

If you want the box to stay offline after the next boot, mask the generated services before powering off:

```bash
sudo systemctl mask alloy nats-exporter nats-leaf
sudo systemctl poweroff
```

To bring a masked box back online later:

```bash
sudo systemctl unmask nats-leaf nats-exporter alloy
sudo systemctl daemon-reload
sudo systemctl start nats-leaf nats-exporter alloy
```

## After Moving The Box

When the box arrives somewhere else:

```bash
sudo systemctl unmask nats-leaf nats-exporter alloy || true
sudo systemctl daemon-reload
sudo systemctl start nats-leaf nats-exporter alloy
cd /home/user/avena-rs/rust-ljm
./avena-service.sh archiver start
./avena-service.sh exporter start
./avena-service.sh streamer start
```

Then run the quick validation commands from the bring-online section.

## Troubleshooting By Symptom

`getent hosts nats1.oats` fails:

- Tailscale/DNS is not ready
- check `tailscale status --self`

`curl http://127.0.0.1:8222/varz` fails:

- local NATS is not running
- check `sudo journalctl -u nats-leaf --no-pager -n 120`

`leafz` shows no OATS connection:

- OATS leaf port/creds/permissions are the issue
- check `/etc/containers/systemd/avena-rs/creds/leaf.creds`
- check `nats1.oats:7422` and `nats2.oats:7422`

`nats stream ls --js-domain edge-i69-mu2` fails:

- JetStream domain/config is the issue
- check `domain: "edge-i69-mu2"` in `nats-leaf.conf`

`kv get avenabox labjackd.config.i69-mu2` fails:

- KV config is missing
- rerun Step 7

`streamer` cannot connect to LabJack:

- check `ping -c 3 192.168.1.111`
- check LabJack power/network
- check serial `470036330`

`archiver` runs but no Parquet files appear:

- confirm streamer is publishing
- confirm archiver subscribed to `avenars.v1.i69.i69-mu2.live.labjack.i69-lj2.sample.*`

Web app archive download fails:

- confirm `avena-exporter.service` is running
- confirm exporter is listening on `avenars.v1.i69.i69-mu2.archive.labjack.i69-lj2.export.request`
- confirm Parquet files exist and have non-zero size
- use a time range that includes closed Parquet files, not only the currently-open file
- check `journalctl --user -u avena-exporter.service --no-pager -n 120`

Metrics do not appear in OATS dashboard:

- check `curl http://127.0.0.1:7777/metrics`
- check `sudo journalctl -u alloy --no-pager -n 120`
- confirm Tailscale tag `tag:prom-node`

## Useful Log Commands

```bash
sudo journalctl -u nats-leaf -f
sudo journalctl -u nats-exporter -f
sudo journalctl -u alloy -f
journalctl --user -u avena-streamer.service -f
journalctl --user -u avena-archiver.service -f
journalctl --user -u avena-exporter.service -f
```

## References

These are only here for maintainers. You should not need them to run this setup.

- NATS JetStream leaf nodes: https://docs.nats.io/running-a-nats-service/configuration/leafnodes/jetstream_leafnodes
- NATS leaf node config: https://docs.nats.io/running-a-nats-service/configuration/leafnodes/leafnode_conf
- NATS Prometheus exporter: https://github.com/nats-io/prometheus-nats-exporter
