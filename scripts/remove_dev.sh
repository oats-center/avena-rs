#!/usr/bin/env bash
set -euo pipefail

USER_NAME="${1:?username}"

BASE=/etc/nats/hub
sed -i "/user: \"$USER_NAME\"/d" "$BASE/auth/read.conf" "$BASE/auth/cfg.conf"
rm -rf "$BASE/devs/$USER_NAME"

systemctl restart nats-hub
echo "[dev] $USER_NAME removed."
