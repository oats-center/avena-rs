# Command-line tools

## request-nats-export.mjs

Makes one export request through central NATS, exactly as the webapp does, and
saves the CSV. Use it to script downloads or to test the export path without a
browser. It uses the webapp's NATS libraries, so run `pnpm install` in
`webapp/` once first.

```bash
node scripts/request-nats-export.mjs \
  --subject avenars.i69.i69-mu1.i69-lj2.export.request \
  --asset 1001 --channels 8,9,10,11 \
  --start 2026-09-22T12:00:00Z --end 2026-09-22T12:30:00Z \
  --output out/mu1.csv --creds apt.creds
```

| Option | Required | Meaning |
|---|---|---|
| `--subject` | yes | The box's export request subject |
| `--asset` | yes | Asset number |
| `--channels` | yes | Comma-separated channel numbers |
| `--start`, `--end` | yes | RFC 3339 times, inclusive |
| `--output` | yes | CSV file to write; its folder is created if needed |
| `--servers` | no | Comma-separated NATS URLs. Default `nats://nats1.oats:4222,nats://nats2.oats:4222` |
| `--creds` | no | Credentials file. Default `/etc/avena-rs/apt.creds` |
| `--report` | no | Also write the JSON summary to this file |
| `--timeout-seconds` | no | Give up after this long. Default `120`; raise it for large exports |

It prints a JSON report with the bytes, chunks and rows received and any
channels with no data.

## recompress

Rewrites archive files written before September 2026 in the current format
(zstd, delta timestamps, one row group). Values do not change, and each file is
checked before it replaces the original.

```bash
cd rust-ljm
cargo build --release --bin recompress
./target/release/recompress <parquet_root> --dry-run   # report only
./target/release/recompress <parquet_root>
```

For each `part-*.parquet` file under the root, it writes a compressed copy next
to it, syncs it, reads it back and compares every timestamp and value bit for
bit, along with the metadata. Only then does it rename the copy over the
original. At any moment every file is either the complete original or the
verified copy, so exports keep working during a run, and a run can be stopped
and started again. It skips files that are already compressed, files with no
rows, unfinished `.inprogress` files and quarantined files.

It prints progress every 1,000 files and a summary at the end, and exits with
an error if any file failed; a failed file keeps its original. Run it with
`nice -n 19` on a box that is recording.

## subscriber

Captures live samples to CSV without the archiver, for checking a stream by
eye. It subscribes to the live wildcard for one source and appends one file
per channel, `labjack_<asset>_<channel>.csv`, with the columns
`sequence,timestamp,raw_value`. It only sees messages published while it runs.

```bash
cd rust-ljm
NATS_SUBJECT=avenars SITE_ID=i69 BOX_ID=i69-mu1 SOURCE_ID=i69-lj2 \
ASSET_NUMBER=1001 OUTPUT_DIR=/tmp/capture NATS_CREDS_FILE=apt.creds \
cargo run --release --bin subscriber
```

Set `SITE_ID`, `BOX_ID` and `SOURCE_ID`. With none of them it falls back to the
legacy wildcard, which matches every asset but names every file after
`ASSET_NUMBER`.

## render-edge-config.py

Turns a [box profile](profile.md) into the box's generated files.

```bash
./shared/render-edge-config.py --config shared/edge-boxes/<box>.json \
  --output-dir target/edge-config/<box>
```

| Option | Default | Meaning |
|---|---|---|
| `--config` | `shared/edge-box.config.json` | The profile to render |
| `--output-dir` | none | Write the files here. Without it, the files are written into `shared/` and `rust-ljm/` in the repository. |
| `--repo-root` | `.` | Repository root that relative paths are resolved against |

## install-edge-services.sh

Renders a profile, builds the release binaries, installs them with their
configuration and credentials, and enables the systemd units. Run it on the
edge node.

```bash
./scripts/install-edge-services.sh --profile shared/edge-boxes/<box>.json [--start]
```

`--start` restarts the services and Alloy after installing. Without it the
units are enabled but left as they were. It expects the client credentials in
`rust-ljm/apt.creds`.

## edge-status.sh

Prints the state of every unit, clock synchronization, the local NATS server
and its leaf connection, the newest LabJack and camera files, disk space and
failed units. Exit status 0 means every check passed. On a box it is installed
as `/home/user/avena-status`.

## LabJack examples

Small programs in `rust-ljm/examples` for checking the LabJack without the
services. They read `LABJACK_IP` (and optionally `LABJACK_SERIAL`) from the
environment, or from the file named by `CONFIG_FILE`, or from a
`streamer.env.json` in the current folder.

| Example | What it does |
|---|---|
| `info` | Opens the device and prints its type, address and connection |
| `read` | Polls the analog inputs and prints their values |
| `stream` | Runs an LJM stream and prints the samples, without NATS |
| `stop` | Stops a stream left running by a crashed process |

```bash
cd rust-ljm
LABJACK_IP=192.168.1.111 cargo run --example info
```

`info` works while the streamer is running. `read` and `stream` do not: the
LabJack runs one stream at a time and refuses analog reads while it streams, so
stop `avena-streamer` first. `stop` ends whatever stream is running, including
the streamer's, so only use it when the streamer is already stopped.
