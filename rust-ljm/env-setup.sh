#!/usr/bin/env bash

# If sourced, don't leak shell options into the parent shell.
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  _RUN_SH_SAVED_OPTS="$(set +o)"
fi

set -eo pipefail

: "${NATS_SUBJECT:=avenabox}"
: "${ASSET_NUMBER:=1001}"
: "${OUTPUT_DIR:=outputs}"
: "${NATS_CREDS_FILE:=apt.creds}"
: "${CFG_BUCKET:=avenabox}"
: "${CFG_KEY:=labjackd.config.i69-mu1}"
: "${LABJACK_IP:=10.165.77.233}"
: "${ROLE:=server}"

export NATS_SUBJECT ASSET_NUMBER OUTPUT_DIR NATS_CREDS_FILE CFG_BUCKET CFG_KEY LABJACK_IP ROLE

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  eval "${_RUN_SH_SAVED_OPTS}"
  unset _RUN_SH_SAVED_OPTS
fi
