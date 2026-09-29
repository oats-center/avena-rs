# How the webapp works

The webapp is three pages, an error page and two components. Every page talks to central
NATS directly from the browser; there is no application server in between.

[![Webapp pages and their NATS traffic](../figures/webapp-flow.svg)](../figures/webapp-flow.svg)

## Connecting

The connect page (`/`) takes the central WebSocket address and a `.creds` file.
It opens a test connection, closes it, and stores the address and the file's
text in the tab's `sessionStorage` as `serverName` and `credentialsContent`.
Each later page reads those two values and opens its own connection. There is
no shared connection object between pages, so a page reload always starts
from a clean connection. A page closes its connection before it retries and
when you leave it, so retrying does not pile up connections.

The credentials file is read when you press Connect, not when you pick it, so a
file chosen while the page is still loading is not lost. Pressing Enter or
clicking Connect again while a test connection is running does nothing.

`connect()` in `src/lib/nats.svelte.ts` accepts a bare host name as well as a
full URL. It builds the `ws://` and `wss://` variants and tries them in order,
and resolves to `null` rather than throwing when none of them works, so the
pages show an error message instead of crashing.

## The LabJack list

`/labjacks` lists every key in the `avenabox` bucket that matches
`*.*.*.config` and shows one card per configuration. It also loads the
calibration presets stored under `calibration.*`. Configurations are read
through `normalizeLabJackConfig()` in `src/lib/labjack-config.ts`, which fills
in missing fields and accepts older field names.

If you have not logged in, the page shows only an error with a link to the
connect page. Adding a configuration is disabled until the page is connected.
The Enabled or Disabled badge on a card is the configuration's
`labjack_on_off` setting. It says whether the box has been told to stream, not
whether the box is actually running.

Adding or editing a configuration opens `LabJackConfigModal`, which edits a copy
and returns it. The page writes the result to
`<site>.<box>.<source>.config`. From there the edge node's streamer picks the
change up within seconds (see [LabJack configuration](../reference/kv-config.md)).
Writes and deletes each open a short-lived connection of their own. Closing the
form with unsaved changes asks first. Saving a calibration preset from the form
writes `calibration.<id>` straight away, after a confirmation, whether or not
the form itself is then saved. Delete sits at the bottom of each card, away from
Edit, and asks for confirmation.

## Live plots

The plot page is `/labjacks/plots/<asset_number>?key=<kv key>`. The key names
the configuration to plot. Asset numbers are not unique across boxes, so the
key is what makes the page unambiguous; without it, the page takes the first
configuration with that asset number.

You choose up to two channels to plot, and the page subscribes to the live
subjects of those channels only, with plain NATS subscriptions. Messages arrive
as FlatBuffer `Scan`s (see [Data
formats](../reference/data-formats.md#live-messages)). Every message is queued
in arrival order until the page next gets to it. The queue is bounded at 5,000
messages per channel; it only fills if the page cannot keep up for a long time,
for example in a background tab that the browser slows down. Then the
oldest messages are dropped, the count shows under Data Statistics, and the
plot shows a gap where they were.

The queued messages are decoded in order. Each sample gets its time from the
`Scan` (first sample time plus its index times the interval), and the channel's
calibration is applied. Missing samples, and missing time between messages,
become gaps in the line rather than being joined across; repeated or late
messages are skipped; and if the timeline jumps backward (a clock step or a new
sampling run) the buffer starts again. The plots are redrawn on the browser's
animation frames. The code for all of this is in `src/lib/plot/`, with unit
tests.

Decoding has one quirk worth knowing. The browser can hand over a message as a
slice of a larger buffer that does not start on an 8-byte boundary, and the
generated FlatBuffers code cannot create a `Float64Array` view on it.
`FlatBufferParser` copies such a slice to a fresh buffer and reads the values
one at a time, so a missing value comes through as a gap.

Each plot is labelled with its channel's own unit from the configuration. The
Y axis limits can be set to any number, zero included. The connection badge
at the top follows the NATS connection's own status, so it shows when the
connection drops and when it comes back. Back returns to the LabJack list.

Each channel runs in one of three modes. **Free Run** plots continuously.
**Trigger Normal** and **Trigger Single** watch for the calibrated value to cross
a level, then freeze a window around the crossing so a single event can be
inspected. Normal re-arms once the post-trigger window has passed; Single keeps
its first capture until you press Re-arm.
Drawing is done by `RealTimePlot` (see [Components](components.md)).

## Exports

The export dialog on the plot page takes a start and end time to the second,
in the time zone you pick, and the channels to export. It builds an export
request and hands it to
`downloadExportViaNats()` in `src/lib/exporter.ts`, which implements the client
side of the [export protocol](../reference/export-protocol.md). It publishes the
request with a reply inbox and an ack subject, acknowledges every chunk as it
arrives, collects the chunks, and when the `complete` frame arrives turns them
into a file download. If no exporter is listening (the box is offline), the
NATS server says so straight away and the export fails with that message. If no
message arrives for ten minutes, it gives up. Cancel stops a running export.

## Current limitations

These are known and worth fixing, but none of them affects recorded data:

- **A plot can show gaps after a long stall.** If the page falls more than
  5,000 messages behind on a channel, the oldest are dropped and counted, and
  the plot shows a gap. A channel that was not selected is not received at
  all, so selecting it again starts with a gap. The archive and exports have
  every sample.
- **Exports are held by the browser until saved.** Received chunks are folded
  into Blob parts every 8 MiB, so little sits in JavaScript memory, but the
  whole file is kept in the browser's Blob storage before it is saved. Split
  very long ranges into several downloads.
- **The credentials sit in `sessionStorage` as plain text** for as long as the
  tab is open. Log out or close the tab on shared machines.
