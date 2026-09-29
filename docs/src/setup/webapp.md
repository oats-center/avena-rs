# The webapp

The webapp runs in your browser and talks only to central NATS, over WebSocket.
It lists the LabJack configurations in the central `avenabox` bucket, edits
them, plots live channels, and downloads archived data as CSV. It never
connects to an edge node directly, so it works from anywhere on the Tailscale
network.

## Run it

You need Node.js 18 or newer and pnpm.

```bash
cd avena-rs/webapp
corepack enable        # provides pnpm, if it is not installed already
pnpm install
pnpm dev
```

Open the address Vite prints, normally <http://localhost:5173>.

## Connect

On the first page, enter the central WebSocket address and choose a
credentials file:

| Setting | Value |
|---|---|
| Server URL | `ws://nats1.oats:8080` |
| Credentials | a `.creds` file allowed to read and write the `avenabox` bucket and subscribe to the `avenars.>` subjects |

The address and credentials are kept in the tab's session storage until you
log out or close the tab, and are only used to connect to NATS.

## Plot a box

The LabJack page lists every configuration in `avenabox`, one per edge node.
The Enabled or Disabled badge on each card is the configuration's
`labjack_on_off` setting, not a live status.
Opening one shows its live channels. Asset numbers are not guaranteed to be
unique between boxes, so the plot address carries the configuration key:

```text
/labjacks/plots/1001?key=i69.i69-mu1.i69-lj2.config
```

The plot page shows up to two channels at a time and receives only the ones
you select, so selecting a channel again starts an empty plot. Each channel
says whether it shows raw volts or calibrated values and in which unit; a
calibrated channel also shows its latest raw reading in volts. The unit is the
one set with the channel's calibration in the configuration form; if a channel
says "unit not set", edit the configuration and choose it.

The connection badge reads Connecting, Connected, Reconnecting or
Disconnected. When the connection drops, a banner says the page is
reconnecting, and the plots carry on by themselves afterwards with a gap for
the missing time. If a plot stays empty, check on the command line that samples
are arriving at central NATS:

```bash
nats --server nats://nats1.oats:4222 --creds apt.creds \
  sub 'avenars.i69.i69-mu1.i69-lj2.live.>' --count 3
```

## Change what a box records

Editing a configuration in the webapp writes the central key. The box's
streamer mirrors the change within a moment and restarts sampling with the new
settings; the archiver starts a new file when a channel's calibration changes.
Each channel has one calibration, set in the form together with its unit (for
example kPa for pressure or µε for a strain gauge); a strain gauge's
calibration can be worked out from its certificate factor, the bridge
excitation and the amplifier gain with the form's bridge helper.
The [LabJack configuration reference](../reference/kv-config.md) explains each
field.

## Download data

The export dialog starts with the plotted channels and the last two minutes.
Times are to the second in your browser's time zone, which the dialog names,
and the same range is shown in UTC underneath. Cancel Download stops a running
export and saves nothing; the box may still send a few more chunks before it
notices, for up to about 30 seconds. The request goes through
central NATS to the exporter on that box, which reads its Parquet archive and
streams the rows back as CSV. Large ranges take a while: the edge node's
uplink is usually the limit.

The same request can be made from the command line, which is handy for
scripting and for checking the export path without a browser. See
[Command-line tools](../reference/tools.md).

For how the webapp is put together, and how to build and host it for others,
see [How the webapp works](../webapp/architecture.md) and [Developing and
deploying](../webapp/development.md).
