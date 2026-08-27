# MU1 PiKVM / Wattdog Power-Control Handoff

Last updated: 2026-08-05 after the 19:36 UTC PiKVM reboot.

## Objective

Diagnose the MU1 enclosure power instability, then validate Wattdog-controlled
low-voltage shutdown and high-voltage recovery without accidentally stranding
the PiKVM or corrupting the LattePanda filesystem.

Do not issue a GPIO write, restart a power-related service, change the PSU, or
remove Wattdog `--dry-run` without first telling the onsite operator exactly:

1. What command/change will run.
2. Which equipment should lose power.
3. How long it should remain off.
4. How recovery will occur without the SSH connection.
5. What the operator must do if automatic recovery fails.

Wait for explicit operator approval immediately before a disruptive action.

## Access and topology

- MU1 LattePanda:
  - hostname: `i69-mu1`
  - Tailscale: `100.64.0.32`
  - enclosure LAN observed as `192.168.1.106/24`
- MU1 PiKVM:
  - hostname: `pikvm`
  - Tailscale: `100.64.0.170`
  - enclosure LAN: `192.168.1.100`
- External Prometheus: `http://100.64.0.174:9090`
- Credentials were supplied by the operator out of band. They are deliberately
  not stored in this file.

The enclosure LAN is the most reliable way to reach PiKVM while MU1 is up. If
the protected-load breaker/relay removes power from MU1 and the modem, remote
SSH and Tailscale disappear even though PiKVM itself remains powered. Any
recovery action must therefore be scheduled and executed locally on PiKVM
before the protected load is switched off.

## Physically confirmed power topology

The onsite operator manually opened the Phoenix Contact breaker while watching
the enclosure:

- PiKVM stayed powered.
- LattePanda, modem, and other protected equipment turned off.

This confirms PiKVM is supplied upstream of the protected-load breaker. The
photographed breaker is a Phoenix Contact `TMC 71B 07A`, item `1019914` (7 A).

PiKVM software labels the three GPIO outputs as:

| Logical channel | GPIO | UI label | Current initial state |
| --- | ---: | --- | --- |
| `relay1` | 26 | LattePanda Mu | `true` |
| `relay2` | 20 | Main Power (only for testing) | `true` |
| `relay3` | 21 | Links & LabJack | `true` |

All three use `inverted: true`. Relay 2 is the leading candidate for the master
protected-load relay because its label and middle physical position agree with
the breaker wiring, but **relay 2 has not yet been verified by an actual GPIO
operation**.

PiKVM GPIO configuration is in `/etc/kvmd/override.yaml`. At the last check all
three logical states were `true`.

## Immediate power-supply diagnosis

The variable PSU was reported as:

- Voltage: `15.6 V`
- Current limit: `1 A`
- Maximum available enclosure power at that setting: `15.6 W`

This is very likely the cause of the unexplained resets. Official requirements
used in the diagnosis:

- LattePanda Mu full evaluation carrier: minimum 45 W supply for the
  LattePanda alone.
- PiKVM V3-class hardware: recommended 5.1 V, 3 A supply (about 15 W before
  conversion losses).
- The modem, LabJack, links, DC/DC losses, and startup inrush are additional.

The proposed bench setting is to leave voltage at 15.6 V and raise the PSU
current limit to 5 A, **only if the PSU and all wiring/connectors are rated for
it**. This remains below the photographed 7 A branch breaker. The operator has
not yet confirmed that this change was made. A current limit is a ceiling; it
does not force that current through the load.

Before any relay test, confirm physically:

- PSU is at 15.6 V with an appropriate current limit (proposed 5 A).
- PSU remains in constant-voltage (`CV`) mode during boot and normal load.
- The `CC`/current-limit indicator does not illuminate during startup.
- The system remains stable for at least ten minutes.

Do not infer total enclosure load from the current Wattdog readings. The two
sensors recently reported approximately `+0.296 A` and `-0.296 A`; their exact
placement/orientation has not been documented, and those values are too small
to establish total enclosure demand.

## Current Wattdog deployment

PiKVM runs Wattdog as a Podman Quadlet:

- Unit: `wattdog.service`
- Quadlet: `/etc/containers/systemd/wattdog/wattdog.container`
- Config: `/etc/containers/systemd/wattdog/config.toml`
- Image: `ghcr.io/oats-center/wattdog:main`
- Metrics: `http://127.0.0.1:9107/metrics`
- Persistent samples: `/var/lib/wattdog` inside the container volume

