#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="exporter"
BUILD_TARGET="exporter"
CONFIG_FILE="${CONFIG_FILE:-$ROOT_DIR/exporter.env.json}"
BIN_PATH="${BIN_PATH:-$ROOT_DIR/target/release/exporter}"
RUNTIME_DIR="${RUNTIME_DIR:-$ROOT_DIR/.runtime}"
PID_FILE="${PID_FILE:-$RUNTIME_DIR/exporter.pid}"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/logs}"
LOG_FILE="${LOG_FILE:-$LOG_DIR/exporter.log}"

# shellcheck disable=SC1091
source "$ROOT_DIR/binctl-common.sh"
run_control_cmd "${1:-}"
