# Avena LattePanda field deployment runbook

This is the offline operator guide for installing one complete Avena edge box.
It covers the camera, LabJack streamer, Parquet archiver, export worker, local
NATS leaf/JetStream, NATS metrics exporter, and Grafana Alloy. WattDog runs on
the PiKVM and is outside this LattePanda procedure.

The commands assume Fedora, a login named `user`, and these source directories:

```text
/home/user/avena-rs     LabJack, NATS, exporter, and monitoring source
/home/user/edge-code    camera source and bundled BMR-Stem ONNX model
```

Do not copy an installed `/opt/edge-code`, `/etc`, `target`, NATS store, or
Parquet directory from another box. Copy source, create a unique profile, and
run the installers.

## 1. Record the new box values

Fill these in before changing files:

```text
HOSTNAME=                       example: i69-mu3
SITE_ID=                        example: i69
BOX_ID=                         normally the hostname
LABJACK_SOURCE_ID=              example: i69-lj3
LABJACK_NAME=                   example: I69-Box-3
LABJACK_ASSET_NUMBER=           unique integer
LABJACK_IP=                     DHCP reservation, example: 192.168.1.12
LABJACK_SERIAL=                 printed by cargo run --example info
CAMERA_IP=                      DHCP reservation, example: 192.168.1.10
CAMERA_ID=                      example: roadside-camera-3
TAILSCALE_HOSTNAME=             example: i69-mu3
JETSTREAM_DOMAIN=               example: edge-i69-mu3
CENTRAL_KV_KEY=                 SITE_ID.BOX_ID.LABJACK_SOURCE_ID.config
```

Also obtain two credential files from the Avena/NATS administrator:

```text
apt.creds       client credentials for streamer, archiver, exporter, and CLI
leaf.creds      credentials dedicated to this box's central leaf connection
```

Never reuse another deployed box's identity, leaf user, asset number, or
LabJack serial without deliberately replacing that box.

## 2. Prepare Fedora, networking, storage, and time

```bash
sudo hostnamectl set-hostname "$HOSTNAME"
sudo timedatectl set-timezone America/New_York
sudo systemctl enable --now NetworkManager chronyd sshd tailscaled
sudo dnf install -y podman jq curl git gcc gcc-c++ make openssl-devel pkg-config rsync python3.13
```

Configure Tailscale/Headscale with the deployment's approved hostname, tags,
and login server. Do not copy a `tailscale up` command blindly; first inspect
the required current non-default flags:

```bash
tailscale status
tailscale debug prefs
```

Confirm the clock and required names before collecting data:

```bash
timedatectl
chronyc tracking
getent hosts nats1.oats nats2.oats prometheus.oats
```

`System clock synchronized: yes` is required. Starting the streamer with a bad
clock creates correctly written files carrying incorrect event timestamps.

Create persistent storage. `/extstore` must be the intended large filesystem,
not an empty directory accidentally residing on a small root disk:

```bash
findmnt /extstore
df -hT /extstore
mkdir -p /home/user/nats
mkdir -p /home/user/avena-rs/rust-ljm/parquet
mkdir -p /home/user/avena-rs/rust-ljm/outputs
sudo install -d -o user -g user -m 0750 /extstore/camera
```

## 3. Install LabJack LJM and confirm the device identity

Install the vendor LJM runtime from the approved offline installer or LabJack
package. The runtime must provide `libLabJackM.so` in the dynamic linker path.

```bash
ldconfig -p | grep LabJackM
ping -c 3 "$LABJACK_IP"
cd /home/user/avena-rs/rust-ljm
cargo run --example info
```

Do not continue if the connected serial differs from the new box profile.

## 4. Create the immutable box profile

Start from the closest profile, then edit only the new file:

```bash
cd /home/user/avena-rs
cp shared/edge-boxes/i69-mu2.json "shared/edge-boxes/$BOX_ID.json"
${EDITOR:-vi} "shared/edge-boxes/$BOX_ID.json"
```

At minimum change:

- `box_id`, `source.id`, and `labjack.name`
- `labjack.asset_number`, `labjack.ip`, and `labjack.serial`
- `nats.leaf_server_name`, `nats.jetstream_domain`, and `nats.kv_key`
- the desired channel list, scan rate, and scans per read
- all paths if the new disk layout differs

Validate and render without installing:

```bash
jq . "shared/edge-boxes/$BOX_ID.json"
./shared/render-edge-config.py \
  --config "shared/edge-boxes/$BOX_ID.json" \
  --output-dir "target/edge-config/$BOX_ID"
jq -r '.env.BOX_ID, .env.LABJACK_IP, .env.LABJACK_SERIAL, .env.CFG_KEY' \
  "target/edge-config/$BOX_ID/streamer.env.json"
grep -nE 'server_name:|domain:' "target/edge-config/$BOX_ID/nats-leaf.conf"
```

Everything under `target/edge-config` is generated. The profile under
`shared/edge-boxes` is the source of truth.

## 5. Install credentials and local NATS/Alloy containers

```bash
cd /home/user/avena-rs
install -m 0600 /path/to/apt.creds rust-ljm/apt.creds
sudo install -d -m 0700 /etc/containers/systemd/creds
sudo install -m 0600 /path/to/leaf.creds /etc/containers/systemd/creds/leaf.creds

sudo install -m 0644 "target/edge-config/$BOX_ID/nats-leaf.conf" \
  /etc/containers/systemd/nats-leaf.conf
sudo install -m 0644 shared/nats-leaf.container \
  /etc/containers/systemd/nats-leaf.container
sudo install -m 0644 shared/nats-exporter.container \
  /etc/containers/systemd/nats-exporter.container
sudo install -m 0644 shared/config.alloy \
  /etc/containers/systemd/config.alloy
sudo install -m 0644 "target/edge-config/$BOX_ID/alloy.container" \
  /etc/containers/systemd/alloy.container

sudo systemctl daemon-reload
sudo systemctl enable --now nats-leaf nats-exporter alloy
```

