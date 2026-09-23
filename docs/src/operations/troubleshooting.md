# Troubleshooting

Start with `/home/user/avena-status`. It usually points at the layer that is
broken. The sections below go from the bottom of the stack to the top.

## The local NATS server is down

`curl http://127.0.0.1:8222/varz` fails, and the Rust services are stopped
because they require it.

- `sudo journalctl -u nats-leaf -n 120 --no-pager` usually says why.
- `/home/user/nats` must exist and be writable.
- `/etc/containers/systemd/creds/leaf.creds` must exist.
- After fixing, `sudo systemctl restart nats-leaf`, then start the Rust
  services in order (see [Everyday tasks](tasks.md)).

## No leaf connection to central

`curl -s http://127.0.0.1:8222/leafz | jq .leafnodes` prints `0`. The box keeps
recording, but nothing reaches central and exports time out.

- Check that `nats1.oats` and `nats2.oats` resolve and that Tailscale is up.
- `nc -zv nats1.oats 7422` must connect.
- If it connects but the leaf does not come up, the leaf credentials are wrong
  or not authorized on central; the `nats-leaf` journal shows the rejection.

## The streamer is stopped

After five LabJack failures in a row the streamer exits on purpose and stays
stopped.

```bash
journalctl -u avena-streamer -n 120 --no-pager
ping -c 3 <labjack ip>
```

Common causes: the LabJack lost power or its network link, it came back on a
different address (it needs a DHCP reservation), or a different T7 was
connected and its serial does not match the profile. Fix the cause, then
`sudo systemctl start avena-streamer`.

## The configuration does not update

A change made in the webapp should appear in the streamer log within a few
seconds.

- Compare the central and local copies with `nats kv get` (central without
  `--js-domain`, local with `--js-domain edge-<box>`).
- `nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds rtt` must
  work from the box; the mirror uses a direct client connection.
- Search the streamer log for `central_kv_sync` errors.

## Live plots do not move

- On the command line, subscribe to the box's live subjects through central
  (see [The webapp](../setup/webapp.md)). If messages arrive there, the problem
  is in the browser: check the configuration key in the plot address.
- If nothing arrives centrally but the streamer is running, check the leaf
  connection.

## Exports fail or return nothing

- The exporter must be running: `systemctl status avena-exporter`.
- An empty result for a channel means there are no closed Parquet files in that
  range. The current five-minute window is not exported until it closes.
- A request that times out immediately with an empty reply means nothing is
  listening on the export subject: the exporter is down or the box is offline.

## Quarantined files appear

On startup the archiver renames any `part-NNNN.parquet.inprogress` file left by
a crash or power cut to
`part-NNNN.parquet.inprogress.unfinished.quarantined-<milliseconds>-<n>`. That
is expected after an unclean shutdown. The samples in it are not lost: their messages were never
acknowledged, so JetStream delivers them again once the consumer's `ack_wait`
has passed (about 18 minutes) and they are written into a new file. The
quarantined file can be deleted once the new file exists.

## The readings themselves look wrong

The software records whatever voltage arrives at the LabJack input. Two
patterns seen in the field point at wiring or power rather than software:

- Several channels sitting at the same voltage, well outside the sensor's
  normal range, usually means the inputs are not connected to live sensors.
- A square wave at an exact frequency on the sensor inputs, but not on unused
  inputs or the LabJack's internal ground (`AIN15`), is entering through the
  sensor wiring or its power supply.

Measure at the LabJack terminals before changing any software.
