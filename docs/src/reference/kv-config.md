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
      "8": { "type": "linear", "a": 70.25, "b": -9.1068, "unit": "kPa" }
    },
    "filters": {
      "8": { "despike": true, "remove_10hz": true, "remove_11_9hz": true,
             "highpass_hz": 1.0, "lowpass_hz": 40.0 }
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
| `filters` | object | Noise filters for reading the data, keyed by channel number as a string. Optional; see [Filters](#filters). |

`scans_per_read` and `scan_rate_hz` together set how often messages are sent:
100 scans per read at 100 Hz is one message per channel per second. Aim for
about ten messages per channel per second or fewer; more messages mean more
overhead without more data.

## Calibrations

Each entry has a `type`, the formula's fields, an optional `unit` (the
calibrated value's unit, such as `kPa` or `µε`) and an optional `id`, a label
carried into exports. Older configurations name their calibrations with `id`;
new ones usually leave it out. The archiver stores `unit` with the calibration
in each Parquet file.

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

## Filters

Each entry switches on noise filters for one channel. They are applied only where
the data is read: the live plot in the webapp and, when asked for, an extra
`filtered_value` column in exports (see [Export protocol](export-protocol.md)).
The streamer and the archiver ignore them, so the archive always keeps the raw
readings, and changing them restarts nothing. What each filter removes, and why,
is in [Noise on the I-69 sensor inputs](../noise.md).

| Field | Type | Meaning |
|---|---|---|
| `despike` | boolean | Strip narrow upward spikes: running minimum, then running maximum, over `ceil(2.5 ms × rate)` samples. Keeps pulses of about 5 ms and longer. Needs a rate above 400 Hz (two or more samples); ignored below. |
| `remove_10hz` | boolean | Subtract the 10 Hz square wave: a template of one 100 ms cycle, aligned on the sample count and learnt from the last 30 s. Needs `rate / 10` to be a whole number of samples, 4 or more; ignored otherwise. |
| `remove_11_9hz` | boolean | Subtract the 11.906 Hz square wave the same way, with a template in phase bins of its (fractional) period. Needs at least 4 samples per period. |
| `highpass_hz` | number | High-pass cutoff, Hz: 2nd-order Butterworth. Ignored at or above 0.45 × rate. |
| `lowpass_hz` | number | Low-pass cutoff, Hz: 2nd-order Butterworth. Ignored at or above 0.45 × rate. |

Every field is optional; a missing, `false`, `null` or non-positive value turns
that filter off, and a missing map (or channel) means no filtering. They run in
the order of the table. The spike and template stages work on the raw volts, where
the interference adds; the calibration is applied next, and the high-pass and
low-pass filters run on the calibrated values, so a high-passed pressure reading
is centred on 0 kPa rather than on the calibration's offset.

The live plot uses causal filters, which add no delay except the despike window
(`ceil(2.5 ms × rate) − 1` samples, 2 ms at 2 kHz). Exports use the same spike
and template stages and run the high-pass and low-pass filters forward and
backward (zero phase, doubling their order: −6 dB at the cutoff). A webapp from
before this field was added drops it when it saves the configuration.

## No calibration presets

Each channel's calibration lives only inside its box's configuration. Earlier
versions of the webapp also kept named presets under `calibration.<id>` keys in
the same bucket. The services never read them, the webapp no longer does either,
and they have been removed.

