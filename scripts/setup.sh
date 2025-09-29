#!/bin/bash
set -e

APP_NAME="avena-rs"
APP_DIR="$HOME/avena-rs"

echo ">>> Checking repo..."
if [ ! -d "$APP_DIR" ]; then
  git clone https://github.com/oats-center/avena-rs.git "$APP_DIR"
fi

cd "$APP_DIR"

echo ">>> Pulling latest code..."
git pull origin main

echo ">>> Installing dependencies..."
pnpm install --frozen-lockfile

echo ">>> Building app..."
pnpm build

echo ">>> Starting app with pm2..."
# Stop old instance if running
pm2 delete "$APP_NAME" >/dev/null 2>&1 || true

# Start fresh (SvelteKit preview mode)
pm2 start "pnpm preview -- --host 0.0.0.0 --port 3002" --name "$APP_NAME"

# Save pm2 state so it restarts on reboot
pm2 save

echo ">>> App deployed at http://$(hostname -I | awk '{print $1}'):3002"
