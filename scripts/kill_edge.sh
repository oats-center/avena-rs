#!/usr/bin/env bash
set -euo pipefail
EDGE_ID="${EDGE_ID:?set EDGE_ID like 075}"

echo "[edge ${EDGE_ID}] stopping services..."
systemctl stop "sampler-${EDGE_ID}" 2>/dev/null || true
systemctl disable "sampler-${EDGE_ID}" 2>/dev/null || true
rm -f "/etc/systemd/system/sampler-${EDGE_ID}.service"

systemctl stop "archiver-${EDGE_ID}" 2>/dev/null || true
systemctl disable "archiver-${EDGE_ID}" 2>/dev/null || true
rm -f "/etc/systemd/system/archiver-${EDGE_ID}.service"

systemctl stop "nats-edge${EDGE_ID}" 2>/dev/null || true
systemctl disable "nats-edge${EDGE_ID}" 2>/dev/null || true
rm -f "/etc/systemd/system/nats-edge${EDGE_ID}.service"

systemctl daemon-reload

echo "[edge ${EDGE_ID}] removing configs and data..."
rm -rf "/etc/nats/edge${EDGE_ID}" "/var/lib/nats/edge${EDGE_ID}" "/var/lib/edge${EDGE_ID}/parquet"

# If this was the last edge, remove system users too
if ! ls /etc/nats/edge* &>/dev/null; then
  echo "[edge ${EDGE_ID}] no other edges found, removing system users..."
  systemctl stop nats-* sampler-* archiver-* 2>/dev/null || true

  userdel -r nats 2>/dev/null || true
  userdel -r sampler 2>/dev/null || true
  userdel -r archiver 2>/dev/null || true
fi

echo "[edge ${EDGE_ID}] fully removed. You can now run edge_install.sh again for a clean setup."
