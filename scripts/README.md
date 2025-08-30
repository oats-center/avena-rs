# Avena Hub/Edge Scripts

These scripts set up a **hub + edge** NATS topology for LabJack sampling with a **serverless Svelte PWA**.

- Users connect **only to the hub** via **WebSocket** (WS/WSS).
- Edges are **not directly reachable** by users; each edge runs NATS+JetStream locally and leaf-links to the hub.
- The PWA:
  - **reads** live data from `labjack.>.data.>`
  - **writes** sampler config by publishing to `KV.sampler_cfg.edge<id>` (only if the user has “cfg” rights)

---

## Architecture (simple & safe)

**Hub**
- TCP client port `4222` bound to **127.0.0.1** (hidden).
- **WebSocket** listener `8080` for the PWA (binds to `HUB_WS_HOST`; put TLS proxy in front for `wss://`).
- **Leafnode** listener `7422` for edge connections (binds to `HUB_LEAF_HOST`).
- Per-user accounts stored **on the hub**:
  - `auth_read.conf` → users who can **subscribe** `labjack.>.data.>`
  - `auth_cfg.conf` → users who can **publish** only specific KV keys (e.g., `KV.sampler_cfg.edge075`)

**Edge (one per device)**
- TCP client port `4222` bound to **127.0.0.1** (only the local sampler connects).
- **JetStream enabled** (stores streams & KV bucket locally).
- **Leafnode remote** connects **out** to the hub on `7422` using a shared `leaf` username/password.
- Sampler connects to `nats://127.0.0.1:4222`.

**PWA flow**
- Connects to hub WS as a **read user** → subscribes `labjack.>.data.>`.
- When operator saves config → connect as a **cfg user** and `publish` JSON to `KV.sampler_cfg.edgeXYZ`, then disconnect.

---

## What is public vs. secret?

**Public**
- Hub WS URL (e.g., `wss://hub.example.com` in prod; `ws://100.64.0.76:8080` in dev)
- Subject names (e.g., `labjack.>.data.>`, `KV.sampler_cfg.edgeXYZ`)
- Edge IDs (e.g., `075`, `087`)
- Config JSON schema

**Secret (on hub only)**
- Per-user **username/password** entries in:
  - `/etc/nats/hub/auth_read.conf`
  - `/etc/nats/hub/auth_cfg.conf`
- Edge leaf credentials: `{ user: "leaf", pass: "<secret>" }` (used by edges to connect to hub; not shared with end users)

---

## Scripts included

### Linux (hub)
- `linux_hub_setup.sh` – creates hub config/service (WS, leaf listen, local TCP only) and empty auth lists.
- `linux_hub_kill.sh` – removes hub service/config.

### Linux (edge)
- `linux_edge_setup.sh` – creates edge config/service (JS on, local TCP only, leaf to hub). Optional sampler unit.
- `linux_edge_kill.sh` – removes edge service/config/data.

### macOS (edge)
- `mac_edge_setup.sh` – edge config and `launchd` service. Optional sampler plist.
- `mac_edge_kill.sh` – removes edge service/config/data.

### Seeding (from your laptop or any box with SSH)
- `seed_edge_configs.sh` – SSH into each edge and seed a default KV entry for that edge.

> **Prereqs**
> - **NATS server** present on each machine:
>   - Linux default path: `/usr/local/bin/nats-server` (override with `NATS_BIN`)
>   - macOS default path: `/opt/homebrew/bin/nats-server` (override with `NATS_BIN`)
> - Optional: `nats` CLI installed **on each edge** if you plan to run the seeder.

---

## Quickstart

### 1) Hub

**Linux**
```bash
# run on hub
sudo HUB_WS_HOST=0.0.0.0 HUB_LEAF_HOST=0.0.0.0 \
     LEAF_USER=leaf LEAF_PASS='leafpass' \
     ./linux_hub_setup.sh
````

This creates:

* `/etc/nats/hub/server.conf`
* `/etc/nats/hub/auth_read.conf` (empty)
* `/etc/nats/hub/auth_cfg.conf` (empty)
* Hub service running

### 2) Create users (per person) on the hub

SSH to the hub and edit:

**`/etc/nats/hub/auth_read.conf`** (one line per user)

```hocon
{ user: "alice", pass: "alice-read",
  permissions: { publish: ["_INBOX.>"], subscribe: ["labjack.>.data.>"] } },
```

**`/etc/nats/hub/auth_cfg.conf`** (KV write to specific edges)

```hocon
{ user: "alice-cfg", pass: "alice-cfg-pass",
  permissions: { publish: ["KV.sampler_cfg.edge075","KV.sampler_cfg.edge087"],
                 subscribe: ["_INBOX.>"] } },
```

Then restart hub:

```bash
# Linux
sudo systemctl restart nats-hub
# mac
sudo launchctl kickstart -k system/io.nats.hub
```

> You can give a single account both abilities by merging permissions into one line (less safe), or keep two (safer: least privilege).

### 3) Edges

Run on each edge.

**Linux**

```bash
# Edge 075
sudo EDGE_ID=075 HUB_HOST=100.64.0.76 LEAF_USER=leaf LEAF_PASS='leafpass' \
     ./linux_edge_setup.sh

