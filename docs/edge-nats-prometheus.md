# i69-mu2 Edge Setup: NATS Leaf, JetStream, Prometheus

This is the complete setup guide for the `i69-mu2` box. Follow it from top to bottom on the box itself.

The goal is:

- local NATS leaf node on `127.0.0.1:4222`
- local JetStream domain `edge-i69-mu2`
- leaf connection from this box to OATS NATS at `nats1.oats:7422`, `nats2.oats:7422`, `nats3.oats:7422`
- Rust `streamer` publishes LabJack data to the local leaf node
- Rust `archiver` writes local Parquet files
- Alloy sends host and local NATS metrics to OATS Prometheus
- the web app connects to central OATS NATS, not directly to this box

For this deployment, do not run the Rust `exporter` on `i69-mu2`. The central exporter/archive path runs on OATS and serves the web app from central-side data access.

## Single Config File

Device-specific values live in one place:

```text
shared/edge-box.config.json
```

If hardware changes, edit that file first. Do not hand-edit generated files unless you are debugging.

Current values:

```text
site_id:        i69
box_id:         i69-mu2
source_type:    labjack
source_id:      i69-lj2
labjack_name:   i69-lj2
labjack_serial: 470036330
labjack_ip:     192.168.1.111
nats_root:      avenars
kv_bucket:      avenabox
kv_key:         labjackd.config.i69-mu2
js_domain:      edge-i69-mu2
local_nats:     nats://127.0.0.1:4222
```

Common hardware/config changes and where to edit them:

- Box name changed: edit `box_id`, `nats.leaf_server_name`, `nats.jetstream_domain`, and `nats.kv_key`.
- LabJack replaced: edit `source.id`, `labjack.name`, `labjack.serial`, and possibly `labjack.ip`.
- LabJack IP changed: edit `labjack.ip`.
- Asset number changed: edit `labjack.asset_number`.
- Enabled channels changed: edit `labjack.sensor_settings.channels_enabled`, plus matching `data_formats`, `measurement_units`, and `calibrations`.
- Sampling changed: edit `labjack.sensor_settings.scans_per_read` and `labjack.sensor_settings.scan_rate_hz`.
- OATS leaf URLs changed: edit `nats.leaf_remotes`.
- Local NATS or monitoring ports changed: edit `nats.local_servers`, `nats.leaf_listen`, or `nats.monitor_listen`.
- Parquet path changed: edit `paths.parquet_dir`.

After editing `shared/edge-box.config.json`, always render generated files:

```bash
cd /home/user/avena-rs
./shared/render-edge-config.py
```

The renderer writes:

```text
shared/nats-leaf.conf
shared/alloy.container
rust-ljm/streamer.env.json
rust-ljm/archiver.env.json
shared/labjack-kv.generated.json
```

Validate the rendered files:

```bash
jq . shared/edge-box.config.json
jq . shared/labjack-kv.generated.json
jq -r '.env.BOX_ID, .env.JS_DOMAIN, .env.CFG_KEY, .env.LABJACK_IP, .env.LABJACK_SERIAL' rust-ljm/streamer.env.json
grep -n 'server_name: "i69-mu2-leaf"' shared/nats-leaf.conf
grep -n 'domain: "edge-i69-mu2"' shared/nats-leaf.conf
```

Live LabJack channel subjects will look like:

```text
avenars.v1.i69.i69-mu2.live.labjack.i69-lj2.sample.ch11
avenars.v1.i69.i69-mu2.live.labjack.i69-lj2.sample.ch13
```

## What Runs On This Box

Run these on `i69-mu2`:

- `nats-leaf.service`: local NATS leaf node with JetStream
- `nats-exporter.service`: local NATS metrics exporter
- `alloy.service`: host/NATS metrics collector and remote writer
- `rust-ljm/archiver`: local Parquet writer
- `rust-ljm/streamer`: LabJack reader and NATS publisher

Do not run these on `i69-mu2` for this deployment:

- `rust-ljm/exporter`: central/OATS owns the export-to-web path
- `webapp`: the browser-facing app connects to central OATS NATS

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
getent hosts nats1.oats nats2.oats nats3.oats prometheus.oats
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
  shared/labjack-kv.generated.json
