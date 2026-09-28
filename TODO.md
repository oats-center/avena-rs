# TODO

Known issues and pending work. Each item should become a GitHub issue.

## Rust services

- [x] **`ParquetLogger::close` can panic** (`rust-ljm/src/store.rs`). Its internal
  `flush` unwraps writer calls, so a failed write panics instead of returning an
  error. Separately, if only the directory fsync fails after the rename, the file
  is published but its JetStream messages stay unacked and are written a second
  time later.
- [x] **Subject fallbacks disagree** (`rust-ljm/src/subjects.rs`). When a
  configuration has no identity fields, live channel subjects fall back to
  `asset<NNN>` for the source, while the stream wildcard and the export subject
  use `unknown-source`, so the stream no longer matches its own channels.
- [x] **`subscriber` file names** (`rust-ljm/src/subscriber.rs`). With the default
  legacy wildcard it receives every asset but names every CSV after
  `ASSET_NUMBER`.

## Webapp

- [x] **Svelte config typo** (`webapp/svelte.config.js`). `complierOptions` should be
  `compilerOptions`; `runes: true` is currently ignored.
- [x] **Config form channel order** (`LabJackConfigModal.svelte`,
  `handleChannelToggle`). Enabling a channel numbered below an enabled one
  misaligns `data_formats` and `measurement_units`. Also check whether Cancel
  undoes nested edits, since the form works on a shallow copy.
- [x] **Trigger capture drawing** (`RealTimePlot.svelte`). In trigger mode the y
  range is captured once, and samples arriving while collecting can be drawn
  outside the plot area.
- [x] **Webapp and Rust disagree** (`webapp/src/lib/subjects.ts`,
  `webapp/src/lib/calibration.ts`). `sanitizeToken` collapses runs of whitespace,
  `.` and `/` into one `-` while Rust maps each to its own `-`, so such names give
  different subjects. An empty polynomial calibration is identity in the webapp
  but 0 in Rust, and linear `a`/`b` given as strings are dropped.
- [x] **Exports** (`webapp/src/lib/exporter.ts`, `nats.svelte.ts`). A request to an
  offline box has no no-responders handling and waits the full ten-minute
  timeout. The whole CSV is held in memory. `updateConfig` and `deleteKey` leave
  their connection open when the write fails.
- [x] **Plot page** (`routes/labjacks/plots/[asset_number]/+page.svelte`).
  - Only the newest message per channel is decoded every 100 ms, so above ten
    messages a second per channel the plot drops data.
  - Raw values with magnitude 100 or more are dropped before calibration.
  - Retry and reload open new NATS connections without closing the old ones; the
    list page never closes its connection.
  - Editing a configuration whose site, box or source changed saves it under the
    old key.
  - An invalid asset number shows the loading spinner forever.

## Operations

- [x] **Recompress the archives.** Done on MU1 and MU2 on 2026-09-28.
- [ ] **Find what makes the archiver replay its backlog.** Both boxes re-archived the
  whole JetStream stream several times (MU1 up to 12 copies of each window between
  Aug 27 and Sep 23, MU2 about 2). The durable consumers were never re-created, and
  plain archiver restarts on 2026-09-28 did not replay; the bursts line up with boots
  and NATS restarts. `dedupe` removed the copies and the exporter now skips duplicates,
  but the cause is still open.
- [ ] **Add `wait-for-local-nats` to the installer.** The script and the
  `10-nats-ready.conf` drop-ins for the three services are installed by hand on both
  boxes but are not in `scripts/`.

## Still open after the bug fixes

- [ ] Plot page: the "Connected" badge does not follow later disconnects; a reload or
  Retry closes a connection an export in progress is using; continuous autoscale only
  widens; the plot redraws only when data arrives.
- [ ] Exports are kept in memory as Blob parts; streaming straight to disk needs the
  save picker opened from the export click handler.
- [ ] Rust treats an explicit empty identity field differently from the webapp in the
  KV key (`labjackConfigKey` falls back to `unknown-source`, subjects to `asset<NNN>`).
- [ ] `svelte-check` reports three type errors in `LabJackConfigModal.svelte`
  (`.a`, `.b`, `.coeffs` on a union type).
- [ ] Browser check against a live box: 100 Hz and 1 kHz or faster plots, trigger
  modes, gap drawing on restart, one WebSocket after Retry/reload, and moving a
  configuration to a new box ID.
