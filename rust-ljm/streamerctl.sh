#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="${CONFIG_FILE:-$ROOT_DIR/streamer.env.json}"
BIN_PATH="${BIN_PATH:-$ROOT_DIR/target/release/streamer}"
RUNTIME_DIR="${RUNTIME_DIR:-$ROOT_DIR/.runtime}"
PID_FILE="${PID_FILE:-$RUNTIME_DIR/streamer.pid}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/logs}"
LOG_FILE="${LOG_FILE:-$LOG_DIR/streamer.log}"

usage() {
  cat <<'EOF'
Usage: ./streamerctl.sh <start|stop|restart|status|build>

Configuration is loaded from CONFIG_FILE or:
  ./streamer.env.json

Commands:
  start    Build if needed, load env from JSON, and start one streamer process
  stop     Stop the running streamer process
  restart  Stop, then start with the current JSON config
  status   Show whether streamer is running
  build    Build the release streamer binary
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
  if [[ ! -x "$BIN_PATH" ]]; then
    echo ">>> Building release streamer binary"
    (cd "$ROOT_DIR" && cargo build --release --bin streamer)
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

start_streamer() {
  ensure_dirs
  if pid="$(get_running_pid)"; then
    echo ">>> streamer already running (pid $pid)"
    return 0
  fi

  ensure_built
  load_env_from_config

  echo ">>> Starting streamer"
  nohup "$BIN_PATH" >> "$LOG_FILE" 2>&1 &
  echo "$!" > "$PID_FILE"
  echo ">>> streamer started (pid $!)"
}

stop_streamer() {
  ensure_dirs
  local pid
  if ! pid="$(get_running_pid)"; then
    echo ">>> streamer not running"
    return 0
  fi

  echo ">>> Stopping streamer (pid $pid)"
  kill "$pid" 2>/dev/null || true
  for _ in {1..20}; do
    if ! kill -0 "$pid" 2>/dev/null; then
      rm -f "$PID_FILE"
      echo ">>> streamer stopped"
      return 0
    fi
    sleep 0.5
  done

  echo ">>> streamer did not stop gracefully, sending SIGKILL"
  kill -9 "$pid" 2>/dev/null || true
  rm -f "$PID_FILE"
}

status_streamer() {
  ensure_dirs
  if pid="$(get_running_pid)"; then
    echo ">>> streamer running (pid $pid)"
    echo ">>> config: $CONFIG_FILE"
    echo ">>> log: $LOG_FILE"
  else
    echo ">>> streamer stopped"
    echo ">>> config: $CONFIG_FILE"
  fi
}

cmd="${1:-}"

case "$cmd" in
  start)
    start_streamer
    ;;
  stop)
    stop_streamer
    ;;
  restart)
    stop_streamer
    start_streamer
    ;;
  status)
    status_streamer
    ;;
  build)
    ensure_built
    ;;
  *)
    usage
    exit 1
    ;;
esac