```

Validate:

```bash
test -s shared/nats-leaf.conf && echo "nats-leaf.conf rendered"
test -s rust-ljm/streamer.env.json && echo "streamer env rendered"
test -s rust-ljm/archiver.env.json && echo "archiver env rendered"
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
nsc add user --account avena-rs --name i69-mu2-leaf -K ./AA5JDYLJA24B5CUEZ6B2XPTCXKH2KLWYMJ2RYOQ5CUYSBFKDO5JKCDKF.nk || true
nsc generate creds --account avena-rs --name i69-mu2-leaf --output-file /tmp/i69-mu2-leaf.creds -K ./AA5JDYLJA24B5CUEZ6B2XPTCXKH2KLWYMJ2RYOQ5CUYSBFKDO5JKCDKF.nk
cd /home/user/avena-rs
```

Validate:

```bash
test -s /tmp/i69-mu2-leaf.creds && echo "creds file exists"
grep -q "BEGIN NATS USER JWT" /tmp/i69-mu2-leaf.creds && echo "creds file looks valid"
```

If this fails, the NSC account/operator setup is the issue.

If `nsc env -o OATS.jwt` appears anywhere in older notes, do not use it. `nsc env -o` expects an operator name already imported into the NSC store. `OATS.jwt` is a file, so import it with `nsc add operator --url OATS.jwt --force`, then select `OATS`.

## Step 2: Install Local Service Configs

Install the Quadlet files and credentials:

```bash
sudo install -d -m 0755 /etc/containers/systemd/avena-rs/creds
sudo install -m 0644 shared/nats-leaf.conf /etc/containers/systemd/avena-rs/nats-leaf.conf
sudo install -m 0644 shared/nats-leaf.container /etc/containers/systemd/avena-rs/nats-leaf.container
sudo install -m 0644 shared/nats-jetstream.volume /etc/containers/systemd/avena-rs/nats-jetstream.volume
sudo install -m 0644 shared/nats-exporter.container /etc/containers/systemd/avena-rs/nats-exporter.container
sudo install -m 0644 shared/alloy.container /etc/containers/systemd/avena-rs/alloy.container
sudo install -m 0644 shared/alloy.volume /etc/containers/systemd/avena-rs/alloy.volume
sudo install -m 0644 shared/config.alloy /etc/containers/systemd/avena-rs/config.alloy
sudo install -m 0600 /tmp/i69-mu2-leaf.creds /etc/containers/systemd/avena-rs/creds/leaf.creds
install -m 0600 /tmp/i69-mu2-leaf.creds rust-ljm/apt.creds
```

Validate files:

```bash
sudo test -s /etc/containers/systemd/avena-rs/nats-leaf.conf && echo "nats config installed"
sudo test -s /etc/containers/systemd/avena-rs/creds/leaf.creds && echo "leaf creds installed"
test -s rust-ljm/apt.creds && echo "rust creds installed"
```

Validate key config values:

```bash
sudo grep -n 'server_name: "i69-mu2-leaf"' /etc/containers/systemd/avena-rs/nats-leaf.conf
sudo grep -n 'domain: "edge-i69-mu2"' /etc/containers/systemd/avena-rs/nats-leaf.conf
sudo grep -n 'nats1.oats:7422' /etc/containers/systemd/avena-rs/nats-leaf.conf
```

If these fail, the wrong config was copied or edited.

## Step 3: Start Local NATS Leaf

Reload systemd and start only NATS first:

```bash
sudo systemctl daemon-reload
sudo systemctl start nats-leaf
```

Do not use `sudo systemctl enable --now nats-leaf` for these Quadlet units on this host. They are generated by Podman from `/etc/containers/systemd/avena-rs/*.container`, and `enable` can fail with `Unit ... is transient or generated`. The `[Install]` section in the Quadlet file is enough for the generator to create the boot-time wants link.

Validate local NATS monitor:

```bash
curl -fsS http://127.0.0.1:8222/varz | jq '{server_name, now, jetstream}'
```

Expected:

- `server_name` is `i69-mu2-leaf`
- `jetstream` exists

Validate leaf connection status:

```bash
curl -fsS http://127.0.0.1:8222/leafz | jq
```

Expected:

- at least one remote leaf connection to OATS
- if there are zero leaf connections, check Tailscale/DNS, the OATS leaf port, and the creds file

Check logs if needed:

```bash
sudo journalctl -u nats-leaf --no-pager -n 80
```

If `leafnodes` is `0`, check OATS port reachability:

```bash
for port in 4222 7422 8080; do
  for host in nats1.oats nats2.oats nats3.oats; do
    echo "== $host:$port =="
    timeout 3 bash -lc "cat < /dev/null > /dev/tcp/$host/$port" && echo open || echo closed
  done
done
```

Expected:

- `4222` open: central NATS client port is reachable
- `8080` open: central NATS WebSocket port is reachable
- `7422` open: central NATS leaf-node port is reachable

If `4222` and `8080` are open but `7422` is closed, the local box is healthy enough to reach OATS, but the OATS leaf-node listener/firewall is not reachable. Fix the OATS side before expecting central live data.

## Step 4: Validate Local NATS And JetStream With CLI

Run:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds rtt
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 account info
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 stream ls -a
```

Expected:

- `rtt` succeeds
- `account info` shows `Connected Server Name: i69-mu2-leaf`
- `account info` shows JetStream domain `edge-i69-mu2`
- `stream ls -a` succeeds, even if only KV/internal streams exist

Do not use `nats server info` as the normal validation command for this leaf user. It requires system-account privileges and can fail even when local NATS and JetStream are healthy.

If `rtt` fails, local NATS auth/connectivity is the issue.

If `rtt` works but `account info` or `stream ls -a` fails, JetStream domain/config is the issue.

## Step 5: Start Metrics

Start NATS exporter and Alloy:

```bash
sudo systemctl start nats-exporter alloy
```

Validate NATS exporter:

```bash
curl -fsS http://127.0.0.1:7777/metrics | head
curl -fsS http://127.0.0.1:7777/metrics | grep -E 'leaf|jetstream|varz' | head
```

If the first command prints metrics and then ends with `curl: (23) Failure writing output to destination`, that is harmless. It happens because `head` closes the pipe after the first few lines while `curl` still has more metrics to write.

Validate Alloy:

```bash
sudo systemctl status alloy --no-pager
sudo journalctl -u alloy --no-pager -n 80
```

Expected:

- `nats-exporter` metrics endpoint returns text
- Alloy is active
- Alloy logs do not repeatedly report remote-write failures

Validate the exact local NATS metric names:

```bash
curl -fsS http://127.0.0.1:7777/metrics | grep -E '^(gnatsd_healthz_status_value|gnatsd_leafz_conn_nodes_total|gnatsd_varz_connections|gnatsd_varz_jetstream_config_domain)'
```

Expected examples:

```text
gnatsd_healthz_status_value{server_id="http://127.0.0.1:8222",value="ok"} 1
gnatsd_leafz_conn_nodes_total{server_id="http://127.0.0.1:8222"} 0
gnatsd_varz_jetstream_config_domain{server_id="http://127.0.0.1:8222",value="edge-i69-mu2"} 1
```

In Grafana, use Explore with the Prometheus data source. The node-exporter dashboard shows host CPU, memory, disk, and network panels; it does not automatically show local NATS panels.

Useful Prometheus queries:

```promql
gnatsd_healthz_status_value{job="avena-rs", service="nats-leaf", instance="i69-mu2", value="ok"}
gnatsd_leafz_conn_nodes_total{job="avena-rs", service="nats-leaf", instance="i69-mu2"}
gnatsd_varz_jetstream_config_domain{job="avena-rs", service="nats-leaf", instance="i69-mu2", value="edge-i69-mu2"}
gnatsd_varz_connections{job="avena-rs", service="nats-leaf", instance="i69-mu2"}
gnatsd_varz_in_msgs{job="avena-rs", service="nats-leaf", instance="i69-mu2"}
gnatsd_varz_out_msgs{job="avena-rs", service="nats-leaf", instance="i69-mu2"}
```

The upstream leaf connection metric is:

```promql
gnatsd_leafz_conn_nodes_total{job="avena-rs", service="nats-leaf", instance="i69-mu2"}
```

For the current setup, this metric stays `0` while OATS port `7422` is unreachable. After OATS accepts the leaf connection, it should become greater than `0`.

If the `instance="i69-mu2"` queries do not return data, confirm the installed Alloy Quadlet has the rendered box label:

```bash
grep -n 'Environment=NODE_NAME=i69-mu2' shared/alloy.container
sudo grep -n 'Environment=NODE_NAME=i69-mu2' /etc/containers/systemd/avena-rs/alloy.container
```

If the installed file still uses `NODE_NAME=%H`, reinstall the rendered Alloy file and restart Alloy:

```bash
sudo install -m 0644 shared/alloy.container /etc/containers/systemd/avena-rs/alloy.container
sudo systemctl daemon-reload
sudo systemctl restart alloy
```

After that, local Prometheus labels should use `i69-mu2`. The old
`localhost.localdomain` option can still appear in Grafana for a while because
Prometheus keeps old samples inside the selected time range. To confirm whether
new samples are fixed, set Grafana to **Last 5 minutes** and query:

```promql
node_uname_info{job="avena-rs", instance="i69-mu2"}
```

If `localhost.localdomain` still appears after Alloy was restarted, check for an
old Alloy process or stale installed config:

```bash
systemctl is-active alloy
sudo grep -n 'Environment=NODE_NAME=' /etc/containers/systemd/avena-rs/alloy.container
pgrep -af 'grafana/alloy|alloy run' || true
```

Expected:

- Alloy is active
- installed `Environment=NODE_NAME=i69-mu2`
- only the systemd-managed Alloy process is running

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
```

Important values must be:

```text
NATS_SERVERS = nats://127.0.0.1:4222
JS_DOMAIN    = edge-i69-mu2
CFG_BUCKET   = avenabox
CFG_KEY      = labjackd.config.i69-mu2
BOX_ID       = i69-mu2
LABJACK_IP   = 192.168.1.111
LABJACK_SERIAL = 470036330
```

Validate with commands:

```bash
jq -r '.env.NATS_SERVERS, .env.JS_DOMAIN, .env.CFG_KEY, .env.BOX_ID, .env.LABJACK_IP, .env.LABJACK_SERIAL' rust-ljm/streamer.env.json
jq -r '.env.NATS_SERVERS, .env.JS_DOMAIN, .env.CFG_KEY, .env.BOX_ID' rust-ljm/archiver.env.json
```

## Step 7: Write The Current LabJack KV Config

The LabJack KV payload is generated from `shared/edge-box.config.json` at:

```text
shared/labjack-kv.generated.json
```

Validate it before writing to NATS:

```bash
jq . shared/labjack-kv.generated.json
```

Create the KV bucket if needed and write the config:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv add avenabox --history=5 || true
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv put avenabox labjackd.config.i69-mu2 "$(cat shared/labjack-kv.generated.json)"
```

Validate:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 kv get avenabox labjackd.config.i69-mu2 | sed -n '/^{/,$p' | jq .
```

If this fails, the local NATS/JetStream/KV setup is the issue. Do not start the Rust processes yet.

## Step 8: Start Archiver

Start archiver before streamer so local data is recorded as soon as samples begin:

Check that no old managed binaries are already running:

```bash
cd /home/user/avena-rs/rust-ljm
./streamerctl.sh status
./archiverctl.sh status
./exporterctl.sh status
pgrep -af '/home/user/avena-rs/rust-ljm/target/(debug|release)/(streamer|archiver|exporter)|target/(debug|release)/(streamer|archiver|exporter)' || true
```

Expected:

- streamer is stopped
- archiver is stopped
- exporter is stopped
- `pgrep` prints nothing

If `pgrep` shows old processes, stop them before continuing:

```bash
./streamerctl.sh stop
./archiverctl.sh stop
./exporterctl.sh stop
pgrep -af '/home/user/avena-rs/rust-ljm/target/(debug|release)/(streamer|archiver|exporter)|target/(debug|release)/(streamer|archiver|exporter)' || true
```

The `*ctl.sh` scripts use PID files and also search for the managed binary path. `stop` is expected to stop all matching managed PIDs, not just the first one.

```bash
cd /home/user/avena-rs/rust-ljm
./archiverctl.sh restart
./archiverctl.sh status
tail -n 80 logs/archiver.log
```

Expected:

- status says archiver is running
- log says it connected to NATS
- log says it loaded config for `labjackd.config.i69-mu2`
- log subscribes to subjects like `avenars.v1.i69.i69-mu2.live.labjack.i69-lj2.sample.ch11`

If archiver cannot load KV, go back to Step 7.

If archiver cannot connect to NATS, go back to Step 4.

## Step 9: Start Streamer

Only do this when the LabJack is powered and reachable at `192.168.1.111`.

Validate LabJack network first:

```bash
ping -c 3 192.168.1.111
```

Start streamer:

```bash
cd /home/user/avena-rs/rust-ljm
./streamerctl.sh restart
./streamerctl.sh status
tail -n 120 logs/streamer.log
```

Expected:

- status says streamer is running
- log says it connected to NATS
- log says it loaded `avenabox:labjackd.config.i69-mu2`
- log says it connected to LabJack serial `470036330`
- log says streaming started

If it fails before LabJack connection, check NATS/KV steps.

If it fails at LabJack connection, check `LABJACK_IP`, cabling, power, and serial.

## Step 10: Validate Live Data And Archive Data

Check the local stream exists:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 stream ls -a
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds --js-domain edge-i69-mu2 stream info labjacks
```

Watch a live subject locally:

```bash
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds sub 'avenars.v1.i69.i69-mu2.live.labjack.i69-lj2.sample.*'
```

You should see binary messages arriving. Press `Ctrl+C` to stop the subscription.

Check Parquet files:

```bash
find /home/user/avena-rs/rust-ljm/parquet -type f -name '*.parquet' | sort | tail
```

Expected:

- files under paths like `parquet/asset1001/YYYY-MM-DD/ch11/part-0001.parquet`

If live data arrives but no Parquet files appear, archiver is the issue.

If no live data arrives, streamer/LabJack/NATS publishing is the issue.

## Step 11: Validate Central Visibility

The web app connects to central OATS NATS. This box should make live messages available through its leaf connection.

From a machine or context that can connect to central OATS NATS with web/user credentials, subscribe to:

```text
avenars.v1.i69.i69-mu2.live.labjack.i69-lj2.sample.*
```

If local subscription works but central subscription does not:

- check `curl http://127.0.0.1:8222/leafz`
- check OATS leaf-node permissions for `i69-mu2-leaf`
- check central account import/export or user permissions

## Normal Bring-Online Command Set

Use this after setup is complete:

```bash
cd /home/user/avena-rs
sudo systemctl unmask nats-leaf nats-exporter alloy || true
sudo systemctl daemon-reload
sudo systemctl start nats-leaf nats-exporter alloy

cd /home/user/avena-rs/rust-ljm
./streamerctl.sh stop
./archiverctl.sh stop
./archiverctl.sh restart
./streamerctl.sh restart
./archiverctl.sh status
./streamerctl.sh status
```

Run the Rust `*ctl.sh start`/`restart` commands from a normal shell on the box. Some temporary automation or coding-agent command runners clean up background child processes when the command session ends, even though the control script writes PID files. A normal interactive shell or a dedicated systemd service does not have that problem.

Quick validation:

```bash
curl -fsS http://127.0.0.1:8222/leafz | jq
curl -fsS http://127.0.0.1:7777/metrics | head
tail -n 40 /home/user/avena-rs/rust-ljm/logs/streamer.log
tail -n 40 /home/user/avena-rs/rust-ljm/logs/archiver.log
find /home/user/avena-rs/rust-ljm/parquet -type f -name '*.parquet' | sort | tail
```

## Normal Turn-Offline Command Set

Use this before shutting the box down or moving it.

Stop LabJack publishing first:

```bash
cd /home/user/avena-rs/rust-ljm
./streamerctl.sh stop
```

Give archiver a few seconds to flush and close files:

```bash
sleep 5
./archiverctl.sh stop
```

Stop edge services:

```bash
sudo systemctl stop alloy nats-exporter nats-leaf
```

Validate nothing is running:

```bash
cd /home/user/avena-rs/rust-ljm
./streamerctl.sh status
./archiverctl.sh status
systemctl is-active nats-leaf nats-exporter alloy || true
pgrep -af '/home/user/avena-rs/rust-ljm/target/(debug|release)/(streamer|archiver|exporter)|target/(debug|release)/(streamer|archiver|exporter)' || true
```

Expected after offline:

- streamer is stopped
- archiver is stopped
- exporter is stopped
- systemd services are inactive
- `pgrep` prints nothing

Power off:

```bash
sudo systemctl poweroff
```

## Reboot Command Set

Use this when you want to reboot the box and have it come back online.

Stop Rust data processes cleanly first:

```bash
cd /home/user/avena-rs/rust-ljm
./streamerctl.sh stop
sleep 5
./archiverctl.sh stop
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
./archiverctl.sh start
./streamerctl.sh start
./archiverctl.sh status
./streamerctl.sh status
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
./archiverctl.sh start
./streamerctl.sh start
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
- check `nats1.oats:7422`, `nats2.oats:7422`, `nats3.oats:7422`

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

Metrics do not appear in OATS dashboard:

- check `curl http://127.0.0.1:7777/metrics`
- check `sudo journalctl -u alloy --no-pager -n 120`
- confirm Tailscale tag `tag:prom-node`

## Useful Log Commands

```bash
sudo journalctl -u nats-leaf -f
sudo journalctl -u nats-exporter -f
sudo journalctl -u alloy -f
tail -f /home/user/avena-rs/rust-ljm/logs/streamer.log
tail -f /home/user/avena-rs/rust-ljm/logs/archiver.log
```

## References

These are only here for maintainers. You should not need them to run this setup.

- NATS JetStream leaf nodes: https://docs.nats.io/running-a-nats-service/configuration/leafnodes/jetstream_leafnodes
- NATS leaf node config: https://docs.nats.io/running-a-nats-service/configuration/leafnodes/leafnode_conf
- NATS Prometheus exporter: https://github.com/nats-io/prometheus-nats-exporter
