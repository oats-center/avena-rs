# rust-ljm

The Rust services that run on each Avena edge node.

| Binary | Source | Role |
|---|---|---|
| `streamer` | `src/main.rs` | Streams the LabJack T7 and publishes samples to the local NATS server |
| `archiver` | `src/store.rs` | Consumes the samples from JetStream and writes Parquet files |
| `exporter` | `src/exporter.rs` | Answers export requests over NATS with CSV read from the archive |
| `subscriber` | `src/subscriber.rs` | Diagnostic capture of live samples to CSV |
| `recompress` | `src/recompress.rs` | Rewrites old archive files in the current compressed format |

```bash
cargo build --release          # all binaries, into target/release
cargo test --release           # unit tests
cargo doc --no-deps --document-private-items --open
```

Building needs the LabJack LJM library only at run time (`dynlink`, the
default). The NATS integration test is ignored by default; run it against a
local JetStream-enabled server with
`AVENA_TEST_NATS_URL=nats://127.0.0.1:4222 cargo test --release -- --ignored`.

On an edge node the binaries are installed and run by
`scripts/install-edge-services.sh`, never from `target/`.

Full documentation, including configuration, data formats and the export
protocol, is at <https://oats-center.github.io/avena-rs/> (source in `../docs`).
