#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   sudo EDGE_ID=075 HUB_HOST=100.64.0.76 \
#        LEAF_USER=leaf LEAF_PASS='changeme-leaf' \
#        ./linux_edge_setup.sh
#
# - Client :4222 binds 127.0.0.1 (only local sampler connects)
# - JetStream enabled (50GB cap at server level)
# - Leafnode connects to HUB_HOST:7422

NATS_BIN="${NATS_BIN:-/usr/local/bin/nats-server}"
[[ -x "$NATS_BIN" ]] || { echo "NATS server not found at $NATS_BIN"; exit 1; }

EDGE_ID="${EDGE_ID:?set EDGE_ID like 075}"
HUB_HOST="${HUB_HOST:?set HUB_HOST ip/host}"
LEAF_USER="${LEAF_USER:?set LEAF_USER}"
LEAF_PASS="${LEAF_PASS:?set LEAF_PASS}"

BASE="/etc/nats/edge${EDGE_ID}"
JS_DIR="/var/lib/nats/edge${EDGE_ID}/jetstream"
PARQ_DIR="/var/lib/edge${EDGE_ID}/parquet"

mkdir -p "${BASE}/creds" "${JS_DIR}" "${PARQ_DIR}"
chmod 700 "${BASE}/creds" "${JS_DIR}" "${PARQ_DIR}"

cat >"${BASE}/server.conf" <<EOF
port: 4222
host: "127.0.0.1"
server_name: "edge${EDGE_ID}"

jetstream: { store_dir: "${JS_DIR}", max_file: "50GB" }

leafnodes {
  remotes: [
    {
      url: "nats://${HUB_HOST}:7422",
      authorization: { user: "${LEAF_USER}", pass: "${LEAF_PASS}" }
    }
  ]
}
EOF

cat >"/etc/systemd/system/nats-edge${EDGE_ID}.service" <<EOF
[Unit]
Description=NATS Edge ${EDGE_ID} (JS; local-only client; leaf to hub)
After=network-online.target
Wants=network-online.target

[Service]
User=root
Group=root
ExecStart=${NATS_BIN} -c ${BASE}/server.conf
Restart=on-failure
LimitNOFILE=262144

[Install]
WantedBy=multi-user.target
EOF

# Optional sampler service (starts only if binary exists)
SAMPLER_BIN="${SAMPLER_BIN:-/usr/local/bin/labjack-sampler}"
cat >"/etc/systemd/system/sampler-${EDGE_ID}.service" <<EOF
[Unit]
Description=LabJack Sampler (Edge ${EDGE_ID})
After=nats-edge${EDGE_ID}.service
Wants=nats-edge${EDGE_ID}.service

[Service]
User=root
Group=root
Environment="NATS_URL=nats://127.0.0.1:4222"
Environment="CFG_BUCKET=sampler_cfg"
Environment="CFG_KEY=edge${EDGE_ID}"
Environment="PARQUET_DIR=${PARQ_DIR}"
ExecStart=${SAMPLER_BIN}
Restart=on-failure
NoNewPrivileges=yes

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

echo "[edge ${EDGE_ID}] NATS running. Client :4222 on localhost; leaf -> ${HUB_HOST}:7422."
