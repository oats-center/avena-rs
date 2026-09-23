# NATS subjects

All traffic for a LabJack source lives under one prefix built from its
identity:

```text
<root>.<site_id>.<box_id>.<source_id>.<purpose>...
```

The root is `avenars`. Every token is lowercased, and spaces, `.` and `/` are
turned into `-`, so a name can never add a level to the subject or introduce a
wildcard. For site `i69`, box `i69-mu1` and LabJack `i69-lj2`:

| Subject | Direction | Carries |
|---|---|---|
| `avenars.i69.i69-mu1.i69-lj2.live.ch11` | streamer to everyone | Samples from channel 11 (`AIN11`), one [FlatBuffer message](data-formats.md#live-messages) per read |
| `avenars.i69.i69-mu1.i69-lj2.live.*` | | The wildcard captured by the edge node's JetStream stream `labjacks` |
| `avenars.i69.i69-mu1.i69-lj2.export.request` | client to exporter | [Export requests](export-protocol.md) |
| a client inbox, e.g. `_INBOX.…` | exporter to client | Export replies |

Channel numbers are zero-padded to two digits: channel 3 is `ch03`.

Live subjects are plain NATS subjects, forwarded to central NATS over the leaf
connection, so anyone with access can subscribe centrally:

```bash
nats --server nats://nats1.oats:4222 --creds apt.creds \
  sub 'avenars.i69.i69-mu1.i69-lj2.live.>'
```

## Configuration keys

Configuration is not published on a subject you subscribe to. It lives in the
key-value bucket `avenabox` under `<site_id>.<box_id>.<source_id>.config`. Use
`nats kv get` and `nats kv put` rather than the bucket's internal `$KV.…`
subjects.

## Missing identity fields

If a configuration has a structured root but lacks a field, the tokens fall
back to `unknown-site` and `unknown-box`. The source token falls back to the
LabJack name, and then to `asset<NNN>` for live channel subjects or
`unknown-source` for the stream wildcard and the export subject. Those
fallbacks do not agree with each other, so always set `site_id`, `box_id` and
`source_id`.

## Legacy subjects

Configurations from before the structured layout, with a root other than
`avenars` and no identity fields, publish on
`<root>.<asset>.data.<chNN>`, for example `avenabox.1456.data.ch11`, with the
asset number padded to three digits. They are still read correctly. New
configurations should use the structured layout; it identifies the box without
a separate lookup, and asset numbers are not unique across boxes.

## Why one prefix per source

Putting site, box and source in the subject lets NATS permissions and
subscriptions work at any level: `avenars.i69.>` is a whole site,
`avenars.i69.i69-mu1.>` one box. It also lets central NATS route an export
request to exactly one edge node without any lookup, because only that node's
exporter subscribes to its request subject.
