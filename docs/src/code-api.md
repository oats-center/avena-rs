# API reference

The reference pages are generated from the source code on every build, so they
always match the code on `main`.

## Rust services

Each program in `rust-ljm` is its own binary, so each has its own reference.
Private items are included, because almost everything in a binary is private
and that is where the interesting code lives.

| Binary | Role |
|---|---|
| [`streamer`](api/rust/streamer/index.html) | LabJack acquisition and NATS publishing |
| [`archiver`](api/rust/archiver/index.html) | JetStream consumer and Parquet writer |
| [`exporter`](api/rust/exporter/index.html) | CSV export worker |
| [`subscriber`](api/rust/subscriber/index.html) | Diagnostic live capture to CSV |
| [`recompress`](api/rust/recompress/index.html) | Rewrites old Parquet files in the current format |

## Webapp library

The TypeScript modules under `webapp/src/lib` are documented in the
[webapp reference](api/webapp/index.html).
