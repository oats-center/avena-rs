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
per million, the streamer compares the two every 60 seconds and shifts the
timeline when they differ by 5 ms or more. Timestamps can therefore step by a
few milliseconds, forward or backward, at a correction. Sort by timestamp
rather than relying on file order.

## Parquet archive

The archiver writes one file per channel per time window:

```text
<parquet_dir>/
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

Each file holds the samples of one channel for one aligned window of
`rotate_secs` (normally :00 to :05, :05 to :10 and so on, in UTC). A window can
be split across two files if the archiver restarted during it, and a file can
be shorter than a window at the start or end of recording.

### Schema

| Column | Parquet type | Meaning |
|---|---|---|
| `timestamp_unix_ns` | `INT64`, required | Sample time, Unix epoch, nanoseconds |
| `value` | `DOUBLE`, required | Raw reading in volts, `NaN` for a skipped sample |

Key-value metadata:

| Key | Value |
|---|---|
| `calibration` | The channel's calibration when the file was written, as JSON, e.g. `{"id":"tp3505","type":"linear","a":70.25,"b":-9.1068}` |

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
