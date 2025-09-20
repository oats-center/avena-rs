#!/bin/bash
APP_NAME="avena-rs"

echo ">>> Stopping app..."
pm2 stop "$APP_NAME" || echo "App not running"
pm2 delete "$APP_NAME" || echo "App already deleted"
pm2 save

echo ">>> App shut down. You can now edit or rebuild manually."
