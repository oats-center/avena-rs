#!/usr/bin/env bash
set -euo pipefail

usage_for() {
  cat <<EOF
Usage: ./${APP_NAME}ctl.sh <start|stop|restart|status|build>

Configuration is loaded from CONFIG_FILE or:
  ./${APP_NAME}.env.json

Commands:
  start    Build if needed, load env from JSON, and start one ${APP_NAME} process
  stop     Stop the running ${APP_NAME} process
  restart  Stop, then start with the current JSON config
  status   Show whether ${APP_NAME} is running
  build    Build the release ${APP_NAME} binary
EOF
}

require_python() {
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 is required to read $CONFIG_FILE" >&2
    exit 1
  fi
}

ensure_dirs() {
  mkdir -p "$RUNTIME_DIR" "$LOG_DIR"
}

get_running_pid() {
  if [[ -f "$PID_FILE" ]]; then
    local pid
    pid="$(cat "$PID_FILE" 2>/dev/null || true)"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      echo "$pid"
      return 0
    fi
    rm -f "$PID_FILE"
  fi

  local pids
  pids="$(pgrep -f "$BIN_PATH" 2>/dev/null || true)"
  if [[ -n "$pids" ]]; then
    echo "$pids" | head -n1
    return 0
  fi

  return 1
}

ensure_built() {
  local needs_build=0

  if [[ ! -x "$BIN_PATH" ]]; then
    needs_build=1
  elif find "$ROOT_DIR/src" "$ROOT_DIR/examples" "$ROOT_DIR/Cargo.toml" "$ROOT_DIR/Cargo.lock" -type f -newer "$BIN_PATH" -print -quit 2>/dev/null | grep -q .; then
    needs_build=1
  fi

  if [[ "$needs_build" -eq 1 ]]; then
    echo ">>> Building release ${APP_NAME} binary"
    (cd "$ROOT_DIR" && cargo build --release --bin "$BUILD_TARGET")
  fi
}

load_env_from_config() {
  require_python
  if [[ ! -f "$CONFIG_FILE" ]]; then
    echo "Missing config file: $CONFIG_FILE" >&2
    exit 1
  fi

  local shell_exports
  shell_exports="$(
    python3 - "$CONFIG_FILE" "$ROOT_DIR" <<'PY'
import json
import os
import platform
import shlex
import sys

config_path = os.path.abspath(sys.argv[1])
root_dir = os.path.abspath(sys.argv[2])
config_dir = os.path.dirname(config_path)

with open(config_path, "r", encoding="utf-8") as fh:
    raw = json.load(fh)

env = raw.get("env", raw)
if not isinstance(env, dict):
    raise SystemExit(f"{config_path}: expected an object or an 'env' object")

defaults = {
    "NATS_SUBJECT": "avenabox",
    "NATS_SERVERS": "nats://nats1.oats:4222,nats://nats2.oats:4222,nats://nats3.oats:4222",
    "ASSET_NUMBER": "1001",
    "OUTPUT_DIR": "outputs",
    "NATS_CREDS_FILE": os.path.join(root_dir, "apt.creds"),
    "CFG_BUCKET": "avenabox",
    "CFG_KEY": "labjackd.config.i69-mu1",
    "LABJACK_IDENTIFIER": "",
    "LABJACK_SERIAL": "",
    "LABJACK_NAME": "",
    "LABJACK_IP": "",
    "LABJACK_USB_ID": "ANY",
    "LABJACK_OPEN_ORDER": "ethernet,usb",
    "EXPORTER_HTTP_URL": "http://127.0.0.1:9001",
    "PARQUET_DIR": "parquet",
    "EXPORTER_ADDR": "0.0.0.0:9001",
}

resolved = defaults.copy()
for key, value in env.items():
    if value is None:
        resolved[key] = ""
    else:
        resolved[key] = str(value)

path_like_keys = {
    "NATS_CREDS_FILE",
    "LJM_LIB_FILE",
    "OUTPUT_DIR",
    "PARQUET_DIR",
}

for key in path_like_keys:
    value = resolved.get(key, "")
    if not value:
        continue
    if not os.path.isabs(value):
        resolved[key] = os.path.abspath(os.path.join(config_dir, value))

if not resolved.get("LJM_LIB_FILE"):
    system = platform.system()
    candidates = []
    if system == "Darwin":
        candidates = [
            "/usr/local/lib/libLabJackM.dylib",
            "/opt/homebrew/lib/libLabJackM.dylib",
            "/usr/local/lib/libLabJackM-1.dylib",
            "/usr/local/lib/libLabJackM-1.23.4.dylib",
        ]
    elif system == "Linux":
        candidates = [
            "/usr/local/lib/libLabJackM.so",
            "/usr/lib/libLabJackM.so",
        ]
    for candidate in candidates:
        if os.path.isfile(candidate):
            resolved["LJM_LIB_FILE"] = candidate
            break

if resolved.get("LJM_LIB_FILE") and os.path.isfile(resolved["LJM_LIB_FILE"]):
    lib_dir = os.path.dirname(resolved["LJM_LIB_FILE"])
    resolved["LJM_PATH"] = resolved["LJM_LIB_FILE"]
    if platform.system() == "Linux":
        current = os.environ.get("LD_LIBRARY_PATH", "")
        resolved["LD_LIBRARY_PATH"] = lib_dir if not current else f"{lib_dir}:{current}"
    elif platform.system() == "Darwin":
        current = os.environ.get("DYLD_FALLBACK_LIBRARY_PATH", "")
        resolved["DYLD_FALLBACK_LIBRARY_PATH"] = lib_dir if not current else f"{lib_dir}:{current}"

for key in sorted(resolved):
    print(f"export {key}={shlex.quote(resolved[key])}")
PY
  )"

  eval "$shell_exports"
}

