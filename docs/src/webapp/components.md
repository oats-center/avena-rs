# Components

The webapp's components live in `src/lib/components/`. None talks to NATS: the
pages load and save data, and the components display and edit it. That keeps the NATS
code in one place per page and makes the components easy to reason about.

## RealTimePlot

`src/lib/components/RealTimePlot.svelte` draws one channel as a line on a
`<canvas>`. The plot page subscribes to the channel, applies the calibration,
and passes the samples in.

The time axis puts 0 at the newest sample in continuous mode and at the trigger
in frozen mode. Its ticks fall on round steps (1, 2 or 5 times a power of ten)
counted from 0, in seconds or milliseconds, and the title says which: "Time
(s)", or "Time from trigger (s)" for a capture. Ticks are about 70 pixels apart;
on a narrow plot they move closer, down to what the labels need, so at least a
few show without overlapping. The value axis is titled
"Value (unit)", and the tick labels are kept clear of the title.

**Continuous mode** shows the last `timeWindow` seconds with the newest sample
at the right. The right edge is the newest sample's own time, not the browser
clock, so a slow link does not leave a blank strip at the edge.

**Frozen mode** is used after a trigger fires. It shows the samples from
`frozenPreWindowSec` before to `frozenPostWindowSec` after the trigger, with
the trigger at 0 s, and holds still so the event can be inspected. A badge
reads COLLECTING while the post-trigger samples are still arriving, then
FROZEN.

Drawing is batched to one redraw per animation frame. Each frame keeps only the
samples inside the window and reduces them to one minimum and one maximum per
pixel column. At 2 kHz a ten-second window holds 20,000 samples for a plot a
thousand pixels wide, so drawing every sample would be slow, and keeping the
minimum and maximum (rather than averaging) means a one-sample spike is still
visible. The line breaks wherever a sample is missing or the stream has a
gap, so a gap is never drawn over. With autoscale on, the y axis snaps to round 1, 2 or 5 steps, and in
continuous mode it only widens, so the scale does not jump with every frame.

Under the canvas the plot shows:

- **Data Points**: the samples inside the visible window, not everything held
  in memory.
- **t=0 Src**: the source clock of the sample at 0.
- **Lag**: how long that sample took to reach the browser. It is hidden while a
  trigger capture is held, since a held capture is not live.
- **Latest**: the latest value, in `unit`.
- **Raw**: when `calibrated` is set, the latest reading in volts before
  calibration.

| Prop | Type | Default | Meaning |
|---|---|---|---|
| `data` | `DataPoint[]` | | Live samples. `timestamp` is Unix epoch milliseconds; the optional `sourceTimestamp` and `receivedAt` (also epoch ms) feed the clock and lag badges. |
| `unit` | `string` | | Unit label for the axis, threshold and badges. The plot page passes `V` for an uncalibrated channel and the calibration's unit for a calibrated one. |
| `calibrated` | `boolean` | `false` | The values are calibrated; shows the Raw badge. `data` points then carry the reading before calibration in `raw`. |
| `timeWindow` | `number` | | Width of the continuous window, seconds |
| `mode` | `'continuous' \| 'frozen'` | | Which mode to draw |
| `isTriggered` | `boolean` | | Whether a trigger has fired |
| `triggerTime` | `number` | | Trigger time, epoch ms; `0` for none |
| `frozenData` | `DataPoint[]` | | Samples around the trigger, for frozen mode |
| `frozenPreWindowSec` | `number` | `timeWindow` | Seconds shown before the trigger |
| `frozenPostWindowSec` | `number` | `timeWindow` | Seconds shown after the trigger |
| `frozenCollecting` | `boolean` | `false` | Show COLLECTING instead of FROZEN |
| `showTriggerThreshold` | `boolean` | `false` | Draw the trigger level line, and a "Trig level" label above the plot |
| `triggerThreshold` | `number` | | Trigger level, in `unit` |
| `prebuffering` | `boolean` | `false` | Show the PREBUFFERING badge |
| `yAutoScale` | `boolean` | `true` | Fit the y axis to the data |
| `yMin`, `yMax` | `number` | `-1`, `1` | Fixed y limits when autoscale is off |
| `invertX`, `invertY` | `boolean` | `false` | Mirror an axis |
| `tag` | `string` | `""` | Badge naming what is plotted; the channel card passes `FILTERED` while filtered values are drawn |

The component emits no events. It exports one function,
`getDisplayedYRange()`, which returns the y range of the last drawn frame; the
channel card uses it to start Y Min and Y Max from what is on screen when Auto
Y-Scale is turned off.

## Plot page components

The plot page is split into four components. All of them only display and edit
state the page owns.

