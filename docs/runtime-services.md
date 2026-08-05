# Edge Runtime Services and Processes

This is the process inventory for each I-69 LattePanda edge box. It covers the
project services, the host services they depend on, the child processes created
by Podman, and the tools that should run only when an operator invokes them.

The webapp, central NATS cluster, and central Prometheus are not edge-box
processes. They are listed separately so it is clear where each component runs.

## Expected Boot Flow

```text
NetworkManager + chronyd + sshd + tailscaled
                     |
                     +--> nats-leaf.service (Podman: nats-server)
                     |          |
                     |          +--> nats-exporter.service
                     |          +--> avena-streamer.service
                     |          +--> avena-archiver.service
                     |          +--> avena-exporter.service
                     |
                     +--> alloy.service
                     |          ^
                     |          |
                     |    /var/lib/avena-rs/metrics/avena.prom
                     |          |
                     +--> avena-health-metrics.timer
                     |          +--> avena-health-metrics.service (oneshot)
                     |
Bluetooth + D-Bus ----+--> wattdog.service (dry-run until power tests pass)
```

`network-online.target` is a systemd synchronization target, not a process.
The Rust units require the local NATS leaf. The LabJack has no host daemon;
`streamer` talks directly to its Ethernet address through `libLabJackM.so`.

## MU1 Snapshot on 2026-08-05

The remote validation found these units running on `i69-mu1`:

```text
NetworkManager.service
chronyd.service
sshd.service
tailscaled.service
dbus-broker.service
nats-leaf.service
nats-exporter.service
alloy.service
wattdog.service                 (--dry-run)
avena-streamer.service
avena-archiver.service
avena-exporter.service
avena-health-metrics.timer      (active/waiting)
```

`avena-health-metrics.service` was normally inactive between successful timer
runs. `bluetooth.service` was enabled but inactive, so Wattdog remained unable
to receive Thornwave advertisements. The final field deployment intends
Bluetooth to be active and Wattdog to receive telemetry, but Wattdog must stay
in dry-run until the lab power tests are complete.

## Project Systemd Units

### `nats-leaf.service`

- Source: `/etc/containers/systemd/nats-leaf.container`, generated from
  `shared/nats-leaf.container`.
- Runtime: Podman container `avena-nats-leaf`, running `nats-server`.
- Purpose: local NATS server, box-local JetStream, KV, and outbound leaf
  connection to central OATS NATS.
- Expected state: `active (running)` whenever the edge stack is operating.
- Restart behavior: always restart after failure.
- Local endpoints: NATS client `127.0.0.1:4222` and monitor
  `127.0.0.1:8222`.
- Outbound endpoints: `nats1.oats:7422` and `nats2.oats:7422`.
- Persistent data: the host directory mounted at `/var/lib/nats` inside the
  container.

### `nats-exporter.service`

- Source: `/etc/containers/systemd/nats-exporter.container`, generated from
  `shared/nats-exporter.container`.
- Runtime: Podman container `avena-nats-exporter`, running
  `prometheus-nats-exporter`.
- Purpose: converts local NATS `/varz`, `/connz`, `/leafz`, and JetStream data
  into Prometheus metrics.
- Expected state: `active (running)`.
- Dependency: starts after and wants `nats-leaf.service`.
- Endpoint: TCP `7777` on the host network; Alloy scrapes
  `127.0.0.1:7777`.

### `alloy.service`

- Source: `/etc/containers/systemd/alloy.container`, rendered from the selected
  box profile.
- Runtime: Podman container `alloy`, running Grafana Alloy.
- Purpose: scrapes host metrics, NATS exporter metrics, and the Avena textfile;
  sends them to central Prometheus using remote write.
- Expected state: `active (running)`.
- Local readiness/API endpoint: `127.0.0.1:12345`.
- Remote-write endpoint: `prometheus.oats:9090`.
- Read-only host inputs: `/proc`, `/sys`, `/`, udev data, and
  `/var/lib/avena-rs/metrics`.
- The XFS node-exporter collector is deliberately disabled because it caused a
  repeating collection error on MU1.

`alloy-volume.service` may appear as `active (exited)`. It is an automatically
generated helper for Alloy's persistent Podman volume, not another daemon.

### `avena-streamer.service`

- Source: `/etc/systemd/system/avena-streamer.service`.
- Runtime: `/usr/local/libexec/avena-rs/streamer`, running as `user`.
- Purpose: mirrors central KV into local KV, watches configuration, connects
  directly to the configured LabJack, and publishes FlatBuffer sample batches
  into local JetStream and through the leaf connection.
