#!/bin/bash
set -euo pipefail

APP_NAME="avena-web"

echo ">>> Stopping PM2 process '${APP_NAME}'..."

# Delete the process if it exists (no-op if it doesn't)
pm2 delete "${APP_NAME}" >/dev/null 2>&1 || true

# Persist PM2 state
pm2 save >/dev/null 2>&1 || true

echo ">>> '${APP_NAME}' stopped (if it was running)."
echo ">>> You can now edit or rebuild the application."
