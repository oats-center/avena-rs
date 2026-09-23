# LabJack configuration

Each edge node's recording settings are one JSON document in the NATS key-value
bucket `avenabox`, under the key `<site_id>.<box_id>.<source_id>.config`, for
example `i69.i69-mu1.i69-lj2.config`. The central copy is authoritative; the
streamer mirrors it into the box's own bucket.

The first copy is generated from the [box profile](profile.md) by
`render-edge-config.py`. After that, change it through the webapp or with
`nats kv put`, not by editing the profile: the profile is only read again when
the box is set up.

## Example

```json
{
  "labjack_name": "i69-lj2",
  "asset_number": 1001,
  "max_channels": 14,
  "site_id": "i69",
  "box_id": "i69-mu1",
  "source_type": "labjack",
  "source_id": "i69-lj2",
  "nats_subject": "avenars",
  "nats_stream": "labjacks",
  "rotate_secs": 300,
  "sensor_settings": {
    "scans_per_read": 100,
    "scan_rate_hz": 100,
    "channels_enabled": [8, 9, 10, 11],
    "gains": 1,
    "data_formats": ["pressure", "pressure", "pressure", "pressure"],
    "measurement_units": ["kPa", "kPa", "kPa", "kPa"],
    "labjack_on_off": true,
    "calibrations": {
      "8": { "id": "tp3505", "type": "linear", "a": 70.25, "b": -9.1068 }
    }
  }
}
```

## Top-level fields

| Field | Type | Meaning |
|---|---|---|
| `labjack_name` | string | Name of the LabJack. Used as the source name in subjects when `source_id` is missing. |
| `asset_number` | integer | Asset number shown in the webapp and used in the archive path (`asset<NNN>/`). |
| `max_channels` | integer | Number of analog inputs the webapp offers. The streamer does not use it. |
| `site_id` | string | Site name, the first token of every subject. |
| `box_id` | string | Edge node name, the second token. |
| `source_type` | string | Kind of source, `labjack`. |
| `source_id` | string | Name of this LabJack in subjects, the third token. |
| `nats_subject` | string | Subject root, `avenars`. |
| `nats_stream` | string | JetStream stream holding the live samples, `labjacks`. |
| `rotate_secs` | integer | Length of an archive file's time window, in seconds. Files cover aligned windows, so 300 gives :00 to :05, :05 to :10, and so on. Changing it restarts the stream. |
| `sensor_settings` | object | What to record, below. |

Subjects take the structured form `avenars.<site>.<box>.<source>.live.chNN`
when `nats_subject` is `avenars` or any of `site_id`, `box_id` and `source_id`
is present. Older documents with none of them publish on
`<nats_subject>.<asset>.data.chNN`; see [NATS subjects](subjects.md).

## `sensor_settings`

| Field | Type | Meaning |
|---|---|---|
| `scans_per_read` | integer | Scans the streamer collects per read, and therefore samples per published message on each channel. Older documents call this `scan_rate`, which is still accepted. |
| `scan_rate_hz` | number | Scans per second, per channel. Older documents call this `sampling_rate`, which is still accepted. |
| `channels_enabled` | integer list | LabJack analog inputs to stream, by number (`AIN<n>`). Each becomes one subject and one archive folder. |
| `gains` | integer | Shown and edited by the webapp. The streamer does not use it: every input is read single-ended on the ±10 V range. |
| `data_formats` | string list | What each enabled channel measures, in the same order as `channels_enabled`. Labels only. |
| `measurement_units` | string list | Unit of each enabled channel after calibration. Labels only. |
| `labjack_on_off` | boolean | `false` stops streaming until it is set back to `true`. |
| `calibrations` | object | Conversion from volts to engineering units, keyed by channel number as a string. Optional. |

`scans_per_read` and `scan_rate_hz` together set how often messages are sent:
100 scans per read at 100 Hz is one message per channel per second. Keep a
message to roughly ten per second or fewer; more messages mean more overhead
without more data.

## Calibrations

Each entry has a `type` and an optional `id`, a label carried into exports.

| `type` | Fields | Converts `v` to |
|---|---|---|
| `identity` | none | `v` |
| `linear` | `a`, `b` | `a * v + b` |
| `polynomial` | `coeffs` (list) | `coeffs[0] + coeffs[1] * v + coeffs[2] * v^2 + ...` |

The streamer publishes and the archiver stores raw volts. Calibration is
applied when the data is read: the webapp applies it for plots, and the
exporter writes both the raw and the calibrated value to every CSV row. The
archiver stores the calibration that was active in each Parquet file's
metadata, and starts a new file when it changes, so old files keep the
calibration they were recorded with.
