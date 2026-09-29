# Data formats

## Live messages

Each message on a live subject is one FlatBuffer `Scan` from
`rust-ljm/src/data.fbs`:

```text
namespace sampler;

table Scan {
  first_sample_unix_ns: ulong;   // time of values[0], Unix epoch, nanoseconds
  sample_interval_ns: ulong;     // time between consecutive values, nanoseconds
  actual_scan_rate_hz: double;   // scan rate the LabJack reported, Hz
  sequence: ulong;               // batch counter, starts at 0 when streaming starts
  values: [double];              // raw readings for one channel, volts
}
```

The time of `values[i]` is `first_sample_unix_ns + i * sample_interval_ns`.
The values are raw volts; calibration is applied by whoever reads them.

A message carries one read for one channel, so it holds `scans_per_read`
values. All channels from the same read share `first_sample_unix_ns` and
`sequence`. `sequence` restarts at 0 every time the stream restarts, for
example after a configuration change; a gap in it means messages were lost.

A value of `NaN` marks a sample the LabJack skipped. LJM fills lost scans with
a placeholder when its buffer overflows, and the streamer converts the
placeholder to `NaN` so it cannot be mistaken for a reading.

The generated bindings are committed: `rust-ljm/src/data_generated.rs` for Rust
and `webapp/src/lib/sampler/` for TypeScript. After changing `data.fbs`,
regenerate both from the repository root:

```bash
flatc --rust -o rust-ljm/src rust-ljm/src/data.fbs
flatc --ts --gen-object-api -o webapp/src/lib rust-ljm/src/data.fbs
```

## Timestamps

The LabJack does not timestamp samples. The streamer stamps the first read of a
stream with the system clock and counts forward using the sample interval.
Because the LabJack's crystal and the system clock drift apart by a few parts
per million, the streamer nudges the timeline toward the system clock by at
most 1 ms a minute, and never by more than half a sample interval, so
timestamps within a run always increase. Two things can still move timestamps
in one jump: the first minute of a run, when the initial anchor is corrected
once, and a system clock step of 2 s or more that lasts three minutes. Both are
logged. Sort by timestamp rather than relying on file order.

## Parquet archive

The archiver writes one file per channel per time window:

```text
<parquet_dir>/
  .archiver-state/
    labjacks/
      avenars.i69.i69-mu1.i69-lj2.live.ch08.json
  asset1001/
    2026-09-23/
      ch08/
        part-0091.parquet
        part-0092.parquet
        part-0093.parquet.inprogress
      ch09/
      ...
```

- `asset<NNN>` is the asset number, padded to three digits.
- The date folder is the UTC date of the samples in the file.
- `part-NNNN` counts up within a day and channel. The number carries no time
  information; read the timestamps.
- `.parquet.inprogress` is the file for the window being recorded. It stays
  empty on disk until the window closes.
- `*.quarantined-*` files are unfinished files set aside after a crash; see
  [Troubleshooting](../operations/troubleshooting.md#quarantined-files-appear).
- `.archiver-state/` holds the archiver's replay-guard checkpoints, described
  [below](#replay-guard-checkpoints). It is not part of the data.

Each file holds the samples of one channel for one aligned window of
`rotate_secs` (normally :00 to :05, :05 to :10 and so on, in UTC). A window can
be split across two files if the archiver restarted during it, and a file can
be shorter than a window at the start or end of recording.

Archives written before the replay guard can hold the same samples more than
once, in extra part files for the same window, because the archiver was fed
the same JetStream messages again. The exporter sends each exact copy once,
and `dedupe` rewrites such folders (see [Command-line
tools](tools.md#dedupe)).

### Schema

| Column | Parquet type | Meaning |
|---|---|---|
| `timestamp_unix_ns` | `INT64`, required | Sample time, Unix epoch, nanoseconds |
| `value` | `DOUBLE`, required | Raw reading in volts, `NaN` for a skipped sample |

Key-value metadata:

| Key | Value |
|---|---|
| `calibration` | The channel's calibration when the file was written, as JSON, e.g. `{"id":null,"type":"linear","a":70.25,"b":-9.1068,"unit":"kPa"}`. Older files may carry an `id` and no `unit`. |

### Encoding

Files written since September 2026 have a single row group, zstd compression,
`DELTA_BINARY_PACKED` timestamps and dictionary-encoded values. The
timestamps are almost perfectly regular and the values come from a 16-bit
converter, so both compress very well: a five-minute file at 2 kHz is about
0.5 MB instead of about 7 MB. Older files use many 1,000-row groups without
compression; `recompress` converts them (see [Command-line
tools](tools.md#recompress)). Readers do not need to care which kind a file is.

### Reading the archive

Any Parquet reader works. In Python:

```python
import pyarrow.parquet as pq, json

table = pq.read_table("part-0092.parquet")
calibration = json.loads(pq.ParquetFile("part-0092.parquet").metadata.metadata[b"calibration"])
```

## Replay-guard checkpoints

The archiver keeps one small JSON file per channel under
`<parquet_dir>/.archiver-state/<stream>/<subject>.json` (or under
`ARCHIVER_STATE_DIR`):

```json
{
  "stream": "labjacks",
  "stream_created_unix_ns": 1790000000000000000,
  "subject": "avenars.i69.i69-mu1.i69-lj2.live.ch08",
  "archived_through_seq": 812345
}
```

`archived_through_seq` is the consumer's ack floor: every message on that
subject up to this stream sequence is inside a closed, synced Parquet file. The
file is replaced atomically and synced, about once a minute and when the
channel stops. It is trusted only while the stream name, its creation time and
the subject match and the stream has not been rewound below it; see
[Troubleshooting](../operations/troubleshooting.md#the-archiver-writes-old-data-again).
Deleting the folder is safe: the guard then starts again from the consumer's
current progress.
