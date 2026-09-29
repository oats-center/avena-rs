#!/usr/bin/env bash
# Waits until the local NATS server has JetStream up before an Avena service starts.
#
# Installed as /usr/local/libexec/avena-rs/wait-for-local-nats and run as
# ExecStartPre of avena-streamer, avena-archiver and avena-exporter through the
# 10-nats-ready.conf drop-ins. After a boot or a nats-leaf restart the container is
# "started" long before JetStream has recovered its store; without this wait the
# services fail their first JetStream calls and cycle through restarts.
#
# Polls the monitoring endpoint once a second for up to 600 s and exits 0 as soon
# as it answers healthy, or 1 when the time runs out.
set -euo pipefail

HEALTH_URL="http://127.0.0.1:8222/healthz?js-enabled-only=true"
TIMEOUT_SECS=600

start=$SECONDS
until curl --fail --silent --max-time 5 "$HEALTH_URL" >/dev/null 2>&1; do
  if (( SECONDS - start >= TIMEOUT_SECS )); then
    echo "Local NATS JetStream is not ready after ${TIMEOUT_SECS}s." >&2
    exit 1
  fi
  sleep 1
done
echo "Local NATS JetStream is ready after $((SECONDS - start))s."
