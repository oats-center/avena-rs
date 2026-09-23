# Setting up a new edge node

This guide takes a freshly installed Fedora machine to a working edge node: the
LabJack is streaming, samples reach central NATS, Parquet files are being
written, and exports work from the webapp. Every step ends with a check. Do not
move on until the check passes; each step depends on the ones before it.

Plan on two to three hours if everything is at hand. The slow parts are
installing packages and the first Rust build.

## What you need

**Hardware.** The edge computer (a LattePanda running Fedora on x86-64) with its
large data disk mounted at `/extstore`, a LabJack T7 on the same Ethernet
network, and a DHCP reservation for the LabJack so its address never changes.

**Accounts and credentials.** Ask the NATS administrator for two files made for
this box. Never reuse another box's files.

| File | Used by | Purpose |
|---|---|---|
| `apt.creds` | the Rust services and the `nats` CLI | Client access to local and central NATS |
| `leaf.creds` | the local NATS server | The box's leaf connection to central NATS |

You also need a Tailscale login for the deployment, since the central servers
(`nats1.oats`, `nats2.oats`, `prometheus.oats`) are only reachable over it.

**Values for this box.** Decide these before starting, and set them as shell
variables so the commands below can be pasted as they are. The examples are for
a hypothetical third I-69 box.

```bash
export SITE_ID=i69                 # site name, shared by all boxes at the site
export BOX_ID=i69-mu3              # this box; also its hostname
export SOURCE_ID=i69-lj3           # this box's LabJack, as named in NATS
export ASSET_NUMBER=1003           # unique integer, shown in the webapp
export LABJACK_IP=192.168.1.113    # the reserved address of the LabJack
export JS_DOMAIN=edge-$BOX_ID      # the box's JetStream domain
export KV_KEY=$SITE_ID.$BOX_ID.$SOURCE_ID.config
```

Two rules keep boxes from interfering with each other: `BOX_ID`, `SOURCE_ID`,
`ASSET_NUMBER` and the JetStream domain must be unique across all edge nodes,
and the LabJack serial in the profile must be the serial of the device actually
wired to this box.

The commands run on the edge node as the login user `user`, unless shown with
`sudo`.

## 1. Prepare Fedora

Set the name and time zone, enable the base services, and install the packages
the build and the containers need.

```bash
sudo hostnamectl set-hostname "$BOX_ID"
sudo timedatectl set-timezone America/New_York
sudo dnf install -y podman jq curl git gcc gcc-c++ make openssl-devel pkg-config rsync
sudo systemctl enable --now NetworkManager chronyd sshd tailscaled
```

Install Rust with rustup, which puts `cargo` in `~/.cargo/bin`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
```

Install the `nats` command-line tool. Download the `linux-amd64` RPM from the
[natscli releases page](https://github.com/nats-io/natscli/releases) and install
it with `sudo dnf install ./nats-*.rpm`.

**Check:**

```bash
hostname                      # prints the value of $BOX_ID
cargo --version && nats --version && podman --version
```

## 2. Join Tailscale and check the central names

Bring the box onto the deployment's Tailscale network with the approved
hostname and tags. The exact `tailscale up` flags depend on the network, so copy
them from an existing box (`tailscale debug prefs`) rather than from memory.

**Check:** all three central names resolve.

```bash
getent hosts nats1.oats nats2.oats prometheus.oats
```

## 3. Make sure the clock is synchronized

Every sample timestamp comes from this machine's clock. A box that is a few
seconds off records perfectly good data with the wrong times, and nothing else
will warn you about it.

```bash
chronyc sources
timedatectl
```

**Check:** `timedatectl` reports `System clock synchronized: yes`, and
`chronyc sources` lists at least one server whose `Reach` column is not `0`.

If no source is reachable, look at the `server` and `pool` lines in
`/etc/chrony.conf`. MU1 once ran for weeks 3.3 s fast because its only server
name was misspelled. Keep a `pool` line as a fallback:

```text
server time.cloudflare.com iburst
pool 2.fedora.pool.ntp.org iburst
```

Then `sudo systemctl restart chronyd` and check again.

## 4. Get the source and prepare storage

Cloning needs read access to the repository on GitHub.

```bash
cd /home/user
git clone https://github.com/oats-center/avena-rs.git
mkdir -p /home/user/nats \
         /home/user/avena-rs/rust-ljm/parquet \
         /home/user/avena-rs/rust-ljm/outputs
