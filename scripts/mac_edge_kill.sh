#!/usr/bin/env bash
set -euo pipefail
EDGE_ID="${EDGE_ID:?set EDGE_ID like 075}"

launchctl unload "/Library/LaunchDaemons/io.sampler.${EDGE_ID}.plist" 2>/dev/null || true
rm -f "/Library/LaunchDaemons/io.sampler.${EDGE_ID}.plist"

launchctl unload "/Library/LaunchDaemons/io.nats.edge${EDGE_ID}.plist" 2>/dev/null || true
rm -f "/Library/LaunchDaemons/io.nats.edge${EDGE_ID}.plist"

rm -rf "/etc/nats/edge${EDGE_ID}" "/var/lib/nats/edge${EDGE_ID}" "/var/lib/edge${EDGE_ID}/parquet"
echo "[edge ${EDGE_ID}] removed (mac)."
