#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE=""
START_SERVICES=false

usage() {
  cat <<'EOF'
Usage: ./scripts/install-edge-services.sh --profile <profile.json> [--start]

Builds the Rust binaries, renders the selected edge profile, and installs
persistent systemd services plus their runtime environment under /etc/avena-rs.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --start)
      START_SERVICES=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$PROFILE" ]]; then
  usage >&2
  exit 2
fi

if [[ "$PROFILE" != /* ]]; then
  PROFILE="$REPO_ROOT/$PROFILE"
fi

if [[ ! -s "$PROFILE" ]]; then
  echo "Profile does not exist: $PROFILE" >&2
  exit 1
fi

BOX_ID="$(jq -er '.box_id' "$PROFILE")"
BUNDLE_REL="target/edge-config/$BOX_ID"
BUNDLE="$REPO_ROOT/$BUNDLE_REL"

cd "$REPO_ROOT"
./shared/render-edge-config.py \
  --config "$PROFILE" \
  --output-dir "$BUNDLE_REL"

cargo build --manifest-path rust-ljm/Cargo.toml --release \
  --bin streamer --bin archiver --bin exporter

sudo install -d -m 0755 /etc/avena-rs
sudo install -d -m 0755 /usr/local/libexec/avena-rs
sudo install -d -o user -g user -m 0755 /var/lib/avena-rs/metrics
sudo install -m 0644 "$PROFILE" /etc/avena-rs/profile.json
sudo install -m 0644 "$BUNDLE/streamer.env.json" /etc/avena-rs/streamer.env.json
sudo install -m 0644 "$BUNDLE/archiver.env.json" /etc/avena-rs/archiver.env.json
sudo install -m 0644 "$BUNDLE/exporter.env.json" /etc/avena-rs/exporter.env.json
sudo install -o root -g user -m 0640 \
  "$REPO_ROOT/rust-ljm/apt.creds" /etc/avena-rs/apt.creds
sudo install -m 0755 \
  "$REPO_ROOT/rust-ljm/avena-service-run.sh" \
  /usr/local/libexec/avena-rs/avena-service-run.sh
sudo install -m 0755 \
  "$REPO_ROOT/scripts/write-edge-health-metrics.sh" \
  /usr/local/libexec/avena-rs/write-edge-health-metrics.sh
for binary in streamer archiver exporter; do
  sudo install -m 0755 \
    "$REPO_ROOT/rust-ljm/target/release/$binary" \
    "/usr/local/libexec/avena-rs/$binary"
done

for unit in avena-streamer avena-archiver avena-exporter; do
  sudo install -m 0644 \
    "$REPO_ROOT/shared/systemd/$unit.service" \
    "/etc/systemd/system/$unit.service"
done
sudo install -m 0644 \
  "$REPO_ROOT/shared/systemd/avena-health-metrics.service" \
  /etc/systemd/system/avena-health-metrics.service
sudo install -m 0644 \
  "$REPO_ROOT/shared/systemd/avena-health-metrics.timer" \
  /etc/systemd/system/avena-health-metrics.timer
sudo install -m 0644 "$REPO_ROOT/shared/config.alloy" \
  /etc/containers/systemd/config.alloy
sudo install -m 0644 "$BUNDLE/alloy.container" \
  /etc/containers/systemd/alloy.container

sudo systemctl daemon-reload
sudo systemctl enable avena-streamer avena-archiver avena-exporter
sudo systemctl enable --now avena-health-metrics.timer

if [[ "$START_SERVICES" == true ]]; then
  sudo systemctl restart avena-streamer
  sudo systemctl restart avena-archiver avena-exporter
  sudo systemctl restart alloy
fi

echo "Installed persistent Avena services for $BOX_ID."
if [[ "$START_SERVICES" == false ]]; then
  echo "Services were enabled but not started. Start after seeding and validating KV."
fi