The first run downloads container images unless they were preloaded. For a
network-isolated installation, preload the exact images listed in the Quadlet
files with `podman load`.

Validate local NATS and the outbound leaf:

```bash
curl -fsS http://127.0.0.1:8222/varz | jq '{server_name,jetstream}'
curl -fsS http://127.0.0.1:8222/leafz | jq '{leafnodes,leafs}'
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds rtt
```

## 6. Seed configuration and install the LabJack services

The generated file `shared/labjack-kv.generated.json` is a convenience output;
verify it corresponds to the selected profile before seeding. Seed central KV
and local KV as described in `avena-rs/docs/setup-guide.md`. Local KV lets the
box start when central NATS is temporarily unavailable.

Then build and install the Rust services:

```bash
cd /home/user/avena-rs
./scripts/install-edge-services.sh \
  --profile "shared/edge-boxes/$BOX_ID.json" \
  --start
```

This installs immutable runtime copies under `/usr/local/libexec/avena-rs`,
configuration under `/etc/avena-rs`, and the systemd units for streamer,
archiver, exporter, and health metrics. The runtime does not execute binaries
from `rust-ljm/target`.

## 7. Install the camera application

The camera source package is self-contained and includes the BMR-Stem model.
Use Python 3.13 because the application currently requires Python `>=3.11,<3.14`.

```bash
cd /home/user/edge-code
sudo AVENA_CAMERA_PYTHON=/usr/bin/python3.13 ./scripts/install.sh
sudoedit /etc/avena-camera.yaml
sudoedit /etc/avena-camera.env
```

Set unique `device_id` and `camera_id` values in YAML. Put the secret only in
the root-readable env file:

```ini
AVENA_CAMERA_SOURCE=rtsp://admin:URL_ENCODED_PASSWORD@CAMERA_IP:554/h264Preview_01_sub
```

Validate and start:

```bash
cd /opt/edge-code
sudo ./scripts/validate.sh
sudo systemctl restart avena-camera
systemctl status avena-camera --no-pager
journalctl -u avena-camera -n 80 --no-pager
```

The installed service retries forever if the camera is absent and begins work
when RTSP becomes reachable.

## 8. Install and use the local status command

```bash
install -m 0755 /home/user/avena-rs/scripts/edge-status.sh /home/user/avena-status
/home/user/avena-status
```

Exit status `0` means all expected units, NATS endpoints, the central leaf, and
systemd failure state passed. A nonzero exit means at least one check failed.

## 9. Mandatory reboot test

```bash
sudo reboot
```

After reconnecting, wait about one minute and run:

```bash
/home/user/avena-status
systemctl is-enabled avena-camera avena-streamer avena-archiver avena-exporter
systemctl is-enabled nats-leaf nats-exporter alloy
timedatectl
```

Confirm the latest LabJack timestamp advances and the camera has an established
RTSP connection:

```bash
ss -tnp | grep "$CAMERA_IP:554"
journalctl -u avena-streamer -b --no-pager | tail -40
journalctl -u avena-camera -b --no-pager | tail -40
```

## 10. Directory map and cleanup rules

```text
KEEP: /home/user/avena-rs
  Source repository. shared/edge-boxes/<box>.json is the deployment source of truth.

KEEP: /home/user/edge-code
  Self-contained camera source, installer, tests, and bundled model.

KEEP: /opt/edge-code
  Installed camera runtime. Replace only by rerunning edge-code/scripts/install.sh.

KEEP: /etc/avena-rs, /etc/avena-camera.*, /etc/containers/systemd
  Installed runtime configuration and credentials.

NEVER DELETE WHILE DEPLOYED: /home/user/nats
  Live local JetStream store mounted by nats-leaf.

DATA: /home/user/avena-rs/rust-ljm/parquet and /extstore/camera
  Field measurements and camera outputs. Stop writers and apply a precise cutoff.

REGENERATABLE: /home/user/avena-rs/rust-ljm/target
  Rust compiler output. Runtime services use /usr/local/libexec, not this folder.

REGENERATABLE: /home/user/avena-rs/target
  Rendered configs and generated documentation.

REGENERATABLE: webapp/node_modules, webapp/build, webapp/.svelte-kit
  Frontend dependency/build caches; not used by edge runtime services.
```

Always verify service references and ask the operator before deleting anything.
Never use a broad recursive delete against `/home/user`, `/extstore`, or a path
held in an unverified variable.

## 11. Offline installation kit

For a deployment with no internet, prepare this kit on another Fedora x86-64
machine before visiting the site:

- complete `avena-rs` source including `Cargo.lock`
- complete `edge-code` source including `requirements.lock` and the ONNX model
- LabJack LJM installer
- box-specific `apt.creds` and `leaf.creds`, transported securely
- RPMs for the prerequisite packages and Python 3.13
- a populated Cargo vendor/cache or prebuilt release binaries from the same OS
- Python wheels matching Python 3.13 and x86-64
- saved Podman images matching the exact Quadlet image names

Keep credentials outside ordinary source archives. Verify checksums for the
model, installers, wheels, RPMs, binaries, and container archives.

The detailed NATS/KV troubleshooting reference remains
`/home/user/avena-rs/docs/setup-guide.md`; camera calibration and tracker
validation remain in `/home/user/edge-code/README.md`.