- **`ChannelCard`** shows one plotted channel: the header with the data format,
  the unit tag, the trigger state and, for a channel with filters, the Filtered
  / Raw switch and a line naming the filters (and any that cannot run at the
  channel's rate), the Mode & Axis panel, the Trigger Settings panel in the
  trigger modes, and its `RealTimePlot`. Props: `view`
  (the channel's `ChannelView`), `dataFormat`, `unitInfo` and `onchange`,
  called after a change that alters what the plot shows.
- **`StatsPanel`** is the Data Statistics card. Props: `config`, `assetNumber`,
  `channels` (the plotted ones), `views` and `connectionState`.
- **`ExportDialog`** is the Export Historical Data form. Props: `channels`,
  `selected`, bindable `start` and `end`, `error`, `warning`, `exporting`,
  `progress`, `total`, `filteredChannels` (channels with filters; the "Include
  filtered values" box shows when there are any), bindable `includeFiltered`,
  and the callbacks `ontoggle`, `onsubmit` and `onclose`.
- **`ConnectionBanner`** is the yellow Reconnecting or red Disconnected banner.
  Props: `state`, `reason` and `onreconnect`.

## LabJackConfigModal

`src/lib/components/LabJackConfigModal.svelte` is the form for adding or
editing one [LabJack configuration](../reference/kv-config.md). It edits a
copy of the document and hands the result to `onSave`; the LabJack list page
writes it to the `avenabox` bucket. When adding, the page builds the key from
`site_id`, `box_id` and `source_id`; when editing, it keeps the existing key.
An empty source falls back to the LabJack name and then to `asset<NNN>`, the
same order the live and export subjects use.

The form covers every field of the document: the identity fields, the subject
root and stream, `rotate_secs`, and under `sensor_settings` the scan rate,
scans per read, gain, on/off switch, enabled channels, and for each enabled
channel its sensor type, calibration, unit and noise filters. Saving validates
the whole form first;
while adding, a name or asset number already used by another configuration
is flagged as you type. Escape, the close button, Cancel or a click outside
the form closes it without saving; if anything was changed, it asks first.

| Prop | Type | Meaning |
|---|---|---|
| `config` | `LabJackConfig` | The document to edit, or defaults for a new one. Copied once when the form opens. |
| `isAddingNew` | `boolean` | Adding a new LabJack: turns on the duplicate checks and changes the titles |
| `existingLabJacks` | `Map<string, LabJackConfig>` | All loaded configurations by key, for the duplicate checks |
| `onSave` | `(config) => void` | Called with the edited document once it validates |
| `onClose` | `() => void` | Called to close the form |

### Channel calibration

Each channel has exactly one calibration, edited in place and saved with the
rest of the form. There are no named presets and nothing to name.

- **Sensor type** (stored in `data_formats`): Voltage, Strain gauge, Pressure,
  Temperature, Current or Resistance. It decides which units are offered.
- **Calibration**: None (raw volts), Linear (`a·x + b`) or Polynomial
  (`c0 + c1·x + c2·x² …`), where `x` is the raw reading in volts.
- **Unit**: V for None. A linear or polynomial calibration must have a unit;
  switching to one picks the sensor type's first unit (µε for a strain gauge,
  kPa for pressure). Strain gauges offer µε, mV/V and V; pressure kPa, Pa, bar
  and PSI.

The card header sums the channel up: sensor type, unit, the formula and what a
raw reading of 1.000 V becomes, for example
`y = 481.26·x + 1058.722 · raw 1.000 V → 1539.98 µε`.

For a strain gauge with a linear calibration you can type `a` and `b` or use
the **bridge helper**. It takes the gauge's calibration factor from its
certificate (µε per mV/V, for example 481.26 for SG194 or 705.47 for SG159),
the bridge excitation in volts, the amplifier gain (may be negative) and,
optionally, the raw reading at rest, and computes

```text
µε = factor × 1000 × (raw − zero) / (excitation × gain)
a  = factor × 1000 / (excitation × gain)
b  = −a × zero
```

It shows `a`, `b` and the 1 V preview before you apply them. What is stored is
a plain linear calibration with unit µε; the helper's inputs are not saved.

What the form saves for a channel, by sensor type:

```json
{"type": "identity", "unit": "V"}
{"type": "linear", "a": 481.26, "b": 1058.722, "unit": "µε"}
{"type": "linear", "a": 250.0, "b": -125.0, "unit": "kPa"}
{"type": "polynomial", "coeffs": [0.5, 100.0, 0.2], "unit": "°C"}
```

On save the calibration's unit is also written to the channel's
`measurement_units` entry, and a calibrated channel's `data_formats` entry is
set to a sensor type that offers the unit, so older readers see the same
labels. An `id` from an older configuration is kept until the formula is
changed; the form never asks for one.

Configurations saved before calibrations had units still open: a calibration
without `unit` takes the channel's `measurement_units` entry unless it is V.
If it is V the unit box is empty and yellow, and the form will not save until a
unit is chosen. The `calibration.*` preset keys that earlier versions wrote are
no longer read.

### Channel filters

Under each channel's calibration is a Filters box: Remove spikes, Remove 10 Hz
and Remove 11.9 Hz switches, and High-pass and Low-pass cutoffs in Hz (blank
turns one off). They are saved in `sensor_settings.filters` (see [LabJack
configuration](../reference/kv-config.md#filters)). The box names the filters
that cannot run at the configured scan rate, and how far behind the live plot
runs because of the despike window. A channel with every filter off has no
entry, and a configuration with no filters has no `filters` field. Turning a
channel off drops its filters.
