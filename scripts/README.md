# NATS Hub + Edge Secure Deployment (with TLS + Tailscale)

##  Overview

* **Hub**: central NATS server with TLS and per-user accounts.
* **Edges**: local NATS servers with JetStream, connect via TLS leaf links.
* **Developers**: connect via WebSocket/TLS with client cert + password.

 All trust is anchored by an **offline CA** that signs certs.
 Runs inside your **Tailscale network** for private IP addressing.

---

## 1. Setup CA (offline machine)

```bash
openssl genrsa -out ca.key 4096
openssl req -x509 -new -key ca.key -out ca.crt -days 3650 -subj "/CN=NATS-CA"
```

* Keep `ca.key` **offline and secure**.
* Distribute only `ca.crt` to hub, edges, and developers.

---

## 2. Install Hub

On hub server (Tailscale IP `100.64.0.10` shown as example):

```bash
scp ca.crt hub:/etc/nats/hub/certs/
sudo HUB_HOST=100.64.0.10 ./hub_install.sh
```

Generate hub CSR:

```bash
openssl req -new -newkey rsa:2048 -nodes \
  -keyout hub.key -out hub.csr \
  -subj "/CN=100.64.0.10"
```

Sign CSR offline:

```bash
openssl x509 -req -in hub.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -out hub.crt -days 365
```

Copy `hub.crt` + `hub.key` into `/etc/nats/hub/certs/`, then:

```bash
sudo systemctl restart nats-hub
```

---

## 3. Add Developer (example: Alice)

On hub:

```bash
sudo ./add_dev.sh alice reader
```

* This creates a CSR in `/etc/nats/hub/certs/alice.csr`.
* Sign it offline with your CA.

Copy back `alice.crt`, then finalize:

```bash
sudo ./finish_add_dev.sh alice reader
```

Give Alice:

* `alice.crt`, `alice.key`, `ca.crt`
* Her password (set when creating the account)

To remove:

```bash
sudo ./remove_dev.sh alice
```

**Best practice**: generate unique certs per device, not just per user.

---

## 4. Install Edge

On edge server (ID `075`):

```bash
scp ca.crt edge:/etc/nats/edge075/certs/
sudo EDGE_ID=075 HUB_HOST=100.64.0.10 ./edge_install.sh
```

Generate edge CSR:

```bash
openssl req -new -newkey rsa:2048 -nodes \
  -keyout edge075.key -out edge075.csr \
  -subj "/CN=100.64.0.75"
```

Sign CSR offline, copy `edge075.crt` into `/etc/nats/edge075/certs/`, then:

```bash
sudo systemctl restart nats-edge075
```

---

## 5. Verify

### From a developer (Alice)

Subscribe:

```bash
nats --server wss://100.64.0.10:8443 \
  --tlscert alice.crt --tlskey alice.key --tlsca ca.crt \
  --user alice --password <pw> \
  sub "dummy.test"
```

Publish:

```bash
nats --server wss://100.64.0.10:8443 ... pub "dummy.test" "hello"
```

### From edge ↔ hub

On hub:

```bash
nats --server nats://127.0.0.1:4222 sub "test.edge"
```

On edge:

```bash
nats --server nats://127.0.0.1:4222 pub "test.edge" "hello from edge"
```

---

## Notes

* Hub runs as `nats` user, not root.
* Sampler and archiver run as separate non-root users.
* CA key never lives on hub or edge.
* Each edge and developer has isolated certs + accounts.
* Access revoked by removing accounts and restarting hub.
* Tailscale provides private networking, TLS adds trust and identity.

