#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   sudo EDGE_ID=075 HUB_HOST=hub.example.com ./edge_install.sh

NATS_BIN="${NATS_BIN:-/usr/local/bin/nats-server}"
[[ -x "$NATS_BIN" ]] || { echo "nats-server not found"; exit 1; }

EDGE_ID="${EDGE_ID:?set EDGE_ID like 075}"
HUB_HOST="${HUB_HOST:?set HUB_HOST}"

id -u nats &>/dev/null    || useradd -r -s /sbin/nologin nats
id -u sampler &>/dev/null || useradd -r -s /sbin/nologin sampler
id -u archiver &>/dev/null|| useradd -r -s /sbin/nologin archiver

BASE="/etc/nats/edge${EDGE_ID}"
JS_DIR="/var/lib/nats/edge${EDGE_ID}/jetstream"
PARQ_DIR="/var/lib/edge${EDGE_ID}/parquet"

mkdir -p "$BASE/certs" "$BASE" "$JS_DIR" "$PARQ_DIR"
chown -R nats:nats "$JS_DIR"
chown -R archiver:archiver "$PARQ_DIR"
chmod 700 "$BASE/certs" "$JS_DIR" "$PARQ_DIR"

# Expect edge cert and key signed by offline CA already copied into $BASE/certs
if [[ ! -f "$BASE/certs/edge${EDGE_ID}.crt" || ! -f "$BASE/certs/edge${EDGE_ID}.key" || ! -f "$BASE/certs/ca.crt" ]]; then
  echo "[error] Missing edge cert/key or ca.crt in $BASE/certs"
  echo "Please generate CSR, sign offline, and copy them here."
  exit 1
fi
chown nats:nats "$BASE/certs/"*

cat >"$BASE/server.conf" <<EOF
port: 4222
host: "127.0.0.1"
server_name: "edge${EDGE_ID}"

jetstream {
  store_dir: "$JS_DIR"
  max_file: "50GB"
}

leafnodes {
  remotes: [
    {
      url: "tls://${HUB_HOST}:7422"
      tls {
        cert_file: "$BASE/certs/edge${EDGE_ID}.crt"
        key_file: "$BASE/certs/edge${EDGE_ID}.key"
        ca_file: "$BASE/certs/ca.crt"
      }
    }
  ]
}
EOF

# Edge NATS service
cat >"/etc/systemd/system/nats-edge${EDGE_ID}.service" <<EOF
[Unit]
Description=NATS Edge ${EDGE_ID}
After=network-online.target
Wants=network-online.target

[Service]
User=nats
Group=nats
ExecStart=${NATS_BIN} -c $BASE/server.conf
Restart=on-failure
LimitNOFILE=262144

[Install]
WantedBy=multi-user.target
EOF

# Sampler service (Rust streamer)
SAMPLER_BIN="${SAMPLER_BIN:-/usr/local/bin/labjack-sampler}"
cat >"/etc/systemd/system/sampler-${EDGE_ID}.service" <<EOF
[Unit]
Description=LabJack Sampler (Edge ${EDGE_ID})
After=nats-edge${EDGE_ID}.service
Wants=nats-edge${EDGE_ID}.service

[Service]
User=sampler
Group=sampler
Environment="NATS_URL=nats://127.0.0.1:4222"
Environment="CFG_BUCKET=sampler_cfg"
Environment="CFG_KEY=edge${EDGE_ID}"
Environment="PARQUET_DIR=${PARQ_DIR}"
ExecStart=${SAMPLER_BIN}
Restart=on-failure
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
CapabilityBoundingSet=
RestrictSUIDSGID=yes

[Install]
WantedBy=multi-user.target
EOF

# Archiver service
ARCHIVER_BIN="${ARCHIVER_BIN:-/usr/local/bin/parquet-archiver}"
cat >"/etc/systemd/system/archiver-${EDGE_ID}.service" <<EOF
[Unit]
Description=Parquet Archiver (Edge ${EDGE_ID})
After=nats-edge${EDGE_ID}.service
Wants=nats-edge${EDGE_ID}.service

[Service]
User=archiver
Group=archiver
WorkingDirectory=${PARQ_DIR}
ExecStart=${ARCHIVER_BIN}
Restart=on-failure
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
CapabilityBoundingSet=
RestrictSUIDSGID=yes

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now "nats-edge${EDGE_ID}"

if [[ -x "$SAMPLER_BIN" ]]; then
  systemctl enable --now "sampler-${EDGE_ID}"
  echo "[edge ${EDGE_ID}] sampler started."
else
  echo "[edge ${EDGE_ID}] sampler binary not found at ${SAMPLER_BIN}; service installed but not started."
fi

if [[ -x "$ARCHIVER_BIN" ]]; then
  systemctl enable --now "archiver-${EDGE_ID}"
  echo "[edge ${EDGE_ID}] archiver started."
else
  echo "[edge ${EDGE_ID}] archiver binary not found at ${ARCHIVER_BIN}; service installed but not started."
fi

echo "[edge ${EDGE_ID}] NATS running. Sampler/Archiver isolated under dedicated users."
