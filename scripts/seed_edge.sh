#!/usr/bin/env bash
set -euo pipefail

# Seeds default KV config on each EDGE by running the NATS CLI over SSH on the EDGE.
#
# Usage:
#   ./seed_edge_configs.sh \
#     --edges "075:sshuser@100.64.0.75,087:sshuser@100.64.0.87" \
#     --scans 200 --rate 7000.0 --channels "0,1,2,3,4,5,6,7,8,9,10,11,12,13"
#
# Requirements on each edge:
#   - 'nats' CLI installed and in PATH
#   - NATS edge running on localhost:4222
#
# What it does on each edge:
#   nats kv add bucket sampler_cfg   (if missing)
#   nats kv put sampler_cfg edge<id> <json>

EDGES=""  # format: EDGEID:sshHost,EDGEID:sshHost,...
SCANS=200
RATE=7000.0
CHANNELS="0,1,2,3,4,5,6,7,8,9,10,11,12,13"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --edges) EDGES="$2"; shift 2;;
    --scans) SCANS="$2"; shift 2;;
    --rate) RATE="$2"; shift 2;;
    --channels) CHANNELS="$2"; shift 2;;
    *) echo "Unknown arg $1"; exit 1;;
  esac
done

[[ -z "$EDGES" ]] && { echo "Usage: $0 --edges \"075:root@100.64.0.75,087:root@100.64.0.87\" [--scans N --rate F --channels \"0,1,2\"]"; exit 1; }

IFS=',' read -r -a pairs <<< "$EDGES"
for pair in "${pairs[@]}"; do
  EDGE_ID="${pair%%:*}"
  SSH_HOST="${pair##*:}"
  EDGE_NUM="${EDGE_ID#0}"

  JSON=$(cat <<EOF
{"scans_per_read":${SCANS},"suggested_scan_rate":${RATE},
 "channels":[${CHANNELS}],
 "asset_number":${EDGE_NUM},
 "nats_url":"nats://127.0.0.1:4222",
 "nats_subject":"labjack",
 "nats_stream":"LABJACK",
 "parquet_file":"/var/lib/edge${EDGE_ID}/parquet"}
EOF
)

  echo "[seed] ${EDGE_ID} via ${SSH_HOST}"
  ssh -o StrictHostKeyChecking=accept-new "${SSH_HOST}" bash -s <<'EOSH' "${JSON}" "${EDGE_ID}"
JSON="$1"; EDGE_ID="$2"
set -e
if ! command -v nats >/dev/null; then echo "nats CLI missing on edge ${EDGE_ID}"; exit 1; fi
# create bucket if missing (ignore errors)
nats --server nats://127.0.0.1:4222 kv add bucket sampler_cfg >/dev/null 2>&1 || true
# put config
nats --server nats://127.0.0.1:4222 kv put sampler_cfg "edge${EDGE_ID}" "${JSON}" >/dev/null
EOSH

done

echo "[done] seeded defaults on all edges."
