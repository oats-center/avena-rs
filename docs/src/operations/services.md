# Services on an edge node

This page lists what runs on a healthy edge node, what each piece depends on,
and where its files live. Use it to tell a real fault from normal behavior.

## Units

| Unit | Kind | Normal state | Restarts |
|---|---|---|---|
| `nats-leaf.service` | Podman container (`nats-server`) | active (running) | always |
| `nats-exporter.service` | Podman container (`prometheus-nats-exporter`) | active (running) | always |
| `alloy.service` | Podman container (Grafana Alloy) | active (running) | always |
| `avena-streamer.service` | Rust binary | active (running) | on failure, but see below |
| `avena-archiver.service` | Rust binary | active (running) | on failure |
| `avena-exporter.service` | Rust binary | active (running) | on failure |
| `avena-health-metrics.timer` | systemd timer, every 15 s | active (waiting) | |
| `avena-health-metrics.service` | shell script, oneshot | inactive between runs | |

The three Rust services need `nats-leaf` (`Requires=`), so stopping the NATS
server stops them too. All units start at boot.

Two states look like faults but are not:

- `avena-health-metrics.service` is **inactive** almost all the time. It is a
  oneshot that the timer runs every 15 seconds. Use `systemctl --failed` to see
  whether a run actually failed.
- The container units run their program under `conmon`, and `podman.service` is
  not running. Podman has no daemon; systemd starts each container directly.

The streamer is deliberately not restarted forever. After five LabJack failures
in a row (`STREAMER_MAX_LABJACK_FAILURES`) it exits with a success status, so
systemd leaves it stopped instead of hammering a device that is unplugged or
unpowered. Fix the LabJack, then start the service by hand.

Host services the stack relies on: `NetworkManager`, `chronyd` (timestamps are
only as good as the clock), `sshd` and `tailscaled` (remote access and the
`.oats` names).

Some boxes also run `wattdog.service`, which watches the battery and solar
system over Bluetooth. It is installed separately from this repository and is
not covered here.

## What each service does at run time

**nats-leaf** is the box's own NATS server. It listens on `127.0.0.1:4222` for
the local services, serves monitoring on `127.0.0.1:8222`, stores the JetStream
stream `labjacks` and the local `avenabox` bucket under `/home/user/nats`, and
keeps one outbound leaf connection to `nats1.oats:7422` or `nats2.oats:7422`.

**streamer** mirrors its configuration key from central `avenabox` into the
local bucket, opens the LabJack at `LABJACK_IP`, checks it is a T7 with the
expected serial, and streams the enabled channels. Each read becomes one
message per channel on `avenars.<site>.<box>.<source>.live.chNN`. A
configuration change stops the stream and starts it again with the new
settings. Every 60 seconds it compares its sample timeline with the system
clock and corrects drift of 5 ms or more, logging `[clock] Re-anchored ...`.

**archiver** has one durable JetStream consumer per enabled channel. It writes
samples into Parquet files that cover aligned five-minute windows (:00 to :05,
:05 to :10, and so on) and acknowledges the messages only once the file is
closed and synced. While a window is open its file exists only as an empty
`.parquet.inprogress` placeholder; the data is written when the window closes.
A file is also closed after 60 seconds without data, for example when the
streamer stops.

**exporter** listens on `avenars.<site>.<box>.<source>.export.request` and
answers each request with CSV read from the Parquet archive. It opens no TCP
port in the normal (worker) mode.

**Alloy** scrapes host metrics, the NATS exporter on port 7777 and the health
textfile, and sends them to `prometheus.oats:9090`.

**avena-health-metrics** writes `/var/lib/avena-rs/metrics/avena.prom` with the
state of each service, the time of the newest JetStream message and of the
newest completed Parquet file, the number of quarantined files, and the leaf
connection count.

## Files

| Path | Contents |
|---|---|
| `/etc/avena-rs/` | Installed profile, one `*.env.json` per service, and `apt.creds` |
| `/usr/local/libexec/avena-rs/` | Installed binaries and service scripts |
| `/etc/systemd/system/avena-*` | The Avena units |
| `/etc/containers/systemd/` | Quadlet container units, `nats-leaf.conf`, `config.alloy`, and `creds/leaf.creds` |
| `/home/user/nats/` | The local JetStream store. Never delete it while the box is in service. |
| `rust-ljm/parquet/asset<NNN>/<YYYY-MM-DD>/ch<NN>/` | The archive, one `part-NNNN.parquet` per window |
| `/var/lib/avena-rs/metrics/avena.prom` | Health metrics for Alloy |

The source checkout in `/home/user/avena-rs` is only used to build and
install. Its `target/` folders can be deleted at any time; the running
services do not use them.

## Ports

| Port | Where | Used by |
|---|---|---|
| 4222/tcp | 127.0.0.1 | local NATS clients |
| 8222/tcp | 127.0.0.1 | NATS monitoring |
| 7777/tcp | host | NATS Prometheus exporter, scraped by Alloy |
| 12345/tcp | 127.0.0.1 | Alloy status |
| 7422/tcp | outbound | leaf connection to central NATS |
| 4222/tcp | outbound | central NATS, for the configuration mirror |
| 9090/tcp | outbound | Prometheus remote write |

Only the NATS exporter binds to all interfaces. Keep it behind the host
firewall.

## One-command status

`scripts/edge-status.sh` (installed as `/home/user/avena-status`) prints the
state of every unit, clock synchronization, the local NATS server and leaf
count, the newest LabJack and camera files, disk space and failed units. It
exits with a non-zero status if any check fails.