- Expected state: `active (running)` while acquisition is enabled.
- Dependency: requires and starts after `nats-leaf.service`.
- Restart behavior: restart on failure. After the configured number of
  consecutive LabJack failures, it exits successfully so systemd does not
  hammer failed hardware indefinitely; an operator must correct the fault and
  restart it.
- Network: connects to local NATS `127.0.0.1:4222`, central NATS for config,
  and the LabJack address from the selected profile.

### `avena-archiver.service`

- Source: `/etc/systemd/system/avena-archiver.service`.
- Runtime: `/usr/local/libexec/avena-rs/archiver`, running as `user`.
- Purpose: consumes enabled live channels from local JetStream and writes
  Parquet archives.
- Expected state: `active (running)`.
- Dependency: requires and starts after `nats-leaf.service`.
- Restart behavior: restart on failure.
- Shutdown behavior: systemd sends `SIGINT`; the service stops each channel
  task, flushes it, writes the Parquet footer, and waits for completion.
- Active files end in `.parquet.inprogress`. A successful close atomically
  renames them to `.parquet`. Startup quarantines unfinished or unreadable
  files rather than exporting them.

### `avena-exporter.service`

- Source: `/etc/systemd/system/avena-exporter.service`.
- Runtime: `/usr/local/libexec/avena-rs/exporter`, running as `user`.
- Purpose: worker-mode CSV exports from local Parquet in response to central
  NATS requests.
- Expected state: `active (running)`.
- Dependency: requires and starts after `nats-leaf.service`.
- Restart behavior: restart on failure.
- NATS request subject:
  `avenars.<site_id>.<box_id>.<source_id>.export.request`.
- It does not listen on TCP port 9001 in the standard worker deployment. Port
  9001 is used only by optional direct WebSocket mode.

### `avena-health-metrics.timer`

- Source: `/etc/systemd/system/avena-health-metrics.timer`.
- Purpose: launches `avena-health-metrics.service` approximately every 15
  seconds.
- Expected state: `active (waiting)`.
- Expected boot state: enabled.

### `avena-health-metrics.service`

- Source: `/etc/systemd/system/avena-health-metrics.service`.
- Runtime: `/usr/local/libexec/avena-rs/write-edge-health-metrics.sh`, running
  as `user`.
- Purpose: writes service state, last stream time, last completed Parquet time,
  quarantine count, and NATS leaf count to
  `/var/lib/avena-rs/metrics/avena.prom`.
- Expected state: normally `inactive (dead)` between timer invocations. This is
  correct for a successful `Type=oneshot` service.
- Failure check: use `systemctl --failed` and inspect its journal; do not treat
  an inactive state alone as a fault.

### `wattdog.service`

- Source on the deployed host: `/etc/containers/systemd/wattdog.container`.
  This Quadlet is installed separately and is not currently generated by this
  repository.
- Runtime: Podman container `wattdog`, running the Wattdog Thornwave scanner.
- Purpose: reads Thornwave battery/solar telemetry and will control the power
  relay after bench validation.
- Intended state: `active (running)` with `bluetooth.service` active and
  Thornwave advertisements arriving.
- Current safety state: `--dry-run`; relay output must remain disabled until
  the wiring, cutoff thresholds, and repeated discharge/recharge tests pass.
- Local metrics endpoint: `127.0.0.1:9107` when configured and healthy.
- Host dependencies: Bluetooth and the system D-Bus socket.

## Supporting Host Services

These services are not built by this repository, but the deployed system relies
on them:

- `NetworkManager.service`: configures Ethernet and other host networking.
- `chronyd.service`: keeps timestamps synchronized; required for meaningful
  correlation between LabJack and camera data.
- `sshd.service`: provides remote shell access.
- `tailscaled.service`: provides the Tailscale address used for remote access
  and OATS name resolution/routing.
- `bluetooth.service`: required by Wattdog/Thornwave. It is intended to be
  active in the final deployment, but was inactive during the MU1 remote audit.
- `dbus-broker.service` or `dbus.service`: supplies the system bus mounted
  read-only into the Wattdog container.

