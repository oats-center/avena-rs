#!/usr/bin/env bash

# Detect whether this file is sourced (works in bash/zsh).
_ENV_SETUP_SOURCED=0
(return 0 2>/dev/null) && _ENV_SETUP_SOURCED=1

# Only set strict options when executed directly, not when sourced into an interactive shell.
if [[ "$_ENV_SETUP_SOURCED" -eq 0 ]]; then
  set -eo pipefail
fi

_ENV_SETUP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_ENV_SETUP_OS="$(uname -s 2>/dev/null || echo unknown)"

: "${NATS_SUBJECT:=avenabox}"
: "${NATS_SERVERS:=nats://nats1.oats:4222,nats://nats2.oats:4222,nats://nats3.oats:4222}"
: "${ASSET_NUMBER:=1001}"
: "${OUTPUT_DIR:=outputs}"
: "${NATS_CREDS_FILE:=${_ENV_SETUP_DIR}/apt.creds}"
: "${CFG_BUCKET:=avenabox}"
: "${CFG_KEY:=labjackd.config.i69-mu1}"
: "${LABJACK_IP:=10.165.77.233}"
: "${LABJACK_USB_ID:=ANY}"
: "${LABJACK_OPEN_ORDER:=ethernet,usb}"
if [[ "${_ENV_SETUP_OS}" == "Darwin" ]]; then
  : "${LJM_LIB_FILE:=/usr/local/lib/libLabJackM-1.23.4.dylib}"
else
  : "${LJM_LIB_FILE:=}"
fi

: "${EXPORTER_HTTP_URL:=http://127.0.0.1:9001}"

export NATS_SUBJECT NATS_SERVERS ASSET_NUMBER OUTPUT_DIR NATS_CREDS_FILE CFG_BUCKET CFG_KEY LABJACK_IP LABJACK_USB_ID LABJACK_OPEN_ORDER TRIGGER_STREAM
export LJM_LIB_FILE
export EXPORTER_HTTP_URL

# Ensure macOS can locate LabJack shared library for streamer/examples.
if [[ "${_ENV_SETUP_OS}" == "Darwin" && -f "${LJM_LIB_FILE}" ]]; then
  LJM_LIB_DIR="$(dirname "${LJM_LIB_FILE}")"
  export LJM_PATH="${LJM_LIB_FILE}"
  if [[ -n "${DYLD_FALLBACK_LIBRARY_PATH:-}" ]]; then
    case ":${DYLD_FALLBACK_LIBRARY_PATH}:" in
      *":${LJM_LIB_DIR}:"*) ;;
      *) export DYLD_FALLBACK_LIBRARY_PATH="${LJM_LIB_DIR}:${DYLD_FALLBACK_LIBRARY_PATH}" ;;
    esac
  else
    export DYLD_FALLBACK_LIBRARY_PATH="${LJM_LIB_DIR}"
  fi
fi

unset _ENV_SETUP_SOURCED
unset _ENV_SETUP_DIR
unset _ENV_SETUP_OS
