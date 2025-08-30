#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   sudo EDGE_ID=075 HUB_HOST=100.64.0.76 \
#        LEAF_USER=leaf LEAF_PASS='changeme-leaf' \
#        ./mac_edge_setup.sh

NATS_BIN="${NATS_BIN:-/opt/homebrew/bin/nats-server}"
[[ -x "$NATS_BIN" ]] || { echo "NATS server not found at $NATS_BIN"; exit 1; }

EDGE_ID="${EDGE_ID:?set EDGE_ID like 075}"
HUB_HOST="${HUB_HOST:?set HUB_HOST}"
LEAF_USER="${LEAF_USER:?set LEAF_USER}"
LEAF_PASS="${LEAF_PASS:?set LEAF_PASS}"

BASE="/etc/nats/edge${EDGE_ID}"
JS_DIR="/var/lib/nats/edge${EDGE_ID}/jetstream"
PARQ_DIR="/var/lib/edge${EDGE_ID}/parquet"

mkdir -p "$BASE/creds" "$JS_DIR" "$PARQ_DIR"
chmod 700 "$BASE/creds" "$JS_DIR" "$PARQ_DIR"

cat >"${BASE}/server.conf" <<EOF
port: 4222
host: "127.0.0.1"
server_name: "edge${EDGE_ID}"
jetstream: { store_dir: "${JS_DIR}", max_file: "50GB" }
leafnodes {
  remotes: [
    { url: "nats://${HUB_HOST}:7422",
      authorization: { user: "${LEAF_USER}", pass: "${LEAF_PASS}" } }
  ]
}
EOF

PLIST="/Library/LaunchDaemons/io.nats.edge${EDGE_ID}.plist"
cat >"$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>io.nats.edge${EDGE_ID}</string>
<key>ProgramArguments</key>
<array><string>${NATS_BIN}</string><string>-c</string><string>${BASE}/server.conf</string></array>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
<key>StandardOutPath</key><string>/var/log/nats-edge${EDGE_ID}.out</string>
<key>StandardErrorPath</key><string>/var/log/nats-edge${EDGE_ID}.err</string>
</dict></plist>
EOF

launchctl unload "$PLIST" >/dev/null 2>&1 || true
launchctl load "$PLIST"

# Optional sampler
SAMPLER_BIN="${SAMPLER_BIN:-/usr/local/bin/labjack-sampler}"
SP="/Library/LaunchDaemons/io.sampler.${EDGE_ID}.plist"
if [[ -x "$SAMPLER_BIN" ]]; then
cat >"$SP" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>io.sampler.${EDGE_ID}</string>
<key>ProgramArguments</key><array><string>${SAMPLER_BIN}</string></array>
<key>EnvironmentVariables</key>
<dict>
  <key>NATS_URL</key><string>nats://127.0.0.1:4222</string>
  <key>CFG_BUCKET</key><string>sampler_cfg</string>
  <key>CFG_KEY</key><string>edge${EDGE_ID}</string>
  <key>PARQUET_DIR</key><string>${PARQ_DIR}</string>
</dict>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
<key>StandardOutPath</key><string>/var/log/sampler-${EDGE_ID}.out</string>
<key>StandardErrorPath</key><string>/var/log/sampler-${EDGE_ID}.err</string>
</dict></plist>
EOF
  launchctl unload "$SP" >/dev/null 2>&1 || true
  launchctl load "$SP"
fi

echo "[edge ${EDGE_ID}] mac edge ready; leaf-> ${HUB_HOST}:7422; client on localhost."
