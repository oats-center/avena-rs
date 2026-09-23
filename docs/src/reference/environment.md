# Environment variables

The Rust services take their settings from environment variables. On an edge
node you never set these by hand: `render-edge-config.py` writes them from the
[box profile](profile.md) into `/etc/avena-rs/<service>.env.json`, and
`avena-service-run.sh` exports them before starting the binary. Relative paths
in `NATS_CREDS_FILE`, `OUTPUT_DIR` and `PARQUET_DIR` are resolved against the
`rust-ljm` directory.

The tables list what each binary actually reads, with the default the code
uses when the variable is unset.

## Shared by all services

| Variable | Default | Meaning |
|---|---|---|
| `NATS_SERVERS` | `nats://127.0.0.1:4222` | Comma-separated URLs of the local NATS server. |
| `NATS_CREDS_FILE` | `apt.creds` | Credentials file for that connection. |
| `JS_DOMAIN` | default domain | The box's JetStream domain, e.g. `edge-i69-mu1`. |

## streamer

| Variable | Default | Meaning |
|---|---|---|
| `CFG_BUCKET` | `avenabox` | Local key-value bucket holding the configuration. |
| `CFG_KEY` | `unknown-site.macbook.unknown-source.config` | Key of this box's configuration. Always set it. |
| `CENTRAL_NATS_SERVERS` | unset | Central NATS URLs to mirror the configuration from. Mirroring is off when neither this nor `CFG_NATS_SERVERS` is set. |
| `CFG_NATS_SERVERS` | unset | Fallback for `CENTRAL_NATS_SERVERS`. This is the one the renderer sets. |
| `CENTRAL_NATS_CREDS_FILE` | `NATS_CREDS_FILE` | Credentials for the central connection. |
| `CENTRAL_CFG_BUCKET` | `CFG_BUCKET` | Central bucket to mirror from. |
| `CENTRAL_CFG_KEY` | `CFG_KEY` | Central key to mirror from. |
| `CENTRAL_JS_DOMAIN` | `CFG_JS_DOMAIN` | JetStream domain on the central server, if any. |
| `STREAM_MAX_BYTES` | unlimited | Size limit of the JetStream stream, in bytes. When full, the oldest messages are discarded. |
| `LABJACK_IP` | none, required | IPv4 address of the LabJack T7. |
| `LABJACK_IDENTIFIER` | unset | Used instead of `LABJACK_IP` only if it is an IPv4 address. |
| `LABJACK_SERIAL` | unset | Expected serial number. A device with a different serial is refused. `ANY` or a non-number turns the check off. |
| `LABJACK_NAME` | unset | Name used in log messages. |
| `STREAMER_MAX_LABJACK_FAILURES` | `5` | Consecutive failures after which the streamer exits and stays stopped. |
| `STREAMER_LABJACK_RETRY_DELAY_SECS` | `5` | Wait before retrying after a failure, in seconds. |
| `LJM_PATH` | unset | Path to `libLabJackM.so`, if it is not on the library path. |

The streamer takes its identity (site, box, source, channels, rates) from the
configuration document, not from the environment. The renderer also writes
`ASSET_NUMBER`, `SITE_ID`, `BOX_ID`, `SOURCE_TYPE`, `SOURCE_ID`, `OUTPUT_DIR`,
`EXPORTER_HTTP_URL`, `LABJACK_USB_ID` and `LABJACK_OPEN_ORDER` into
`streamer.env.json`; the streamer does not read them.

## archiver

| Variable | Default | Meaning |
|---|---|---|
| `PARQUET_DIR` | `parquet` | Root of the archive. |
| `CFG_BUCKET`, `CFG_KEY` | as for the streamer | The configuration to follow. |
| `CENTRAL_NATS_SERVERS`, `CFG_NATS_SERVERS` and the other `CENTRAL_*` variables | as for the streamer | Optional configuration mirror, same behavior as the streamer. |

## exporter

| Variable | Default | Meaning |
|---|---|---|
| `EXPORTER_MODE` | `worker` | `worker` answers requests over NATS. `direct` (or `local`) serves a WebSocket instead. |
| `EXPORTER_ADDR` | `0.0.0.0:9001` | Listen address in `direct` mode only. |
| `PARQUET_DIR` | `parquet` | Root of the archive to read. |
| `NATS_SUBJECT` | `avenars` | First token of the request subject. |
| `SITE_ID` | `unknown-site` | Site token of the request subject. |
| `EXPORT_BOX_ID` or `BOX_ID` | none, required in `worker` mode | Box token of the request subject. |
| `SOURCE_ID` | `unknown-source` | Source token of the request subject. |
| `SOURCE_TYPE` | `labjack` | Read but currently not used in the subject. |

In `worker` mode the exporter subscribes to
`<NATS_SUBJECT>.<SITE_ID>.<BOX_ID>.<SOURCE_ID>.export.request`, so these values
must match the box's configuration exactly.

## subscriber

`subscriber` is a diagnostic tool and reads its settings from the environment
as well; see [Command-line tools](tools.md#subscriber).
