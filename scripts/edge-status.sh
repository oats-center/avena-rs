#!/usr/bin/env bash
set -uo pipefail

units=(
  avena-camera.service
  avena-streamer.service
  avena-archiver.service
  avena-exporter.service
  nats-leaf.service
  nats-exporter.service
  alloy.service
  avena-health-metrics.timer
)

bad=0
printf 'Avena edge status -- %s -- %s\n\n' "$(hostname)" "$(date --iso-8601=seconds)"
printf '%-36s %-12s %-12s\n' UNIT ACTIVE BOOT
printf '%-36s %-12s %-12s\n' '------------------------------------' '------------' '------------'
for unit in "${units[@]}"; do
  active=$(systemctl is-active "$unit" 2>/dev/null || true)
  boot=$(systemctl is-enabled "$unit" 2>/dev/null || true)
  [[ -n "$active" ]] || active=missing
  [[ -n "$boot" ]] || boot=missing
  printf '%-36s %-12s %-12s\n' "$unit" "$active" "$boot"
  [[ "$active" == active ]] || bad=1
done

printf '\nClock\n'
printf '  timezone: %s\n' "$(timedatectl show --property=Timezone --value 2>/dev/null || echo unknown)"
printf '  NTP synchronized: %s\n' "$(timedatectl show --property=NTPSynchronized --value 2>/dev/null || echo unknown)"

printf '\nLocal NATS\n'
if command -v curl >/dev/null && command -v jq >/dev/null; then
  if varz=$(curl --max-time 2 --fail --silent http://127.0.0.1:8222/varz); then
    printf '  server: %s\n' "$(jq -r '.server_name // "unknown"' <<<"$varz")"
    printf '  JetStream: %s\n' "$(jq -r 'if .jetstream then "enabled" else "disabled" end' <<<"$varz")"
  else
    printf '  monitor endpoint: UNREACHABLE\n'
    bad=1
  fi
  if leafz=$(curl --max-time 2 --fail --silent http://127.0.0.1:8222/leafz); then
    leaf_count=$(jq -r '.leafnodes // (.leafs | length) // 0' <<<"$leafz")
    printf '  central leaf connections: %s\n' "$leaf_count"
    [[ "$leaf_count" =~ ^[0-9]+$ && "$leaf_count" -gt 0 ]] || bad=1
  else
    printf '  leaf endpoint: UNREACHABLE\n'
    bad=1
  fi
else
  printf '  skipped: curl and jq are required\n'
fi

printf '\nRecent output\n'
latest_labjack=$(find /home/user/avena-rs/rust-ljm/parquet -type f \
  \( -name '*.parquet' -o -name '*.parquet.inprogress' \) \
  -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2-)
latest_camera=$(find /extstore/camera -type f \
  \( -name '*.parquet' -o -name '*.jpg' -o -name '*.parquet.inprogress' \) \
  -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2-)
if [[ -n "$latest_labjack" ]]; then
  printf '  LabJack: %s  %s\n' "$(stat -c '%y' "$latest_labjack" | cut -d. -f1)" "$latest_labjack"
else
  printf '  LabJack: no data files found\n'
fi
if [[ -n "$latest_camera" ]]; then
  printf '  camera:  %s  %s\n' "$(stat -c '%y' "$latest_camera" | cut -d. -f1)" "$latest_camera"
else
  printf '  camera: no event/Parquet files found (normal when there are no events)\n'
fi

printf '\nStorage\n'
df -h /extstore 2>/dev/null | awk 'NR == 1 || NR == 2 {printf "  %s\n", $0}'

printf '\nFailed systemd units\n'
failed=$(systemctl --failed --no-legend --plain 2>/dev/null || true)
if [[ -n "$failed" ]]; then
  printf '%s\n' "$failed" | sed 's/^/  /'
  bad=1
else
  printf '  none\n'
fi

printf '\nUseful commands\n'
printf '  journalctl -fu avena-camera\n'
printf '  journalctl -fu avena-streamer\n'
printf '  journalctl -fu avena-archiver\n'
printf '  journalctl -fu avena-exporter\n'
printf '  journalctl -fu nats-leaf\n'
printf '  journalctl -fu alloy\n'

exit "$bad"