The running command was last verified as:

```text
/usr/local/bin/wattdog --config /config.toml --dry-run
```

Wattdog is intentionally unable to move a relay in its present state:

- `--dry-run` is active.
- Action URLs point to the non-routable placeholder `127.0.0.1:9`.
- No local relay action bridge exists yet.

Current state configuration:

```toml
[[states]]
name = "i69-mu1"
serial = "239148418773806"
field = "voltage1_volts"
default_state = "on"
stale_after = "45s"

[states.on]
op = ">="
value = 15.4
duration = "10m"
url = "http://127.0.0.1:9/i69-mu1/load/on"

[states.off]
op = "<="
value = 13.4
duration = "2m"
url = "http://127.0.0.1:9/i69-mu1/load/off"
```

Detected Thornwave devices:

| Serial | Advertised name | Voltage field used/observed |
| --- | --- | --- |
| `239148418773806` | `Battery` | `voltage1_volts` (selected state input) |
| `212412767172514` | `PowerMon` | `voltage1_volts` |

Both devices advertise reliably while the enclosure is powered. Voltage 2 has
reported an implausible `131.071 V` and must not be used.

## Proposed final safety behavior (not applied)

The final configuration should be fail-safe:

- PiKVM `relay2`: `initial: false` so a PiKVM reboot does not immediately
  energize protected loads.
- `relay1` and `relay3`: retain `initial: true` if relay 2 is confirmed as the
  master upstream switch for those branches.
- Wattdog `default_state = "off"`.
- Wattdog OFF: measured Thornwave bus `<= 13.4 V` continuously for 10 seconds.
- Wattdog ON: measured Thornwave bus `>= 15.4 V` continuously for 10 minutes.
- Stale/missing telemetry must never energize the protected load.
- Wattdog action endpoint must be local, authenticated, and fixed to relay 2;
  callers must not be able to select arbitrary GPIO channels.

The 13.4 V and 15.4 V thresholds were explicitly approved by the operator.
They apply to the Thornwave bus measurement, not necessarily the PSU display.
Observed PSU-to-sensor difference has been approximately 0.2 to 0.3 V.

The proposed 10-second OFF delay replaces the current two-minute delay because
the prior sudden-drop test lost connectivity before two minutes elapsed. The
10-minute ON delay is intended to prevent chatter and require sustained charge.

Do not apply `initial: false` or remove `--dry-run` until the tests below pass.

## Incident timeline and evidence

All times below are UTC on 2026-08-05 unless stated otherwise.

### Initial threshold dry run

- About 18:22: Wattdog was changed from placeholder sensor/thresholds to the
  real `Battery` serial and 13.4/15.4 V thresholds, and explicitly placed in
  `--dry-run`.
- 18:29:06: external Prometheus first recorded the selected sensor below
  13.4 V, around 13.01 V.
- 18:30:21: last real scrape before the first outage, only about 75 seconds
  after the low condition began.
- Wattdog never completed its two-minute OFF condition. The OFF transition
  series did not exist; no physical action was possible.
- Around 18:35: PiKVM rebooted and logged undervoltage. `vcgencmd
  get_throttled` later returned `0x50000` (historical undervoltage/throttling).
- At PSU approximately 13.9 V, the bus was about 13.57 V.
- At PSU approximately 15.8 V, the bus was about 15.56 V.

### Manual breaker test and later restarts

- 18:59:35 to 19:06:51: off-box Prometheus had no new scrape. This is
  consistent with the operator opening the protected-load breaker: modem and
  MU1 lost power/network while PiKVM visibly remained powered.
- After telemetry returned, Wattdog state was unknown until approximately
  19:13:10, then returned to the configured default ON state. There was no OFF
  transition.
- 19:14:35: last external sample before another hard restart was about
  13.715 V.
- PiKVM booted around 19:15 and logged undervoltage; MU1 also performed XFS log
  recovery after its unclean shutdown.
- All primary MU1 project services subsequently returned active.

### Planned relay-2 test was not executed

After the operator said “Do it,” both systems disconnected before any command
was issued. The assistant's execution record contains no relay write in that
interval. Device-side checks after recovery found:

