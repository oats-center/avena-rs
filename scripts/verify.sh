#!/usr/bin/env bash
set -euo pipefail

# verify_nats.sh
# Checks if hub or edge NATS service is installed, running, and responding.

NATS_BIN="${NATS_BIN:-/usr/local/bin/nats-server}"
NATS_CLI="${NATS_CLI:-/usr/local/bin/nats}"
SYSTEMCTL_BIN="$(command -v systemctl || true)"

echo "[verify] Starting NATS verification..."

# 1. Check if nats-server binary exists
if [[ ! -x "$NATS_BIN" ]]; then
  echo "[error] nats-server binary not found at $NATS_BIN"
  exit 1
else
  echo "[ok] nats-server binary found: $NATS_BIN"
fi

# 2. Detect running services (hub or edge)
HUB_ACTIVE=false
EDGE_ACTIVE=false
if [[ -n "$SYSTEMCTL_BIN" ]]; then
  if systemctl is-active --quiet nats-hub 2>/dev/null; then
    HUB_ACTIVE=true
    echo "[ok] hub service 'nats-hub' is active."
  fi

  for svc in $(systemctl list-units --type=service --no-legend | awk '{print $1}' | grep -E '^nats-edge[0-9]+\.service$' || true); do
    if systemctl is-active --quiet "$svc"; then
      EDGE_ACTIVE=true
      echo "[ok] edge service '$svc' is active."
    fi
  done
fi

if [[ "$HUB_ACTIVE" = false && "$EDGE_ACTIVE" = false ]]; then
  echo "[error] No active hub or edge NATS services found."
  exit 1
fi

# 3. Verify TCP ports
check_port() {
  local host="$1" port="$2" name="$3"
  if nc -z "$host" "$port" 2>/dev/null; then
    echo "[ok] $name port $host:$port reachable"
  else
    echo "[warn] $name port $host:$port NOT reachable"
  fi
}

if [[ "$HUB_ACTIVE" = true ]]; then
  check_port 127.0.0.1 4222 "hub client"
  check_port 0.0.0.0 8080 "hub websocket"
  check_port 0.0.0.0 7422 "hub leafnode"
fi

if [[ "$EDGE_ACTIVE" = true ]]; then
  check_port 127.0.0.1 4222 "edge client"
fi

# 4. Test CLI connectivity
if [[ -x "$NATS_CLI" ]]; then
  echo "[verify] Testing NATS CLI..."
  if "$NATS_CLI" --server nats://127.0.0.1:4222 ping >/dev/null 2>&1; then
    echo "[ok] NATS CLI can connect and ping localhost:4222"
  else
    echo "[warn] NATS CLI could not ping localhost:4222"
  fi
else
  echo "[warn] nats CLI not installed; skipping CLI test"
fi

# 5. If edge, check JetStream
if [[ "$EDGE_ACTIVE" = true && -x "$NATS_CLI" ]]; then
  if "$NATS_CLI" --server nats://127.0.0.1:4222 account info >/dev/null 2>&1; then
    echo "[ok] JetStream enabled and responding"
  else
    echo "[warn] JetStream not responding on this edge"
  fi
fi

echo "[verify] Done."
