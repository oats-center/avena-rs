# MU1 Remote Deployment Validation — 2026-08-05

This report records work performed remotely on the `i69-mu1` LattePanda. It
does not claim MU2, PiKVM, cold-boot, power-cut, Wattdog, or ADC validation.

## Identity and configuration

- `hostname` and `hostnamectl --static` both returned `i69-mu1`.
- `tailscale status --self` identified `100.64.0.32` as `i69-mu1`.
- The installed profile uses `edge-i69-mu1`, KV key
  `i69.i69-mu1.i69-lj2.config`, LabJack IP `192.168.1.111`, and expected serial
  `470036312`.
- Central and local KV payload SHA-256 values matched:
  `8dfd34633838a4cc0173474bcefc33bb53c70b34bc467a46fa36d3b382fc4e39`.
- Separate committed MU1 and MU2 profiles render to independent bundles. The
  rendered MU2 identity expects serial `470036330`; it has not been installed
  or hardware-verified because MU2 is offline.

The MU1 streamer log recorded:

```text
Loaded initial config from KV 'avenabox:i69.i69-mu1.i69-lj2.config'
[labjack] connected via ETHERNET, serial 470036312, ip 192.168.1.111, self-test ok
[run #1] Streaming started: 100 scans/read @ 500 Hz
[run #1] Derived sample interval: 2000000 ns from actual scan rate 500 Hz
```

The active JetStream stream includes the current subject
`avenars.i69.i69-mu1.i69-lj2.live.*`. Its historical `v1` subject is retained
intentionally so the existing messages and consumers are not invalidated.

## Persistent services

The following reported both `enabled` and `active`:

```text
avena-streamer.service
avena-archiver.service
avena-exporter.service
avena-health-metrics.timer
```

The exporter log showed its worker subscription:

```text
[exporter] worker listening on NATS subject 'avenars.i69.i69-mu1.i69-lj2.export.request'
```

The installation is reproducible with:

```bash
./scripts/install-edge-services.sh \
  --profile shared/edge-boxes/i69-mu1.json \
  --start
```

No reboot was performed because both PiKVMs were offline. Enabled-after-reboot
and cold-boot ordering therefore remain deployment-gate items.

## Live acquisition and config sync

A central-NATS capture of the current wildcard received the following for both
enabled channels over 5.598 seconds:

| Channel | Rows | Batches | Sequence range | Contiguous | First sample | Last sample |
|---|---:|---:|---:|---|---|---|
| ch11 | 2,800 | 28 | 2366–2393 | yes | 15:35:57.239624943Z | 15:36:02.837624943Z |
| ch13 | 2,800 | 28 | 2366–2393 | yes | 15:35:57.239624943Z | 15:36:02.837624943Z |

Each batch contained 100 samples; the FlatBuffer sample interval was 2 ms.
This is the configured 500 samples/second per channel and approximately five
batches/second.

Central KV was changed from channels `[11,13]` to `[11]` and restored to
`[11,13]`. Local KV mirrored both revisions. The streamer stopped and restarted
the LabJack run, while the archiver closed ch13 before removal and attached it
again after restoration. Central and local KV hashes match after restoration.

## Archive safety

- Active parts use `.parquet.inprogress`; only successfully closed parts have
  the `.parquet` suffix.
- Stopping `avena-archiver` closed the active ch11 and ch13 writers and left no
  in-progress files. The service was restarted afterward.
- The channel removal/restoration test closed ch13 instead of aborting its
  writer. The subsequent central export read the affected range without a
  skipped-file error.
- Startup found and preserved 11 pre-existing corrupt historical parts using
  `.corrupt.quarantined-...` names.
- Automated tests cover atomic publication of a readable one-row part and
  quarantine of deliberately unfinished/corrupt files.

## Ten-minute central NATS export

The export was requested without the webapp using
`scripts/request-nats-export.mjs`:

```text
subject: avenars.i69.i69-mu1.i69-lj2.export.request
range:   2026-08-05T15:25:45Z through 2026-08-05T15:35:45Z
asset:   1001
channels: 11, 13
bytes:   52,721,036
chunks:  101
rows:    599,102 plus header
missing channels: none
```

Both ch11 and ch13 contributed 299,551 rows. The first exported timestamp was
`15:25:45.001805388Z` and the last was `15:35:44.999624943Z`. CSV columns were
`timestamp,channel,raw_value,calibrated_value,calibration_id`, and both channels
reported calibration ID `identity`.

The local generated artifact is
`target/evidence/2026-08-05-mu1/mu1-2026-08-05T152545Z-10min.csv` (52.7 MB,
SHA-256 `53de3e916f652d8e056bbedcb484065ff09d6f92267913840edb3ec8179b3ef5`).
The adjacent `export-report.json` preserves the request and counts. `target/`
is intentionally git-ignored, so the large CSV is not committed.

The 898-row difference from the theoretical 600,000 total is explained by the
deliberate channel-disable and streamer-restart test within this range; the
same gap occurs on both channels and the cadence capture after restoration is
contiguous.

## Monitoring

- Alloy was reconfigured without the XFS collector; no prior repeating XFS
  collection error has appeared since the 11:31 EDT restart. The required
  one-hour observation is not yet complete.
- Central Prometheus receives metrics labeled with
  `box=instance=server=i69-mu1`.
- `avena_service_state` exposes one-hot `running`, `failed`, and `stopped`
  states for streamer, archiver, and exporter. A controlled exporter stop
  produced `stopped=1`; restart restored `running=1`.
- Central Prometheus also received last stream time, last completed Parquet
  time, quarantine count (`11`), and leaf connections (`1`).

## Repository verification

After these changes:

- `cargo test --all-targets`: 25 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- `pnpm run build` in `webapp`: passed.
- `./scripts/build-docs-site.sh`: passed.
- Shell syntax, Node syntax, and independent MU1/MU2 render-value checks:
  passed.

## Remote blockers

At the end of this validation, Tailscale returned `no matching peer` for:

```text
100.64.0.170  i69-mu1-kvm
100.64.0.127  i69-mu2-kvm
100.64.0.154  i69-mu2
```

Consequently, MU2 installation/validation and any reboot that would require
PiKVM recovery remain open.
