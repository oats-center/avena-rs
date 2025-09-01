#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   sudo HUB_HOST=hub.example.com ./hub_install.sh

NATS_BIN="${NATS_BIN:-/usr/local/bin/nats-server}"
[[ -x "$NATS_BIN" ]] || { echo "nats-server not found"; exit 1; }

HUB_HOST="${HUB_HOST:?set HUB_HOST like hub.example.com}"

# Create nats user
id -u nats &>/dev/null || useradd -r -s /sbin/nologin nats

BASE=/etc/nats/hub
mkdir -p "$BASE/certs" "$BASE/auth" /var/lib/nats/hub
chown -R nats:nats /var/lib/nats/hub
chmod 700 "$BASE/certs" "$BASE/auth"

# Expect hub cert and key signed by offline CA already copied into $BASE/certs
if [[ ! -f "$BASE/certs/hub.crt" || ! -f "$BASE/certs/hub.key" || ! -f "$BASE/certs/ca.crt" ]]; then
  echo "[error] Missing hub.crt, hub.key, or ca.crt in $BASE/certs"
  echo "Please generate CSR, sign offline, and copy them here."
  exit 1
fi
chown nats:nats "$BASE/certs/"*

: > "$BASE/auth/read.conf"
: > "$BASE/auth/cfg.conf"
chmod 600 "$BASE/auth/"*

cat >"$BASE/server.conf" <<EOF
port: 4222
host: "127.0.0.1"
server_name: "hub"

websocket {
  port: 8443
  tls {
    cert_file: "$BASE/certs/hub.crt"
    key_file: "$BASE/certs/hub.key"
    ca_file: "$BASE/certs/ca.crt"
    verify_and_map: true
  }
}

leafnodes {
  listen: 7422
  tls {
    cert_file: "$BASE/certs/hub.crt"
    key_file: "$BASE/certs/hub.key"
    ca_file: "$BASE/certs/ca.crt"
    verify_and_map: true
  }
  authorization {
    users: []
  }
}

authorization {
  users: [
    \$include "$BASE/auth/read.conf",
    \$include "$BASE/auth/cfg.conf"
  ]
}
EOF

cat >/etc/systemd/system/nats-hub.service <<EOF
[Unit]
Description=NATS Hub
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

systemctl daemon-reload
systemctl enable --now nats-hub

echo "[hub] Hub installed. Certs in $BASE/certs, auth in $BASE/auth."
