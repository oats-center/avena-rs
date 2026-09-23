# TODO

Known issues and pending work. Each item should become a GitHub issue.

## Rust services

- [ ] **`ParquetLogger::close` can panic** (`rust-ljm/src/store.rs`). Its internal
  `flush` unwraps writer calls, so a failed write panics instead of returning an
  error. Separately, if only the directory fsync fails after the rename, the file
  is published but its JetStream messages stay unacked and are written a second
  time later.
- [ ] **Subject fallbacks disagree** (`rust-ljm/src/subjects.rs`). When a
  configuration has no identity fields, live channel subjects fall back to
  `asset<NNN>` for the source, while the stream wildcard and the export subject
  use `unknown-source`, so the stream no longer matches its own channels.
- [ ] **`subscriber` file names** (`rust-ljm/src/subscriber.rs`). With the default
  legacy wildcard it receives every asset but names every CSV after
  `ASSET_NUMBER`.

## Webapp

- [ ] **Svelte config typo** (`webapp/svelte.config.js`). `complierOptions` should be
  `compilerOptions`; `runes: true` is currently ignored.
- [ ] **Config form channel order** (`LabJackConfigModal.svelte`,
  `handleChannelToggle`). Enabling a channel numbered below an enabled one
  misaligns `data_formats` and `measurement_units`. Also check whether Cancel
  undoes nested edits, since the form works on a shallow copy.
- [ ] **Trigger capture drawing** (`RealTimePlot.svelte`). In trigger mode the y
  range is captured once, and samples arriving while collecting can be drawn
  outside the plot area.
- [ ] **Webapp and Rust disagree** (`webapp/src/lib/subjects.ts`,
  `webapp/src/lib/calibration.ts`). `sanitizeToken` collapses runs of whitespace,
  `.` and `/` into one `-` while Rust maps each to its own `-`, so such names give
  different subjects. An empty polynomial calibration is identity in the webapp
  but 0 in Rust, and linear `a`/`b` given as strings are dropped.
- [ ] **Exports** (`webapp/src/lib/exporter.ts`, `nats.svelte.ts`). A request to an
  offline box has no no-responders handling and waits the full ten-minute
  timeout. The whole CSV is held in memory. `updateConfig` and `deleteKey` leave
  their connection open when the write fails.
- [ ] **Plot page** (`routes/labjacks/plots/[asset_number]/+page.svelte`).
  - Only the newest message per channel is decoded every 100 ms, so above ten
    messages a second per channel the plot drops data.
  - Raw values with magnitude 100 or more are dropped before calibration.
  - Retry and reload open new NATS connections without closing the old ones; the
    list page never closes its connection.
  - Editing a configuration whose site, box or source changed saves it under the
    old key.
  - An invalid asset number shows the loading spinner forever.

## Operations

- [ ] **Recompress MU1's archive** once MU1 is back online. Build `recompress` from
  this branch (it includes the fix that skips empty files), run it with
  `--dry-run` first, then for real at low priority, and compare an export from
  before and after.
