#!/bin/bash
set -euo pipefail

APP_NAME="avena-web"

echo ">>> Stopping PM2 process '${APP_NAME}'..."
if pm2 describe "${APP_NAME}" >/dev/null 2>&1; then
  pm2 stop "${APP_NAME}" >/dev/null || true
  pm2 delete "${APP_NAME}" >/dev/null || true
  pm2 save >/dev/null || true
  echo ">>> '${APP_NAME}' stopped and removed from PM2."
else
  echo ">>> No PM2 process named '${APP_NAME}' running."
fi

echo ">>> You can now edit or rebuild the application."