- No `avena-relay2-*` transient service or timer.
- No retained relay-2 switch command in root shell history.
- No Wattdog OFF transition.
- Wattdog still in dry-run with dummy action URLs.
- External voltage stayed around 15.35 to 15.37 V until the next telemetry gap.
- PiKVM itself rebooted around 19:36, which the intended protected-load relay
  test should not cause because PiKVM is upstream.

Root history contains one older command targeting relay 1 through a different,
apparently invalid API path. It is unrelated to the unexecuted relay-2 test.

## Useful read-only checks

Run these before changing anything:

```bash
date -u
uptime
systemctl status wattdog bluetooth kvmd --no-pager
podman inspect wattdog --format '{{json .Config.Cmd}}'
curl -fsS http://127.0.0.1:9107/metrics \
  | grep -E '^wattdog_(voltage1_volts|current_amperes|power_watts|state_)'
grep -nE 'relay[123]|initial|inverted' /etc/kvmd/override.yaml
journalctl -b -u wattdog.service --no-pager
journalctl -b -k --no-pager \
  | grep -Ei 'under.?voltage|throttl|brown|recover|error'
vcgencmd get_throttled
```

GPIO state can be read through the local API after supplying credentials
interactively; do not embed credentials in scripts or this file.

External Prometheus survives loss of the enclosure and can be queried through
its HTTP API. Use `timestamp(metric)` rather than ordinary range-query
evaluation timestamps to distinguish a real scrape from Prometheus's stale
lookback value.

## Required test order

### 1. Stabilize the source

Raise the PSU current limit only after confirming ratings, keep voltage at
15.6 V, and observe at least ten minutes with no current-limit indication or
reboot. Record PSU voltage/current and Thornwave values.

### 2. Verify relay 2 and BLE survival with autonomous recovery

Keep the current `initial: true` setting for this test so rebooting PiKVM is a
physical fallback.

Before switching OFF:

1. Start a local PiKVM logger that records timestamps, GPIO state, Wattdog
   observation count, both sensor ages/voltages, and action output.
2. Create an independent PiKVM-local recovery timer that unconditionally sets
   relay 2 ON after 15 seconds.
3. Verify the recovery timer is armed.
4. Tell the operator that MU1/modem and this remote session should disappear,
   but PiKVM must remain powered.
5. Wait for explicit “ready/trigger” approval.

Then switch relay 2 OFF. Expected behavior:

- Protected loads go OFF.
- PiKVM and its local logger continue.
- Relay 2 returns ON after 15 seconds without SSH/network.
- MU1 and modem take roughly three to five minutes to boot/reconnect.
- The retained PiKVM log proves whether the Battery sensor kept advertising
  while protected power was absent.

If the Battery sensor stops advertising while relay 2 is OFF, automatic
high-voltage recovery cannot work. Stop and rewire/repower the selected sensor
upstream before proceeding.

For this test only, if power does not return after 30 seconds, the onsite
operator may power-cycle only PiKVM. The current `initial: true` configuration
should restore relay 2. A full PSU cycle is the last resort.

### 3. Validate thresholds in dry-run

Change only the proposed state defaults/delays while retaining `--dry-run`.
Prove:

- OFF transition after ten continuous seconds at measured bus <=13.4 V.
- No OFF transition from a brief transient.
- ON transition only after ten continuous minutes at measured bus >=15.4 V.
- No ON transition for voltage between the thresholds.
- Restart/stale input produces OFF/unknown behavior, never an ON action.

### 4. First live Wattdog cycle

Install and test a restricted local relay-2 action bridge, then remove
`--dry-run` only for a controlled cycle with the operator present. Retain
PiKVM `relay2 initial: true` until Wattdog has independently restored power
successfully. Collect GPIO, Wattdog, voltage, service recovery, filesystem, and
data-continuity evidence.

### 5. Apply boot-safe final state

Only after live OFF/ON recovery works, set relay 2 `initial: false`. Test a
PiKVM cold boot at a voltage between thresholds: protected loads must remain
OFF. Raise measured voltage above 15.4 V and prove Wattdog restores protected
loads after the full ten-minute delay.

Repeat at least five complete discharge/recharge cycles before deployment.

## Current safety state at handoff

- No relay test is armed or scheduled.
- All relay logical states were last observed `true`.
- Wattdog is active, receiving both BLE devices, and remains in `--dry-run`.
- Wattdog action URLs remain harmless placeholders.
- No final GPIO/default-state configuration has been applied.
- Do not continue until the 1 A PSU limit is corrected and stability is
  demonstrated.
