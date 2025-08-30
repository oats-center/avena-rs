#!/usr/bin/env bash
set -euo pipefail
EDGE_ID="${EDGE_ID:?set EDGE_ID like 075}"

systemctl stop "sampler-${EDGE_ID}" 2>/dev/null || true
systemctl disable "sampler-${EDGE_ID}" 2>/dev/null || true
rm -f "/etc/systemd/system/sampler-${EDGE_ID}.service"

systemctl stop "nats-edge${EDGE_ID}" 2>/dev/null || true
systemctl disable "nats-edge${EDGE_ID}" 2>/dev/null || true
rm -f "/etc/systemd/system/nats-edge${EDGE_ID}.service"
systemctl daemon-reload

rm -rf "/etc/nats/edge${EDGE_ID}" "/var/lib/nats/edge${EDGE_ID}" "/var/lib/edge${EDGE_ID}/parquet"
echo "[edge ${EDGE_ID}] removed."
