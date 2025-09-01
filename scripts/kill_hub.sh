#!/usr/bin/env bash
set -euo pipefail

echo "[hub] stopping service..."
systemctl stop nats-hub 2>/dev/null || true
systemctl disable nats-hub 2>/dev/null || true
rm -f /etc/systemd/system/nats-hub.service
systemctl daemon-reload

echo "[hub] removing configs and data..."
rm -rf /etc/nats/hub /var/lib/nats/hub

# If no edges or hub remain, remove system user
if ! ls /etc/nats/edge* /etc/nats/hub 1>/dev/null 2>&1; then
  echo "[hub] no hub/edges found, removing system users..."
  systemctl stop nats-* sampler-* archiver-* 2>/dev/null || true
  userdel -r nats 2>/dev/null || true
  userdel -r sampler 2>/dev/null || true
  userdel -r archiver 2>/dev/null || true
fi

echo "[hub] fully removed. You can now run hub_install.sh again for a clean setup."