Podman is daemonless. `podman.service` and `podman.socket` do not need to remain
active for Quadlet containers. Systemd starts the container through `podman`,
then a `conmon` process supervises each container. Seeing `conmon`, `crun`,
`nats-server`, `prometheus-nats-exporter`, `alloy`, or `wattdog` in `ps` is the
expected child-process view of the systemd units above. Manage the unit with
`systemctl`; do not kill these child processes directly.

## Operator-Initiated Processes

The following are tools, not persistent edge daemons:

- `subscriber`: diagnostic live NATS capture that writes per-channel CSV.
- `scripts/request-nats-export.mjs`: one export request through the same central
  NATS worker path used by the webapp.
- `shared/render-edge-config.py`: renders one selected MU profile.
- `scripts/install-edge-services.sh`: builds and installs the persistent Rust
  units and monitoring integration.
- `nats`, `curl`, `jq`, and `journalctl`: validation and troubleshooting tools.
- `pnpm dev`: local development webapp only; it should not run as an edge-box
  production service.

## Processes Outside The Edge Box

- Central `nats1.oats` and `nats2.oats`: authoritative central NATS/KV and leaf
  endpoints.
- Central `prometheus.oats`: accepts Alloy remote-write metrics.
- Operator laptop webapp: connects to the central NATS WebSocket endpoint; it
  does not connect directly to the edge-box NATS listener.
- PiKVM: a separate appliance for console and power recovery, not a process on
  the LattePanda.
- LabJack and Thornwave devices: physical devices, not host services.

## Ports and Connections

The normal edge runtime uses:

- `127.0.0.1:4222/tcp`: local NATS client and JetStream traffic.
- `127.0.0.1:8222/tcp`: local NATS monitoring endpoint.
- `7777/tcp`: Prometheus NATS exporter; consumed locally by Alloy.
- `127.0.0.1:12345/tcp`: Alloy readiness and local API.
- `127.0.0.1:9107/tcp`: Wattdog metrics, when enabled.
- `41641/udp`: Tailscale transport, subject to Tailscale's runtime choice.
- outbound `7422/tcp`: NATS leaf connections to the central servers.
- outbound `4222/tcp`: central NATS client connection for KV mirroring and
  remote export tests.
- outbound `9090/tcp`: Prometheus remote write.
- LabJack Ethernet: the configured device IP, currently `192.168.1.111` for the
  I-69 profiles.

Do not expose local NATS, NATS monitoring, Alloy, or Wattdog endpoints beyond
what is required. The standard config binds the sensitive administrative
endpoints to loopback; firewall the NATS exporter port if the host-network
container exposes it on other interfaces.

## Runtime Files and Data

- `/etc/avena-rs/profile.json`: installed box identity and paths.
- `/etc/avena-rs/{streamer,archiver,exporter}.env.json`: installed process
  environments.
- `/etc/avena-rs/apt.creds`: Rust-service NATS credentials; root-owned and
  group-readable by `user`.
- `/etc/containers/systemd/*.container`: installed Quadlet definitions.
- `/etc/containers/systemd/nats-leaf.conf`: rendered box-specific NATS config.
- `/etc/containers/systemd/creds/leaf.creds`: leaf connection credentials.
- `/usr/local/libexec/avena-rs/`: installed Rust binaries and service scripts.
- `/var/lib/nats` in the container: local JetStream data, backed by the host
  path configured in the NATS Quadlet.
- `/var/lib/avena-rs/metrics/avena.prom`: atomic textfile metrics output.
- `<PARQUET_DIR>/assetNNN/YYYY-MM-DD/chNN/`: completed and active archive
  parts.

## One-Command Status Check

```bash
systemctl is-active \
  NetworkManager chronyd sshd tailscaled \
  nats-leaf nats-exporter alloy \
  avena-streamer avena-archiver avena-exporter \
  avena-health-metrics.timer

systemctl is-enabled \
  tailscaled sshd \
  avena-streamer avena-archiver avena-exporter \
  avena-health-metrics.timer

systemctl --failed
curl -fsS http://127.0.0.1:12345/-/ready
curl -fsS http://127.0.0.1:8222/leafz | jq '.leafnodes'
```

Also check Wattdog separately because it is still under power-system testing:

```bash
systemctl status bluetooth wattdog --no-pager
journalctl -u wattdog -n 60 --no-pager
```

For a healthy data deployment, the persistent services should be active, the
health timer should be waiting, `avena-health-metrics.service` should have a
recent successful invocation, the NATS leaf count should be at least one, and
the stream/archive timestamps in Prometheus should continue advancing.
