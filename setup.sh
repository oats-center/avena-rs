#!/bin/bash
set -euo pipefail

APP_NAME="avena-web"
REPO_URL="https://github.com/oats-center/avena-rs.git"
APP_DIR="$HOME/avena-rs"
WEBAPP_DIR="$APP_DIR/webapp"

# Defaults for Tailnet deployment
EXPORTER_WS_URL="${EXPORTER_WS_URL:-ws://100.64.0.75:9001/export}"
WEB_PORT="${WEB_PORT:-3002}"

command -v pnpm >/dev/null || { echo "pnpm is required"; exit 1; }
command -v pm2 >/dev/null  || { echo "pm2 is required"; exit 1; }

echo ">>> Checking repository..."
if [ ! -d "$APP_DIR/.git" ]; then
  git clone "$REPO_URL" "$APP_DIR"
fi

echo ">>> Pulling latest code..."
cd "$APP_DIR"
git pull --rebase origin main

echo ">>> Installing web dependencies..."
cd "$WEBAPP_DIR"
pnpm install --frozen-lockfile

echo ">>> Building web app with exporter endpoint ${EXPORTER_WS_URL}..."
VITE_EXPORT_WS_URL="$EXPORTER_WS_URL" pnpm run build

echo ">>> Updating PM2 process..."
pm2 delete "$APP_NAME" || true

WEB_PORT="$WEB_PORT" pm2 start bash \
  --name "$APP_NAME" \
  --cwd "$WEBAPP_DIR" \
  -- \
  -lc "pnpm run preview -- --host 0.0.0.0 --port ${WEB_PORT}"

pm2 save

pm2 save >/dev/null || true

host_ip=$(tailscale ip -4 2>/dev/null | head -n1 || hostname -I | awk '{print $1}')
echo ">>> Deployment complete."
echo ">>> Web UI: http://${host_ip}:${WEB_PORT}"
echo ">>> Exporter expected at ${EXPORTER_WS_URL}"
