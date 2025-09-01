#!/usr/bin/env bash
set -euo pipefail

USER_NAME="${1:?username}"
ROLE="${2:?reader|cfg}"

BASE=/etc/nats/hub
DEV_DIR="$BASE/devs/$USER_NAME"

[[ -f "$DEV_DIR/$USER_NAME.crt" ]] || { echo "Missing signed cert"; exit 1; }

read -s -p "Enter password for $USER_NAME: " PASSWORD
echo
HASH=$(htpasswd -bnBC 12 "" "$PASSWORD" | tr -d ':\n')

case "$ROLE" in
  reader)
    echo "{ user: \"$USER_NAME\", password: \"$HASH\" }" >> "$BASE/auth/read.conf"
    ;;
  cfg)
    echo "{ user: \"$USER_NAME\", password: \"$HASH\" }" >> "$BASE/auth/cfg.conf"
    ;;
  *)
    echo "Invalid role"; exit 1;;
esac

systemctl restart nats-hub
echo "[dev] $USER_NAME added with role $ROLE."
