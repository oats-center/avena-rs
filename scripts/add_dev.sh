#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   sudo ./add_dev.sh alice reader|cfg

USER_NAME="${1:?dev username}"
ROLE="${2:?reader|cfg}"

BASE=/etc/nats/hub
DEV_DIR="$BASE/devs/$USER_NAME"
mkdir -p "$DEV_DIR"

echo "[info] Generate CSR for $USER_NAME. Sign offline with hub CA."
openssl genrsa -out "$DEV_DIR/$USER_NAME.key" 2048
openssl req -new -key "$DEV_DIR/$USER_NAME.key" -subj "/CN=$USER_NAME" -out "$DEV_DIR/$USER_NAME.csr"

echo "Now sign $DEV_DIR/$USER_NAME.csr with your offline CA and save as $DEV_DIR/$USER_NAME.crt"
echo "Then run ./finish_add_dev.sh $USER_NAME $ROLE"
