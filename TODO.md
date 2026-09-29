# TODO

Open issues and pending work. Each item should become a GitHub issue. The nine
bugs listed here earlier, the archive recompression and the duplicate cleanup
were fixed in #21 and deployed to MU1 and MU2 on 2026-09-29.

## Rust services

- [ ] **Find what makes the archiver replay its backlog.** Both boxes re-archived the
  whole JetStream stream several times (MU1 up to 12 copies of each window between
  Aug 27 and Sep 23, MU2 about 2). The durable consumers were never re-created, and
  plain archiver restarts did not replay; the bursts line up with boots and NATS
  restarts. `dedupe` removed the copies and the exporter now skips duplicates, but
  the cause is still open. A test: restart `nats-leaf` on one box while watching
  the consumers' ack floor and the day folders that get new files.
- [ ] **Export cancel has no protocol message.** After the webapp cancels, the edge
  exporter keeps sending up to 8 more chunks until its ack timeout (about 30 s).
  A cancel frame would stop it at once.
- [ ] **Empty identity fields in the KV key.** Subjects fall back to `asset<NNN>`
  for a missing or empty source, but the webapp's `labjackConfigKey` falls back to
  `unknown-source`. Pick one.

## Webapp

- [ ] **Split the plot page.** `routes/labjacks/plots/[asset_number]/+page.svelte`
  is about 2,000 lines with parallel per-channel maps. Move to one state object per
  channel plus `ChannelCard`, `ExportDialog` and `StatsPanel` components.
- [ ] **Unticking Auto Y-Scale** starts the limits at -1 and 1 instead of the
  current auto range, so the plot jumps.
- [ ] **A reload or Retry closes the connection** an export in progress is using.
- [ ] **Stream exports to disk.** Exports are kept in memory as Blob parts;
  streaming to a file needs the save picker opened from the export click handler.
- [ ] **Small cleanups:** the inline invalid-asset branch in the plot page is now
  unreachable (`+page.ts` handles it); the trigger plot shows the level twice
  (label box and LEVEL badge); at 390 px the time axis shows only two ticks.

## Setup and operations

- [ ] **Add `wait-for-local-nats` to the installer.** The script and the
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
