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
: "${VIDEO_BUCKET:=avena_videos}"
: "${VIDEO_TZ:=America/New_York}"
: "${FFMPEG_BIN:=ffmpeg}"
: "${VIDEO_TMP_DIR:=/tmp/avena-video}"
: "${EXPORTER_HTTP_URL:=http://127.0.0.1:9001}"
: "${ROLE:=server}"

export NATS_SUBJECT ASSET_NUMBER OUTPUT_DIR NATS_CREDS_FILE CFG_BUCKET CFG_KEY LABJACK_IP
export VIDEO_BUCKET VIDEO_TZ FFMPEG_BIN VIDEO_TMP_DIR EXPORTER_HTTP_URL ROLE

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  eval "${_RUN_SH_SAVED_OPTS}"
  unset _RUN_SH_SAVED_OPTS
fi
