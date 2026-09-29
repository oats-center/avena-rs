# TODO

Open issues and pending work. Each item should become a GitHub issue. The nine
bugs listed here earlier, the archive recompression and the duplicate cleanup
were fixed in #21 and deployed to MU1 and MU2 on 2026-09-29.

## Rust services

- [x] **Find what makes the archiver replay its backlog.** Both boxes re-archived the
  whole JetStream stream several times (MU1 up to 12 copies of each window between
  Aug 27 and Sep 23, MU2 about 2). The durable consumers were never re-created, and
  plain archiver restarts did not replay; the bursts line up with boots and NATS
  restarts. `dedupe` removed the copies and the exporter now skips duplicates. Cause: nats-server renames a consumer's state file
  without an fsync, so a power cut on XFS can leave it empty and the consumer
  restarts from the beginning of the stream. The archiver now keeps its own synced
  checkpoint and skips messages it already archived.
- [x] **Export cancel has no protocol message.** After the webapp cancels, the edge
  exporter keeps sending up to 8 more chunks until its ack timeout (about 30 s).
  A cancel frame would stop it at once.
- [x] **Empty identity fields in the KV key.** Subjects fall back to `asset<NNN>`
  for a missing or empty source, but the webapp's `labjackConfigKey` falls back to
  `unknown-source`. Pick one.

## Webapp

- [x] **Split the plot page.** `routes/labjacks/plots/[asset_number]/+page.svelte`
  is about 2,000 lines with parallel per-channel maps. Move to one state object per
  channel plus `ChannelCard`, `ExportDialog` and `StatsPanel` components.
- [x] **Unticking Auto Y-Scale** starts the limits at -1 and 1 instead of the
  current auto range, so the plot jumps.
- [x] **A reload or Retry closes the connection** an export in progress is using.
- [x] **Stream exports to disk.** Exports are kept in memory as Blob parts;
  streaming to a file needs the save picker opened from the export click handler.
- [x] **Small cleanups:** the inline invalid-asset branch in the plot page is now
  unreachable (`+page.ts` handles it); the trigger plot shows the level twice
  (label box and LEVEL badge); at 390 px the time axis shows only two ticks.

## Still open

- [ ] **Scan rates the T7 can't hit exactly.** At 2200 Hz the LJM library reported
  2200 Hz, but the MU2 data drift about 100 ppm (0.36 s an hour) against the sample
  count, which is more than the clock slew can absorb (60 ms an hour). Check the
  rate the T7 actually runs at (for example read `STREAM_SCANRATE_HZ` back after
  starting, or compute it from the T7's clock divisor) and use that for the sample
  interval. 100, 2000 and 2500 Hz are not affected.

- [ ] `downloadExportViaNats` keeps its own in-memory copy of an export even when
  the webapp streams it to a file. An `onChunk` hook awaited before each ack would
  remove that copy and the chunk tap in `lib/plot/export-sink.ts`.
- [ ] Consider `sync_interval: always` in `nats-leaf.conf` so nats-server fsyncs its
  state; weigh the extra writes on the box disks first.

## Setup and operations

- [x] **Add `wait-for-local-nats` to the installer.** The script and the
  `10-nats-ready.conf` drop-ins for the three services are installed by hand on
  both boxes but are not in `scripts/`.
- [ ] **Data caveat.** MU1 timestamps from the 2026-09-25 reboot to the
  2026-09-28 21:28 UTC deploy carry the old per-minute re-anchoring jitter
  (plus or minus 60 to 430 ms).

## Field (MU1 and MU2 hardware)

Details and evidence are in the escalation note and the strain follow-up report.

- [ ] Pressure cells read about 3.7 V instead of a few tenths of a volt: check
  excitation and signal wiring at the terminals.
- [ ] 10 Hz and 11.9 Hz square waves, one-sample spikes and a tone near 2.1 kHz on
  the sensor inputs: power one sensor from a battery or linear supply while
  recording to find the source.
- [ ] Strain scale: shunt-calibrate SG194 and SG159 while recording, and read the
  strain board's gain resistor and excitation.
- [ ] A few passes with a known truck at 2 kHz, pass times noted to the second.