start_managed_bin() {
  ensure_dirs
  if pid="$(get_running_pid)"; then
    echo ">>> ${APP_NAME} already running (pid $pid)"
    return 0
  fi

  ensure_built
  load_env_from_config

  echo ">>> Starting ${APP_NAME}"
  nohup "$BIN_PATH" >> "$LOG_FILE" 2>&1 &
  echo "$!" > "$PID_FILE"
  echo ">>> ${APP_NAME} started (pid $!)"
}

stop_managed_bin() {
  ensure_dirs
  local pid
  if ! pid="$(get_running_pid)"; then
    echo ">>> ${APP_NAME} not running"
    return 0
  fi

  echo ">>> Stopping ${APP_NAME} (pid $pid)"
  kill "$pid" 2>/dev/null || true
  for _ in {1..20}; do
    if ! kill -0 "$pid" 2>/dev/null; then
      rm -f "$PID_FILE"
      echo ">>> ${APP_NAME} stopped"
      return 0
    fi
    sleep 0.5
  done

  echo ">>> ${APP_NAME} did not stop gracefully, sending SIGKILL"
  kill -9 "$pid" 2>/dev/null || true
  rm -f "$PID_FILE"
}

status_managed_bin() {
  ensure_dirs
  if pid="$(get_running_pid)"; then
    echo ">>> ${APP_NAME} running (pid $pid)"
    echo ">>> config: $CONFIG_FILE"
    echo ">>> log: $LOG_FILE"
  else
    echo ">>> ${APP_NAME} stopped"
    echo ">>> config: $CONFIG_FILE"
  fi
}

run_control_cmd() {
  local cmd="${1:-}"

  case "$cmd" in
    start)
      start_managed_bin
      ;;
    stop)
      stop_managed_bin
      ;;
    restart)
      stop_managed_bin
      start_managed_bin
      ;;
    status)
      status_managed_bin
      ;;
    build)
      ensure_built
      ;;
    *)
      usage_for
      exit 1
      ;;
  esac
}