```

`/home/user/nats` holds the local JetStream store and
`rust-ljm/parquet` holds the archive. Put both on the disk you intend to fill.

**Check:** the disks are the ones you expect, with room to spare.

```bash
df -hT /home/user /extstore
```

## 5. Install LJM and confirm the LabJack

The streamer talks to the LabJack through LabJack's LJM library. Download the
LJM installer for Linux (x86-64) from [labjack.com](https://labjack.com) and run
it. It installs `libLabJackM.so` into the system library path.

**Check:** the library is visible, the LabJack answers, and it opens as a T7.

```bash
ldconfig -p | grep LabJackM
ping -c 3 "$LABJACK_IP"
cd /home/user/avena-rs/rust-ljm
LABJACK_IP=$LABJACK_IP cargo run --example info
```

The last command prints `Device Type: T7` and the IP address. It does not print
the serial number. Read the serial from the label on the T7 (or with LabJack's
Kipling application) and write it down for the next step. The streamer checks
it every time it connects and refuses a device with a different serial, so a
mistake here shows up immediately at step 10.

## 6. Write the box profile

Each box is described by one JSON file in `shared/edge-boxes/`. Everything else
the box needs (service environments, the NATS server config, the initial
LabJack configuration) is generated from it, so this file is the only place a
box's identity is written down. Start from an existing box:

```bash
cd /home/user/avena-rs
cp shared/edge-boxes/i69-mu2.json shared/edge-boxes/$BOX_ID.json
```

Edit the new file and change these fields. The [profile
reference](../reference/profile.md) describes every field.

| Field | Set to |
|---|---|
| `box_id` | `$BOX_ID` |
| `source.id` | `$SOURCE_ID` |
| `nats.leaf_server_name` | `$BOX_ID-leaf` |
| `nats.jetstream_domain` | `$JS_DOMAIN` |
| `nats.kv_key` | `$KV_KEY` |
| `labjack.name` | `$SOURCE_ID` |
| `labjack.asset_number` | `$ASSET_NUMBER` |
| `labjack.ip` | `$LABJACK_IP` |
| `labjack.serial` | the serial from the label |
| `labjack.sensor_settings` | the channels, scan rate and calibrations to record |

Render the generated files into a folder of their own:

```bash
./shared/render-edge-config.py \
  --config shared/edge-boxes/$BOX_ID.json \
  --output-dir target/edge-config/$BOX_ID
```

**Check:** the renderer prints the box, leaf name, JetStream domain and KV key,
and they are this box's values, not the box you copied from.

```bash
jq -r '.env.BOX_ID, .env.JS_DOMAIN, .env.CFG_KEY, .env.LABJACK_IP, .env.LABJACK_SERIAL' \
  target/edge-config/$BOX_ID/streamer.env.json
```

Commit the new profile to the repository. The generated folder under `target/`
is never committed; it can always be rendered again.

## 7. Install the credentials

```bash
cd /home/user/avena-rs
install -m 0600 /path/to/apt.creds rust-ljm/apt.creds
sudo install -d -m 0700 /etc/containers/systemd/creds
sudo install -m 0600 /path/to/leaf.creds /etc/containers/systemd/creds/leaf.creds
```

The two paths are easy to confuse:

- `rust-ljm/apt.creds` is where the installer picks up the client credentials.
  It copies them to `/etc/avena-rs/apt.creds`, which is the path the services
  actually read.
- `/etc/containers/systemd/creds/` on the host is mounted into the NATS
  container as `/etc/nats/creds/`. That is why the profile's
  `nats.leaf_credentials_path` says `/etc/nats/creds/leaf.creds`: it is a path
  inside the container.

## 8. Start the local NATS server and monitoring

The NATS server, its Prometheus exporter and Alloy run as Podman containers
managed by systemd through Quadlet files.

```bash
cd /home/user/avena-rs
B=target/edge-config/$BOX_ID
sudo install -m 0644 $B/nats-leaf.conf           /etc/containers/systemd/nats-leaf.conf
sudo install -m 0644 shared/nats-leaf.container   /etc/containers/systemd/nats-leaf.container
sudo install -m 0644 shared/nats-exporter.container /etc/containers/systemd/nats-exporter.container
sudo install -m 0644 shared/config.alloy          /etc/containers/systemd/config.alloy
sudo install -m 0644 $B/alloy.container           /etc/containers/systemd/alloy.container
sudo systemctl daemon-reload
sudo systemctl enable --now nats-leaf nats-exporter alloy
```

The first start pulls the container images, which takes a minute.
`systemctl is-enabled` reports these units as `generated` rather than
`enabled`. That is normal for Quadlet units.

**Check:** the server runs with this box's name and JetStream, the leaf link to
central is up, and your credentials work both locally and centrally.

```bash
curl -fsS http://127.0.0.1:8222/varz | jq '{server_name, jetstream: (.jetstream != null)}'
curl -fsS http://127.0.0.1:8222/leafz | jq '.leafnodes'      # 1 or more
nats --server nats://127.0.0.1:4222 --creds rust-ljm/apt.creds rtt
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds rtt
```

`server_name` must be `$BOX_ID-leaf`. If the local `rtt` works but the central
one fails with `Authorization Violation`, the credentials are not authorized on
the central servers yet; that is fixed on the central side.

## 9. Seed the LabJack configuration

The streamer and archiver read their recording settings from the key-value
bucket `avenabox`. The central copy is the one the webapp edits. The edge node
keeps a mirror so it can start and keep recording while central NATS is
unreachable. Seed both from the file rendered for this box (not from
`shared/labjack-kv.generated.json`, which may belong to another box):

```bash
cd /home/user/avena-rs
KV=target/edge-config/$BOX_ID/labjack-kv.generated.json
CREDS=rust-ljm/apt.creds

