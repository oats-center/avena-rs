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
Set ROLE=edge to run streamer + video-recorder.
Set VIDEO_MULTI_CAMERA_INSTANCES=cam11,cam10 to run streamer + one video-recorder per camera instance.
Set ROLE=server to run archiver + exporter + clip-worker.
Set INCLUDE_SUBSCRIBER=1 to include subscriber by default.
Instance names are supported with the @ suffix (e.g. archiver@edge01).
For video-recorder instances, use env overrides like VIDEO_SOURCE_URL_CAM11, VIDEO_ASSET_NUMBER_CAM11, VIDEO_CAMERA_ID_CAM11.
Set BIN_DIR/LOG_DIR/PID_DIR/ENV_SETUP to override paths.
EOF
}

resolve_bins() {
  local -a bins
  if [[ "$#" -gt 0 ]]; then
    if [[ "$1" == "all" ]]; then
      bins=("streamer" "archiver" "exporter" "video-recorder" "clip-worker" "subscriber")
    else
      bins=("$@")
    fi
  else
    case "${ROLE}" in
      edge)
        bins=("streamer")
        if [[ -n "${VIDEO_MULTI_CAMERA_INSTANCES:-}" ]]; then
          IFS=',' read -r -a camera_instances <<< "${VIDEO_MULTI_CAMERA_INSTANCES}"
          for instance in "${camera_instances[@]}"; do
            instance="$(echo "$instance" | xargs)"
            [[ -n "$instance" ]] && bins+=("video-recorder@${instance}")
          done
          if [[ "${#bins[@]}" -eq 1 ]]; then
            bins+=("video-recorder")
          fi
        else
          bins+=("video-recorder")
        fi
        ;;
      server)
        bins=("archiver" "exporter" "clip-worker")
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

instance_suffix() {
  local instance="$1"
  echo "$instance" | tr '[:lower:]-' '[:upper:]_' | tr -cd 'A-Z0-9_'
}

instance_override() {
  local key="$1"
  local instance="$2"
  local suffix
  suffix="$(instance_suffix "$instance")"
  local override_key="${key}_${suffix}"
  echo "${!override_key:-}"
}

ensure_labjack_runtime_lib() {
  local lib_file="${LJM_LIB_FILE:-}"
  if [[ -z "$lib_file" ]]; then
    return 0
  fi

  if [[ ! -f "$lib_file" ]]; then
    echo ">>> streamer runtime error: LJM_LIB_FILE does not exist: $lib_file"
    return 1
  fi

  local lib_name link_path
  lib_name="$(basename "$lib_file")"
  link_path="$ROOT_DIR/$lib_name"

  if [[ -L "$link_path" ]]; then
    local target
    target="$(readlink "$link_path" || true)"
    if [[ "$target" == "$lib_file" ]]; then
      return 0
    fi
    rm -f "$link_path"
  elif [[ -e "$link_path" ]]; then
    echo ">>> streamer runtime error: $link_path exists and is not a symlink"
    return 1
  fi

  ln -s "$lib_file" "$link_path"
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
  if [[ "$bin" == "streamer" ]]; then
    ensure_labjack_runtime_lib || return 1
  fi
  echo ">>> Starting $token..."
  if [[ -n "$instance" ]]; then
    if [[ "$bin" == "video-recorder" ]]; then
      local source_url asset_number camera_id spool_dir transport segment_sec settle_sec scan_sec
      local -a env_args
      source_url="$(instance_override VIDEO_SOURCE_URL "$instance")"
      asset_number="$(instance_override VIDEO_ASSET_NUMBER "$instance")"
      camera_id="$(instance_override VIDEO_CAMERA_ID "$instance")"
      spool_dir="$(instance_override VIDEO_SPOOL_DIR "$instance")"
      transport="$(instance_override VIDEO_RTSP_TRANSPORT "$instance")"
      segment_sec="$(instance_override VIDEO_SEGMENT_SEC "$instance")"
      settle_sec="$(instance_override VIDEO_UPLOAD_SETTLE_SEC "$instance")"
      scan_sec="$(instance_override VIDEO_SCAN_INTERVAL_SEC "$instance")"

      [[ -z "$camera_id" ]] && camera_id="$instance"
      [[ -z "$spool_dir" ]] && spool_dir="/tmp/avena-video-recorder-${instance}"

      echo ">>> video-recorder instance=$instance camera_id=$camera_id source=${source_url:-${VIDEO_SOURCE_URL:-unset}}"
      env_args=(
        "INSTANCE=$instance"
        "VIDEO_CAMERA_ID=$camera_id"
        "VIDEO_SPOOL_DIR=$spool_dir"
      )
      [[ -n "$source_url" ]] && env_args+=("VIDEO_SOURCE_URL=$source_url")
      [[ -n "$asset_number" ]] && env_args+=("VIDEO_ASSET_NUMBER=$asset_number")
      [[ -n "$transport" ]] && env_args+=("VIDEO_RTSP_TRANSPORT=$transport")
      [[ -n "$segment_sec" ]] && env_args+=("VIDEO_SEGMENT_SEC=$segment_sec")
      [[ -n "$settle_sec" ]] && env_args+=("VIDEO_UPLOAD_SETTLE_SEC=$settle_sec")
      [[ -n "$scan_sec" ]] && env_args+=("VIDEO_SCAN_INTERVAL_SEC=$scan_sec")

      nohup env "${env_args[@]}" "$BIN_DIR/$bin" >> "$LOG_DIR/$token.log" 2>&1 &
    else
      nohup env INSTANCE="$instance" "$BIN_DIR/$bin" >> "$LOG_DIR/$token.log" 2>&1 &
    fi
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
      bins=()
      for pid_path in "$PID_DIR"/*.pid; do
        [[ -e "$pid_path" ]] || continue
        token="$(basename "$pid_path")"
        bins+=("${token%.pid}")
      done
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
