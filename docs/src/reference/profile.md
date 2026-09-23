# Box profile

A box profile is one JSON file in `shared/edge-boxes/`, named after the box
(`i69-mu1.json`). It is the only place an edge node's identity is written down.
`shared/render-edge-config.py` turns it into everything else the box needs:

```bash
./shared/render-edge-config.py \
  --config shared/edge-boxes/i69-mu1.json \
  --output-dir target/edge-config/i69-mu1
```

| Generated file | Installed as | Read by |
|---|---|---|
| `nats-leaf.conf` | `/etc/containers/systemd/nats-leaf.conf` | the NATS server |
| `alloy.container` | `/etc/containers/systemd/alloy.container` | systemd (Quadlet) |
| `streamer.env.json` | `/etc/avena-rs/streamer.env.json` | the streamer |
| `archiver.env.json` | `/etc/avena-rs/archiver.env.json` | the archiver |
| `exporter.env.json` | `/etc/avena-rs/exporter.env.json` | the exporter |
| `labjack-kv.generated.json` | the key-value bucket, by hand ([setup step 9](../setup/edge-node.md#9-seed-the-labjack-configuration)) | streamer and archiver |

Without `--output-dir`, the renderer writes into the repository instead
(`shared/` and `rust-ljm/`), overwriting whichever box was rendered last. Always
pass `--output-dir`. The installer does.

The renderer stops with an error naming the field if a required field is
missing or empty.

## Top level

| Field | Required | Meaning |
|---|---|---|
| `site_id` | yes | Site name, e.g. `i69`. First token of subjects and keys. |
| `box_id` | yes | This box, e.g. `i69-mu1`. Also its hostname. |
| `source.type` | yes | `labjack`. |
| `source.id` | yes | This box's LabJack in subjects and keys, e.g. `i69-lj2`. |

## `nats`

| Field | Required | Meaning |
|---|---|---|
| `root_subject` | yes | Subject root, `avenars`. |
| `local_servers` | yes | How the services reach the local server: `nats://127.0.0.1:4222`. |
| `config_servers` | no | Central servers to mirror configuration from, comma-separated: `nats://nats1.oats:4222,nats://nats2.oats:4222`. Empty turns mirroring off. |
| `config_jetstream_domain` | no | JetStream domain of the central bucket. Empty for the default. |
| `leaf_server_name` | yes | Name of the local server, `<box_id>-leaf`. Shows up in `/varz` and on central. |
| `leaf_listen` | yes | Client listen address of the local server, `127.0.0.1:4222`. |
| `monitor_listen` | yes | Monitoring listen address, `127.0.0.1:8222`. |
| `jetstream_domain` | yes | This box's JetStream domain, `edge-<box_id>`. Must be unique. |
| `jetstream_store_dir` | yes | Store directory **inside the container**, `/var/lib/nats/jetstream`. On the host it is under `/home/user/nats`. |
| `leaf_credentials_path` | yes | Leaf credentials **inside the container**, `/etc/nats/creds/leaf.creds`. On the host, `/etc/containers/systemd/creds/leaf.creds`. |
| `leaf_remotes` | yes, at least one | Central leaf endpoints: `nats://nats1.oats:7422`, `nats://nats2.oats:7422`. |
| `stream_max_bytes` | no | Size limit of the local stream in bytes, a positive integer. The oldest messages are dropped when it is reached. |
| `kv_bucket` | yes | Configuration bucket, `avenabox`. |
| `kv_key` | yes | This box's configuration key, `<site_id>.<box_id>.<source_id>.config`. |
| `stream_name` | yes | JetStream stream for live samples, `labjacks`. |

## `paths`

| Field | Required | Meaning |
|---|---|---|
| `rust_creds_file` | yes | Credentials the services use: `/etc/avena-rs/apt.creds`. |
| `parquet_dir` | yes | Root of the archive. |
| `output_dir` | yes | Written to the streamer's environment; the streamer does not currently use it. |
| `exporter_http_url` | yes | Written to the streamer's environment; not used by the current services. |

## `labjack`

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Name of the LabJack. Usually the same as `source.id`. |
| `asset_number` | yes | Unique integer shown in the webapp and used in archive paths. |
| `max_channels` | yes | Analog inputs the webapp offers, `14` for a T7. |
| `ip` | yes | The LabJack's reserved IPv4 address. |
| `serial` | yes | The LabJack's serial number. The streamer refuses a different device. |
| `rotate_secs` | yes | Archive file window in seconds, normally `300`. |
| `sensor_settings` | yes | Initial recording settings. `scans_per_read` and `scan_rate_hz` are required and `channels_enabled` must not be empty. The whole object is copied into the [LabJack configuration](kv-config.md). |
| `max_failures` | no | Becomes `STREAMER_MAX_LABJACK_FAILURES`. Default `5`. |
| `retry_delay_secs` | no | Becomes `STREAMER_LABJACK_RETRY_DELAY_SECS`. Default `5`. |
| `identifier`, `usb_id`, `open_order` | no | Written to the environment but effectively unused: the streamer opens the LabJack by `ip`, and falls back to `identifier` only when no IP is set, which the renderer does not allow. |

`sensor_settings` in the profile only seeds the configuration. Once the box is
running, the copy in the key-value bucket is what counts, and edits belong
there.
