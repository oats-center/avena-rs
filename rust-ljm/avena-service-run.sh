#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="${1:-}"

case "$APP_NAME" in
  streamer|archiver|exporter) ;;
  *)
    echo "Usage: $0 <streamer|archiver|exporter>" >&2
    exit 2
    ;;
esac

CONFIG_DIR="${AVENA_CONFIG_DIR:-$ROOT_DIR}"
CONFIG_FILE="$CONFIG_DIR/${APP_NAME}.env.json"
if [[ -x "$ROOT_DIR/$APP_NAME" ]]; then
  BIN_PATH="$ROOT_DIR/$APP_NAME"
else
  BIN_PATH="$ROOT_DIR/target/release/$APP_NAME"
fi

if [[ ! -x "$BIN_PATH" ]]; then
  echo "Missing executable: $BIN_PATH" >&2
  exit 1
fi

if [[ ! -s "$CONFIG_FILE" ]]; then
  echo "Missing environment config: $CONFIG_FILE" >&2
  exit 1
fi

mapfile -d '' env_values < <(
  python3 - "$CONFIG_FILE" "$ROOT_DIR" <<'PY'
import json
import os
import sys

config_path = sys.argv[1]
root_dir = sys.argv[2]
path_keys = {"NATS_CREDS_FILE", "OUTPUT_DIR", "PARQUET_DIR"}

with open(config_path, "r", encoding="utf-8") as fh:
    env = json.load(fh)["env"]

for key in sorted(env):
    value = "" if env[key] is None else str(env[key])
    if key in path_keys and value and not os.path.isabs(value):
        value = os.path.abspath(os.path.join(root_dir, value))
    sys.stdout.buffer.write(f"{key}={value}".encode("utf-8"))
    sys.stdout.buffer.write(b"\0")
PY
)

exec env "${env_values[@]}" "$BIN_PATH"