nats --server nats://nats1.oats:4222 --creds $CREDS kv add avenabox --history=5 || true
nats --server nats://nats1.oats:4222 --creds $CREDS kv put avenabox "$KV_KEY" "$(cat $KV)"

nats --server nats://127.0.0.1:4222 --creds $CREDS --js-domain $JS_DOMAIN kv add avenabox --history=5 || true
nats --server nats://127.0.0.1:4222 --creds $CREDS --js-domain $JS_DOMAIN kv put avenabox "$KV_KEY" "$(cat $KV)"
```

`kv add` fails harmlessly when the bucket already exists.

**Check:** both copies read back as this box's configuration.

```bash
nats --server nats://nats1.oats:4222 --creds $CREDS kv get avenabox "$KV_KEY" --raw | jq '.box_id, .sensor_settings.scan_rate_hz'
nats --server nats://127.0.0.1:4222 --creds $CREDS --js-domain $JS_DOMAIN kv get avenabox "$KV_KEY" --raw | jq '.box_id'
```

## 10. Install and start the Rust services

The installer builds the release binaries, installs them with their
configuration, and enables the systemd units.

```bash
cd /home/user/avena-rs
./scripts/install-edge-services.sh --profile shared/edge-boxes/$BOX_ID.json
```

It puts the binaries in `/usr/local/libexec/avena-rs/`, the configuration and
credentials in `/etc/avena-rs/`, and the units in `/etc/systemd/system/`. The
services never run anything from `rust-ljm/target`, so later builds do not
affect a running box until you install them.

Start the consumers before the producer, so the archiver is attached to the
stream before the first sample arrives:

```bash
sudo systemctl start avena-archiver avena-exporter
sudo systemctl start avena-streamer
```

**Check:** the streamer connected to the right device and is streaming.

```bash
journalctl -u avena-streamer -n 20 --no-pager
```

Look for `connected via ETHERNET, serial <your serial>` and `Streaming started`.
Then confirm that samples reach central NATS:

```bash
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  sub "avenars.$SITE_ID.$BOX_ID.$SOURCE_ID.live.>" --count 3
```

Finally, wait for the next five-minute boundary plus a few seconds, and confirm
the archiver closed its first files:

```bash
find rust-ljm/parquet -name '*.parquet' -mmin -10 | head
journalctl -u avena-archiver -n 20 --no-pager | grep Closed
```

## 11. Install the camera (if this box has one)

The roadside camera software lives in the separate `edge-code` repository and
has its own installer and README. It is independent of the services above, so
it can be installed now or later.

## 12. Reboot and verify

A box that works until its first power cut is not finished. Install the status
command and reboot:

```bash
install -m 0755 /home/user/avena-rs/scripts/edge-status.sh /home/user/avena-status
sudo reboot
```

Wait a minute after it comes back, then run:

```bash
/home/user/avena-status
```

**Check:** the command exits with status 0. It lists every expected unit as
active, shows the NATS server with JetStream enabled and at least one central
leaf connection, and shows a recent LabJack file. Also open the box in the
webapp and confirm the live plot moves and a short export downloads (see [The
webapp](webapp.md)).

The box is now set up. [Services on an edge node](../operations/services.md)
describes what is running and how to look after it.
