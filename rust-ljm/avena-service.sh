#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat <<'EOF'
Usage:
  ./avena-service.sh <streamer|archiver|exporter> <start|stop|restart|status|logs>

This runs the Rust services under the user systemd manager:
  avena-streamer.service
  avena-archiver.service
  avena-exporter.service
EOF
}

if [[ $# -ne 2 ]]; then
  usage
  exit 2
fi

APP_NAME="$1"
ACTION="$2"

case "$APP_NAME" in
  streamer|archiver|exporter) ;;
  *)
    usage
    exit 2
    ;;
esac

case "$ACTION" in
  start|stop|restart|status|logs) ;;
  *)
    usage
    exit 2
    ;;
esac

UNIT="avena-${APP_NAME}.service"
BIN_PATH="$ROOT_DIR/target/release/$APP_NAME"
CONFIG_FILE="$ROOT_DIR/${APP_NAME}.env.json"

start_service() {
  if systemctl --user is-active --quiet "$UNIT"; then
    echo "$UNIT is already running"
    return 0
  fi

  if [[ ! -x "$BIN_PATH" ]]; then
    echo "Missing executable: $BIN_PATH" >&2
    echo "Build first: cargo build --release --bin $APP_NAME" >&2
    exit 1
  fi

  if [[ ! -s "$CONFIG_FILE" ]]; then
    echo "Missing env config: $CONFIG_FILE" >&2
    echo "Render first: cd /home/user/avena-rs && ./shared/render-edge-config.py" >&2
    exit 1
  fi

  mapfile -d '' env_args < <(
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
    sys.stdout.buffer.write(f"--setenv={key}={value}".encode("utf-8"))
    sys.stdout.buffer.write(b"\0")
PY
  )

  systemd-run --user \
    --unit="$UNIT" \
    --collect \
    --property=Restart=on-failure \
    --property=RestartSec=2 \
    --working-directory="$ROOT_DIR" \
    "${env_args[@]}" \
    "$BIN_PATH"
}

case "$ACTION" in
  start)
    start_service
    ;;
  stop)
    systemctl --user stop "$UNIT" || true
    ;;
  restart)
    systemctl --user stop "$UNIT" || true
    systemctl --user reset-failed "$UNIT" >/dev/null 2>&1 || true
    start_service
    ;;
  status)
    systemctl --user --no-pager --full status "$UNIT" || true
    ;;
  logs)
    journalctl --user -u "$UNIT" -f
    ;;
esac
