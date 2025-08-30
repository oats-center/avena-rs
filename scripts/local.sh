#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   ./laptop_orchestrate.sh \
#     --hub 100.64.0.76 --ui-cfg-pass 'strongcfg' \
#     --edges "075:100.64.0.75,087:100.64.0.87"
#
# Does:
# - (optional) seeds KV defaults for each edge via hub publish using ui_cfg user.

HUB=""
UI_CFG_PASS=""
EDGES=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --hub) HUB="$2"; shift 2;;
    --ui-cfg-pass) UI_CFG_PASS="$2"; shift 2;;
    --edges) EDGES="$2"; shift 2;;
    *) echo "Unknown arg $1"; exit 1;;
  esac
done

[[ -z "$HUB" || -z "$UI_CFG_PASS" || -z "$EDGES" ]] && {
  echo "Usage: $0 --hub HUB_IP --ui-cfg-pass PASS --edges \"075:100.64.0.75,087:100.64.0.87\""
  exit 1
}

if ! command -v nats >/dev/null 2>&1; then
  echo "[INFO] 'nats' CLI not found. Skipping KV seeding. Install from https://github.com/nats-io/natscli/releases"
  exit 0
fi

WS_URL="nats://$HUB:4222"   # using TCP here via CLI; PWA will use WS
echo "[hub] seeding via ${WS_URL} as ui_cfg"

IFS=',' read -r -a pairs <<< "$EDGES"
for p in "${pairs[@]}"; do
  id="${p%%:*}"
  ip="${p##*:}"

  json=$(cat <<EOF
{"scans_per_read":200,"suggested_scan_rate":7000.0,
 "channels":[0,1,2,3,4,5,6,7],
 "asset_number": ${id#0},
 "nats_url":"nats://${ip}:4222","nats_subject":"labjack","nats_stream":"LABJACK","parquet_file":"/var/lib/edge${id}/parquet"}
EOF
)
  subj="KV.sampler_cfg.edge${id}"
  echo "[seed] ${subj}"
  nats --server "${WS_URL}" --user ui_cfg --password "${UI_CFG_PASS}" pub "${subj}" "${json}" >/dev/null
done

echo "[done] seeded KV defaults via hub."

