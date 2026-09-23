# Components

The webapp has two reusable components. Neither talks to NATS: the pages load
and save data, and the components display and edit it. That keeps the NATS
code in one place per page and makes the components easy to reason about.

## RealTimePlot

`src/lib/components/RealTimePlot.svelte` draws one channel as a line on a
`<canvas>`. The plot page subscribes to the channel, applies the calibration,
and passes the samples in.

**Continuous mode** shows the last `timeWindow` seconds with the newest sample
at the right. The right edge follows the browser clock, unless the newest
sample lags it by more than 75 ms or a tenth of the window, whichever is
larger; then it follows the newest sample, so a slow link does not leave a
blank strip at the edge.

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
visible. The line breaks where the data has a gap wider than a quarter of the
plot. With autoscale on, the y axis snaps to round 1, 2 or 5 steps, and in
continuous mode it only widens, so the scale does not jump with every frame.

Under the canvas the plot shows the number of samples drawn, the source clock
of the sample at t = 0, its lag behind the browser, and the latest value.

| Prop | Type | Default | Meaning |
|---|---|---|---|
| `data` | `DataPoint[]` | | Live samples. `timestamp` is Unix epoch milliseconds; the optional `sourceTimestamp` and `receivedAt` (also epoch ms) feed the clock and lag badges. |
| `unit` | `string` | | Unit label for the axis, threshold and badges |
| `timeWindow` | `number` | | Width of the continuous window, seconds |
| `mode` | `'continuous' \| 'frozen'` | | Which mode to draw |
| `isTriggered` | `boolean` | | Whether a trigger has fired |
| `triggerTime` | `number` | | Trigger time, epoch ms; `0` for none |
| `frozenData` | `DataPoint[]` | | Samples around the trigger, for frozen mode |
| `frozenPreWindowSec` | `number` | `timeWindow` | Seconds shown before the trigger |
| `frozenPostWindowSec` | `number` | `timeWindow` | Seconds shown after the trigger |
| `frozenCollecting` | `boolean` | `false` | Show COLLECTING instead of FROZEN |
| `showTriggerThreshold` | `boolean` | `false` | Draw the trigger level line |
| `triggerThreshold` | `number` | | Trigger level, in `unit` |
| `prebuffering` | `boolean` | `false` | Show the PREBUFFERING badge |
| `yAutoScale` | `boolean` | `true` | Fit the y axis to the data |
| `yMin`, `yMax` | `number` | `-1`, `1` | Fixed y limits when autoscale is off |
| `invertX`, `invertY` | `boolean` | `false` | Mirror an axis |

The component emits no events.

## LabJackConfigModal

`src/lib/components/LabJackConfigModal.svelte` is the form for adding or
editing one [LabJack configuration](../reference/kv-config.md). It edits a
copy of the document and hands the result to `onSave`; the LabJack list page
writes it to the `avenabox` bucket. When adding, the page builds the key from
`site_id`, `box_id` and `source_id`; when editing, it keeps the existing key.

The form covers every field of the document: the identity fields, the subject
root and stream, `rotate_secs`, and under `sensor_settings` the scan rate,
scans per read, gain, on/off switch, enabled channels, and for each enabled
channel its format, unit and calibration. A calibration can be chosen from
saved presets or saved as a new one. Saving validates the whole form first;
while adding, a name or asset number already used by another configuration
is flagged as you type. Escape, the close button, Cancel or a click outside
the form closes it without saving.

| Prop | Type | Meaning |
|---|---|---|
| `config` | `LabJackConfig` | The document to edit, or defaults for a new one. Copied once when the form opens. |
| `isAddingNew` | `boolean` | Adding a new LabJack: turns on the duplicate checks and changes the titles |
| `existingLabJacks` | `Map<string, LabJackConfig>` | All loaded configurations by key, for the duplicate checks |
| `availableCalibrations` | `Map<string, CalibrationSpec>` | Saved calibration presets by id |
| `onSaveCalibration` | `(spec) => Promise<boolean>` | Saves a calibration preset; resolves `true` on success |
| `onSave` | `(config) => void` | Called with the edited document once it validates |
| `onClose` | `() => void` | Called to close the form |

Calibration presets are stored in the same `avenabox` bucket under
`calibration.<id>`, separate from the per-box configuration keys.
