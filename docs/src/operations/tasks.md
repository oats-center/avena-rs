# Everyday tasks

Commands on this page run on the edge node from `/home/user/avena-rs`, with
`apt.creds` available as `rust-ljm/apt.creds`.

## Look at logs

```bash
journalctl -u avena-streamer -f
journalctl -u avena-archiver -n 100 --no-pager
journalctl -u avena-exporter --since "1 hour ago"
```

## Stop and start in the right order

Stop the producer first, so no samples arrive while the consumers shut down,
and start it last:

```bash
sudo systemctl stop avena-streamer
sudo systemctl stop avena-archiver avena-exporter
# ... work ...
sudo systemctl start avena-archiver avena-exporter
sudo systemctl start avena-streamer
```

Stopping the archiver closes every open file and acknowledges its messages
before the unit reports stopped, so a planned stop loses nothing. Stopping the
streamer leaves a gap in the data for as long as it is stopped.

To restart the local NATS server as well:

```bash
sudo systemctl stop avena-streamer
sudo systemctl stop avena-archiver avena-exporter
sudo systemctl restart nats-leaf
sudo systemctl start avena-archiver avena-exporter
sudo systemctl start avena-streamer
```

## Change channels, sample rate or calibration

Edit the box's configuration in the webapp. It writes the central key; the
streamer mirrors it and restarts sampling. Or from the command line:

```bash
KEY=i69.i69-mu1.i69-lj2.config
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv get avenabox $KEY --raw > config.json
# edit config.json
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds \
  kv put avenabox $KEY "$(cat config.json)"
journalctl -u avena-streamer -n 20 --no-pager    # look for the restart
```

Storage grows with the sample rate. At 2 kHz on four channels the archive
takes about 0.6 GB a day; at 100 Hz, about 30 MB.

## Export data without the webapp

```bash
node scripts/request-nats-export.mjs \
  --subject avenars.i69.i69-mu1.i69-lj2.export.request \
  --asset 1001 --channels 8,9,10,11 \
  --start 2026-09-22T12:00:00Z --end 2026-09-22T12:30:00Z \
  --creds rust-ljm/apt.creds --output mu1.csv
```

This needs the webapp's `node_modules` (run `pnpm install` in `webapp/` once).
See [Command-line tools](../reference/tools.md) for all options.

## Install a new build

```bash
git pull
./scripts/install-edge-services.sh --profile shared/edge-boxes/$(hostname).json
sudo systemctl restart avena-archiver avena-exporter
sudo systemctl restart avena-streamer
```

To go back, check out the previous commit and run the same commands.

## Recompress old archive files

Files written before the archiver used zstd compression are about ten times
larger than they need to be. `recompress` rewrites them in place, checking
every value before it replaces a file:

```bash
cd rust-ljm && cargo build --release --bin recompress
nice -n 19 ./target/release/recompress /home/user/avena-rs/rust-ljm/parquet --dry-run
nice -n 19 ./target/release/recompress /home/user/avena-rs/rust-ljm/parquet
```

It can run while the services are up and can be stopped and restarted at any
point. See [Command-line tools](../reference/tools.md#recompress).

## Free disk space

Nothing on the box deletes old data automatically. Before removing anything
from the archive, copy it off the box and check the copy. Never delete
`/home/user/nats` while the box is in service; it holds the JetStream stream
and the local configuration.
