#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="streamer"
BUILD_TARGET="streamer"
CONFIG_FILE="${CONFIG_FILE:-$ROOT_DIR/streamer.env.json}"
BIN_PATH="${BIN_PATH:-$ROOT_DIR/target/release/streamer}"
RUNTIME_DIR="${RUNTIME_DIR:-$ROOT_DIR/.runtime}"
PID_FILE="${PID_FILE:-$RUNTIME_DIR/streamer.pid}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/logs}"
LOG_FILE="${LOG_FILE:-$LOG_DIR/streamer.log}"

# shellcheck disable=SC1091
source "$ROOT_DIR/binctl-common.sh"
run_control_cmd "${1:-}"
