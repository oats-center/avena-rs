#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${BIN_DIR:-$ROOT_DIR/target/release}"
PID_DIR="${PID_DIR:-$ROOT_DIR/.pids}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/logs}"
ENV_SETUP="${ENV_SETUP:-$ROOT_DIR/env-setup.sh}"

if [[ -f "$ENV_SETUP" ]]; then
  # shellcheck disable=SC1090
  source "$ENV_SETUP"
fi

mkdir -p "$PID_DIR" "$LOG_DIR"

DEFAULT_BINS=("streamer" "archiver" "exporter")
ROLE="${ROLE:-}"

usage() {
  cat <<'EOF'
Usage: ./deploy-binary.sh <start|stop|restart|status|build> [bin...]

Defaults to: streamer archiver exporter
Set ROLE=edge to run streamer only.
Set ROLE=server to run archiver + exporter.
Set INCLUDE_SUBSCRIBER=1 to include subscriber by default.
Instance names are supported with the @ suffix (e.g. archiver@edge01).
Set BIN_DIR/LOG_DIR/PID_DIR/ENV_SETUP to override paths.
EOF
}

resolve_bins() {
  local -a bins
  if [[ "$#" -gt 0 ]]; then
    if [[ "$1" == "all" ]]; then
      bins=("streamer" "archiver" "exporter" "subscriber")
    else
      bins=("$@")
    fi
  else
    case "${ROLE}" in
      edge)
        bins=("streamer")
        ;;
      server)
        bins=("archiver" "exporter")
        ;;
      ""|*)
        bins=("${DEFAULT_BINS[@]}")
        ;;
    esac
    if [[ "${INCLUDE_SUBSCRIBER:-0}" == "1" ]]; then
      bins+=("subscriber")
    fi
  fi
  echo "${bins[@]}"
}

pidfile_for() {
  local token="$1"
  echo "$PID_DIR/${token}.pid"
}

is_running_pid() {
  local pid="$1"
  [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

matches_bin() {
  local pid="$1"
  local bin="$2"
  local instance="${3:-}"
  local cmd
  cmd="$(ps -p "$pid" -o args= 2>/dev/null || true)"
  if [[ -n "$instance" ]]; then
    [[ "$cmd" == *"INSTANCE=$instance"* && "$cmd" == *"$BIN_DIR/$bin"* ]]
  else
    [[ "$cmd" == *"$BIN_DIR/$bin"* ]]
  fi
}

parse_token() {
  local token="$1"
  local bin="$token"
  local instance=""
  if [[ "$token" == *"@"* ]]; then
    bin="${token%@*}"
    instance="${token#*@}"
  fi
  echo "$bin" "$instance"
}

get_running_pid() {
  local token="$1"
  local bin instance
  read -r bin instance < <(parse_token "$token")
  local pidfile
  pidfile="$(pidfile_for "$token")"

  if [[ -f "$pidfile" ]]; then
    local pid
    pid="$(cat "$pidfile" 2>/dev/null || true)"
    if is_running_pid "$pid" && matches_bin "$pid" "$bin" "$instance"; then
      echo "$pid"
      return 0
    fi
    rm -f "$pidfile"
  fi

  local pids
  if [[ -n "$instance" ]]; then
    pids="$(pgrep -f "INSTANCE=${instance} .*${BIN_DIR}/${bin}" 2>/dev/null || true)"
  else
    pids="$(pgrep -f "$BIN_DIR/$bin" 2>/dev/null || true)"
  fi
  if [[ -n "$pids" ]]; then
    echo "$pids" | head -n1
    return 0
  fi
  return 1
}

ensure_built() {
  local missing=0
  for token in "$@"; do
    local bin instance
    read -r bin instance < <(parse_token "$token")
    if [[ ! -x "$BIN_DIR/$bin" ]]; then
      missing=1
      break
    fi
  done
  if [[ "$missing" -eq 1 ]]; then
    echo ">>> Building Rust binaries (missing in $BIN_DIR)..."
    (cd "$ROOT_DIR" && cargo build --release)
  fi
}

start_one() {
  local token="$1"
  local bin instance
  read -r bin instance < <(parse_token "$token")
  if get_running_pid "$token" >/dev/null; then
    echo ">>> $token already running (pid $(get_running_pid "$token"))."
    return 0
  fi
  if [[ ! -x "$BIN_DIR/$bin" ]]; then
    echo ">>> $bin not found at $BIN_DIR/$bin"
    return 1
  fi
  echo ">>> Starting $token..."
  if [[ -n "$instance" ]]; then
    nohup env INSTANCE="$instance" "$BIN_DIR/$bin" >> "$LOG_DIR/$token.log" 2>&1 &
  else
    nohup "$BIN_DIR/$bin" >> "$LOG_DIR/$bin.log" 2>&1 &
  fi
  echo $! > "$(pidfile_for "$token")"
}

stop_one() {
  local token="$1"
  local pid
  if ! pid="$(get_running_pid "$token")"; then
    echo ">>> $token not running."
    return 0
  fi
  echo ">>> Stopping $token (pid $pid)..."
  kill "$pid" 2>/dev/null || true
  for _ in {1..20}; do
    if ! is_running_pid "$pid"; then
      rm -f "$(pidfile_for "$token")"
      echo ">>> $token stopped."
      return 0
    fi
    sleep 0.5
  done
  echo ">>> $token did not stop gracefully, sending SIGKILL..."
  kill -9 "$pid" 2>/dev/null || true
  rm -f "$(pidfile_for "$token")"
}

status_one() {
  local token="$1"
  if pid="$(get_running_pid "$token")"; then
    echo ">>> $token running (pid $pid)"
  else
    echo ">>> $token stopped"
  fi
}

cmd="${1:-}"
shift || true

case "$cmd" in
  start)
    bins=($(resolve_bins "$@"))
    ensure_built "${bins[@]}"
    for bin in "${bins[@]}"; do
      start_one "$bin"
    done
    ;;
  stop)
    bins=($(resolve_bins "$@"))
    for bin in "${bins[@]}"; do
      stop_one "$bin"
    done
    ;;
  restart)
    bins=($(resolve_bins "$@"))
    for bin in "${bins[@]}"; do
      stop_one "$bin"
    done
    ensure_built "${bins[@]}"
    for bin in "${bins[@]}"; do
      start_one "$bin"
    done
    ;;
  status)
    if [[ "$#" -gt 0 ]]; then
      bins=($(resolve_bins "$@"))
    else
      mapfile -t bins < <(ls -1 "$PID_DIR"/*.pid 2>/dev/null | xargs -n1 basename 2>/dev/null | sed 's/\.pid$//' || true)
      if [[ "${#bins[@]}" -eq 0 ]]; then
        bins=($(resolve_bins))
      fi
    fi
    for bin in "${bins[@]}"; do
      status_one "$bin"
    done
    ;;
  build)
    bins=($(resolve_bins "$@"))
    ensure_built "${bins[@]}"
    ;;
  *)
    usage
    exit 1
    ;;
esac
