#!/usr/bin/env bash
set -euo pipefail

CONFIG_DIR="${AVENA_CONFIG_DIR:-/etc/avena-rs}"
PROFILE="$CONFIG_DIR/profile.json"
CREDS="$CONFIG_DIR/apt.creds"
METRICS_DIR="${AVENA_METRICS_DIR:-/var/lib/avena-rs/metrics}"
PARQUET_DIR="$(jq -er '.paths.parquet_dir' "$PROFILE")"
BOX_ID="$(jq -er '.box_id' "$PROFILE")"
JS_DOMAIN="$(jq -er '.nats.jetstream_domain' "$PROFILE")"
STREAM_NAME="$(jq -er '.nats.stream_name' "$PROFILE")"

mkdir -p "$METRICS_DIR"
OUTPUT="$METRICS_DIR/avena.prom"
TEMP_OUTPUT="$OUTPUT.tmp"

service_active() {
  if systemctl is-active --quiet "$1"; then
    echo 1
  else
    echo 0
  fi
}

service_state() {
  if systemctl is-active --quiet "$1"; then
    echo running
  elif systemctl is-failed --quiet "$1"; then
    echo failed
  else
    echo stopped
  fi
}

stream_json="$(
  nats --server nats://127.0.0.1:4222 --creds "$CREDS" \
    --js-domain "$JS_DOMAIN" stream info "$STREAM_NAME" --json 2>/dev/null || true
)"
last_stream_timestamp="$(jq -r '.state.last_ts // empty' <<<"$stream_json" 2>/dev/null || true)"
last_stream_seconds=0
if [[ -n "$last_stream_timestamp" ]]; then
  last_stream_seconds="$(date -d "$last_stream_timestamp" +%s 2>/dev/null || echo 0)"
fi

last_parquet_seconds="$(
  find "$PARQUET_DIR" -type f -name '*.parquet' -printf '%T@\n' 2>/dev/null \
    | sort -nr | head -n 1 | cut -d. -f1 || true
)"
last_parquet_seconds="${last_parquet_seconds:-0}"

quarantined_files="$(
  find "$PARQUET_DIR" -type f -name '*.quarantined-*' 2>/dev/null | wc -l
)"
leaf_connections="$(
  curl -fsS http://127.0.0.1:8222/leafz 2>/dev/null \
    | jq -r '.leafnodes // 0' 2>/dev/null || echo 0
)"

{
  echo '# HELP avena_service_active Whether an Avena system service is active.'
  echo '# TYPE avena_service_active gauge'
  for service in streamer archiver exporter; do
    printf 'avena_service_active{box="%s",service="%s"} %s\n' \
      "$BOX_ID" "$service" "$(service_active "avena-$service")"
  done
  echo '# HELP avena_service_state One-hot Avena service state (running, failed, or stopped).'
  echo '# TYPE avena_service_state gauge'
  for service in streamer archiver exporter; do
    current_state="$(service_state "avena-$service")"
    for state in running failed stopped; do
      value=0
      [[ "$state" == "$current_state" ]] && value=1
      printf 'avena_service_state{box="%s",service="%s",state="%s"} %s\n' \
        "$BOX_ID" "$service" "$state" "$value"
    done
  done
  echo '# HELP avena_stream_last_message_timestamp_seconds Unix time of the newest local JetStream message.'
  echo '# TYPE avena_stream_last_message_timestamp_seconds gauge'
  printf 'avena_stream_last_message_timestamp_seconds{box="%s",stream="%s"} %s\n' \
    "$BOX_ID" "$STREAM_NAME" "$last_stream_seconds"
  echo '# HELP avena_parquet_last_completed_timestamp_seconds Modification time of the newest completed Parquet file.'
  echo '# TYPE avena_parquet_last_completed_timestamp_seconds gauge'
  printf 'avena_parquet_last_completed_timestamp_seconds{box="%s"} %s\n' \
    "$BOX_ID" "$last_parquet_seconds"
  echo '# HELP avena_parquet_quarantined_files Number of preserved incomplete or corrupt Parquet files.'
  echo '# TYPE avena_parquet_quarantined_files gauge'
  printf 'avena_parquet_quarantined_files{box="%s"} %s\n' \
    "$BOX_ID" "$quarantined_files"
  echo '# HELP avena_nats_leaf_connections Current NATS leaf connection count.'
  echo '# TYPE avena_nats_leaf_connections gauge'
  printf 'avena_nats_leaf_connections{box="%s"} %s\n' \
    "$BOX_ID" "$leaf_connections"
} >"$TEMP_OUTPUT"

mv -f "$TEMP_OUTPUT" "$OUTPUT"
