#!/usr/bin/env bash
set -euo pipefail
systemctl stop nats-hub || true
systemctl disable nats-hub || true
rm -f /etc/systemd/system/nats-hub.service
systemctl daemon-reload
rm -rf /etc/nats/hub
echo "[hub] removed."