# Edge 087
sudo EDGE_ID=087 HUB_HOST=100.64.0.76 LEAF_USER=leaf LEAF_PASS='leafpass' \
     ./linux_edge_setup.sh
```

**macOS**

```bash
# Edge 075
sudo EDGE_ID=075 HUB_HOST=100.64.0.76 LEAF_USER=leaf LEAF_PASS='leafpass' \
     ./mac_edge_setup.sh

# Edge 087
sudo EDGE_ID=087 HUB_HOST=100.64.0.76 LEAF_USER=leaf LEAF_PASS='leafpass' \
     ./mac_edge_setup.sh
```

This creates:

* `/etc/nats/edge<id>/server.conf`
* JetStream dir at `/var/lib/nats/edge<id>/jetstream`
* Parquet dir at `/var/lib/edge<id>/parquet`
* Edge NATS service running (`client :4222` on localhost; leaf -> hub)

### 4) Seed default config (KV per edge)

From your **laptop** (or any box with SSH to edges):

```bash
./seed_edge_configs.sh \
  --edges "075:root@100.64.0.75,087:root@100.64.0.87" \
  --scans 200 --rate 7000.0 \
  --channels "0,1,2,3,4,5,6,7,8,9,10,11,12,13"
```

This runs on each edge:

* `nats kv add bucket sampler_cfg` (if missing)
* `nats kv put sampler_cfg edge<id> <json>`

### 5) PWA connection hints

* **WS URL**: `ws://<hub-ip>:8080` (use **`wss://`** via reverse proxy in production)
* **Read login**: per-user `user` / `pass` from `auth_read.conf` → subscribe `labjack.>.data.>`
* **Cfg login**: per-user `user` / `pass` from `auth_cfg.conf` → publish to `KV.sampler_cfg.edgeXYZ` only
* Keep **cfg** creds ephemeral (ask when saving; close connection after write).

---

## Config JSON (example)

The app should only let users change these fields:

```json
{
  "scans_per_read": 200,
  "suggested_scan_rate": 7000.0,
  "channels": [0,1,2,3,4,5,6,7,8,9,10,11,12,13],
  "asset_number": 75,
  "nats_url": "nats://127.0.0.1:4222",
  "nats_subject": "labjack",
  "nats_stream": "LABJACK",
  "parquet_file": "/var/lib/edge075/parquet"
}
```

**KV subject to publish** for edge `075`: `KV.sampler_cfg.edge075`
(Your sampler already watches this key and restarts when it changes.)

---

## Ports & exposure

* **Hub**

  * WS: `8080` (public via TLS proxy → `wss://…`)
  * Leaf: `7422` (edge → hub; restrict sources if possible)
  * TCP client: `4222` bound to `127.0.0.1` (hidden)
* **Edge**

  * TCP client: `4222` bound to `127.0.0.1` (hidden)
  * No WS listener

---

## Security notes (practical)

* Per-user accounts let you **revoke/rotate** one person without interrupting others.
* Keep the **edges** invisible to users (client on localhost; only leaf → hub).
* The **browser** never sees device/root secrets; only the user’s own hub credentials.
* Use **TLS** for WS in production (`wss://`) via a reverse proxy (Caddy/NGINX).
* Optionally rate-limit and IP-restrict the hub’s WS listener.

---

## Kill and Reset Setup

**Hub**

```bash
# Linux
sudo ./linux_hub_kill.sh
```

**Edge**

```bash
# Linux
sudo EDGE_ID=075 ./linux_edge_kill.sh
# mac
sudo EDGE_ID=075 ./mac_edge_kill.sh
```

---

## Troubleshooting

* **PWA can’t connect**: confirm WS is listening (`ss -lntp | grep 8080` on Linux hub), and credentials are present in the correct `auth_*.conf`.
* **No data in PWA**: check leaf links: on hub,

  ```bash
  nats --server nats://127.0.0.1:4222 server report connections
  ```

  (Run from the hub box; TCP client is localhost-only.)
* **KV writes don’t seem to take**: on the edge, verify the KV entry exists:

  ```bash
  nats --server nats://127.0.0.1:4222 kv get sampler_cfg edge075
  ```
* **Sampler not restarting after config**: check sampler logs (`journalctl -u sampler-075 -f` on Linux).

---

## Changing or adding edges

* Add a new edge with the same edge setup script (`EDGE_ID=XYZ`).
* Add its key to PWA UI, and (optionally) grant specific users `KV.sampler_cfg.edgeXYZ` in `auth_cfg.conf`.
* Restart hub after editing `auth_cfg.conf`.

---

## File map

```
/etc/nats/hub/server.conf            # hub listener definitions
/etc/nats/hub/auth_read.conf         # per-user read accounts
/etc/nats/hub/auth_cfg.conf          # per-user config-writer accounts
/var/lib/nats/edge<id>/jetstream     # edge JetStream storage
/var/lib/edge<id>/parquet            # edge parquet output directory
```

---

## Minimal Svelte pattern (conceptual)

* **Always-on** read connection (user’s read account).
* **On save**: prompt for cfg credentials → open short-lived connection → publish KV → close → forget password.
* Guard rail: display “Last updated” timestamp (KV revision) and confirm overwrite if changed.

```
