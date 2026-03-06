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
: "${LABJACK_IDENTIFIER:=}"
: "${LABJACK_SERIAL:=}"
: "${LABJACK_NAME:=}"
: "${LABJACK_IP:=}"
: "${LABJACK_USB_ID:=ANY}"
: "${LABJACK_OPEN_ORDER:=ethernet,usb}"
if [[ "${_ENV_SETUP_OS}" == "Darwin" ]]; then
  if [[ -z "${LJM_LIB_FILE:-}" ]]; then
    for candidate in \
      /usr/local/lib/libLabJackM.dylib \
      /opt/homebrew/lib/libLabJackM.dylib \
      /usr/local/lib/libLabJackM-1.dylib \
      /usr/local/lib/libLabJackM-1.23.4.dylib
    do
      if [[ -f "${candidate}" ]]; then
        LJM_LIB_FILE="${candidate}"
        break
      fi
    done
  fi
elif [[ "${_ENV_SETUP_OS}" == "Linux" ]]; then
  if [[ -z "${LJM_LIB_FILE:-}" ]]; then
    for candidate in \
      /usr/local/lib/libLabJackM.so \
      /usr/lib/libLabJackM.so
    do
      if [[ -f "${candidate}" ]]; then
        LJM_LIB_FILE="${candidate}"
        break
      fi
    done
  fi
else
  : "${LJM_LIB_FILE:=}"
fi

: "${EXPORTER_HTTP_URL:=http://127.0.0.1:9001}"

export NATS_SUBJECT NATS_SERVERS ASSET_NUMBER OUTPUT_DIR NATS_CREDS_FILE CFG_BUCKET CFG_KEY LABJACK_IDENTIFIER LABJACK_SERIAL LABJACK_NAME LABJACK_IP LABJACK_USB_ID LABJACK_OPEN_ORDER TRIGGER_STREAM
export LJM_LIB_FILE
export EXPORTER_HTTP_URL

# Ensure streamer/examples can locate the LabJack shared library.
if [[ -f "${LJM_LIB_FILE}" ]]; then
  LJM_LIB_DIR="$(dirname "${LJM_LIB_FILE}")"
  export LJM_PATH="${LJM_LIB_FILE}"
  export LJM_LIB_DIR
fi

if [[ "${_ENV_SETUP_OS}" == "Darwin" && -f "${LJM_LIB_FILE}" ]]; then
  if [[ -n "${DYLD_FALLBACK_LIBRARY_PATH:-}" ]]; then
    case ":${DYLD_FALLBACK_LIBRARY_PATH}:" in
      *":${LJM_LIB_DIR}:"*) ;;
      *) export DYLD_FALLBACK_LIBRARY_PATH="${LJM_LIB_DIR}:${DYLD_FALLBACK_LIBRARY_PATH}" ;;
    esac
  else
    export DYLD_FALLBACK_LIBRARY_PATH="${LJM_LIB_DIR}"
  fi
fi

if [[ "${_ENV_SETUP_OS}" == "Linux" && -f "${LJM_LIB_FILE}" ]]; then
  if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
    case ":${LD_LIBRARY_PATH}:" in
      *":${LJM_LIB_DIR}:"*) ;;
      *) export LD_LIBRARY_PATH="${LJM_LIB_DIR}:${LD_LIBRARY_PATH}" ;;
    esac
  else
    export LD_LIBRARY_PATH="${LJM_LIB_DIR}"
  fi
fi

unset _ENV_SETUP_SOURCED
unset _ENV_SETUP_DIR
unset _ENV_SETUP_OS
