#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   sudo HUB_WS_HOST=0.0.0.0 HUB_LEAF_HOST=0.0.0.0 \
#        LEAF_USER=leaf LEAF_PASS='changeme-leaf' \
#        ./linux_hub_setup.sh
#
# Notes:
# - Hub TCP client port (4222) binds to 127.0.0.1 (not exposed)
# - WebSocket listens on HUB_WS_HOST:8080
# - Leafnode listens on HUB_LEAF_HOST:7422
# - Per-user accounts are managed in:
#     /etc/nats/hub/auth_read.conf  (readers)
#     /etc/nats/hub/auth_cfg.conf   (config writers)

NATS_BIN="${NATS_BIN:-/usr/local/bin/nats-server}"
[[ -x "$NATS_BIN" ]] || { echo "NATS server not found at $NATS_BIN"; exit 1; }

HUB_WS_HOST="${HUB_WS_HOST:-0.0.0.0}"
HUB_LEAF_HOST="${HUB_LEAF_HOST:-0.0.0.0}"
LEAF_USER="${LEAF_USER:-leaf}"
LEAF_PASS="${LEAF_PASS:-$(tr -dc 'A-Za-z0-9' </dev/urandom | head -c 20)}"

echo "[hub] WS host=${HUB_WS_HOST}  leaf host=${HUB_LEAF_HOST}"
echo "[hub] leaf creds: user=${LEAF_USER} pass=${LEAF_PASS}"

mkdir -p /etc/nats/hub
chmod 755 /etc/nats /etc/nats/hub
: > /etc/nats/hub/auth_read.conf
: > /etc/nats/hub/auth_cfg.conf
chmod 600 /etc/nats/hub/auth_*.conf

cat >/etc/nats/hub/server.conf <<EOF
port: 4222
host: "127.0.0.1"
server_name: "hub"

websocket {
  port: 8080
  host: "${HUB_WS_HOST}"
}

leafnodes {
  listen: 7422
  host: "${HUB_LEAF_HOST}"
  authorization {
    users: [
      { user: "${LEAF_USER}", pass: "${LEAF_PASS}" }
    ]
  }
}

authorization {
  users: [
    \$include /etc/nats/hub/auth_read.conf,
    \$include /etc/nats/hub/auth_cfg.conf
  ]
}
EOF

cat >/etc/systemd/system/nats-hub.service <<EOF
[Unit]
Description=NATS Hub (WS + leafnodes; TCP client is localhost-only)
After=network-online.target
Wants=network-online.target

[Service]
User=root
Group=root
ExecStart=${NATS_BIN} -c /etc/nats/hub/server.conf
Restart=on-failure
LimitNOFILE=262144

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now nats-hub

echo
echo "[hub] Up. Manage users by editing:"
echo "       /etc/nats/hub/auth_read.conf  (subscribe labjack.>.data.>)"
echo "       /etc/nats/hub/auth_cfg.conf   (publish KV.sampler_cfg.edgeXXX)"
echo "       then: systemctl restart nats-hub"
echo
echo "[hub] Leaf creds for edges: ${LEAF_USER} / ${LEAF_PASS}"
