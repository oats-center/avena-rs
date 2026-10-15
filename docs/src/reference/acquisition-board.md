# Acquisition board interface

This page is the hand-off for the group building a board to replace the LabJack
T7. It has three parts: what the software does with the T7 today, what the
I-69 records taught us about the measurement, and a proposed interface (version
1, draft) that the board firmware and our software can both be built against.
The wire format is defined exactly in
[`daq-protocol/avena_daq.h`](https://github.com/oats-center/avena-rs/blob/main/daq-protocol/avena_daq.h);
this page explains it.

Everything after the streamer (NATS, the archiver, exports, the webapp) stays as
it is. Only the part of the streamer that talks to the T7 is replaced.

## How the software uses the T7 today

### Connection and set-up

The streamer (`rust-ljm/src/main.rs`, `labjack.rs`) uses LabJack's closed-source
LJM library over Ethernet:

1. Open the device by IP address (`LABJACK_IP`), check it is a T7 and, if set,
   that its serial number matches `LABJACK_SERIAL`.
2. Self-test: read `STREAM_SETTLING_US` and write it back. If the write fails
   because a stream is still running from an earlier session, stop that stream
   and try again.
3. Write four settings, the same for every input:

   | Register | Value | Meaning |
   |---|---|---|
   | `AIN_ALL_NEGATIVE_CH` | 199 | single-ended: every input measured against ground |
   | `AIN_ALL_RANGE` | 10.0 | ±10 V range |
   | `AIN_ALL_RESOLUTION_INDEX` | 0 | default resolution (16-bit effective at these rates) |
   | `STREAM_SETTLING_US` | 0 | default settling between channels |

4. Start streaming the enabled inputs (`AIN<n>`) at `scan_rate_hz`, asking for
   `scans_per_read` scans per read. The T7 answers with the rate it actually
   runs at.

### Data received

Each read returns a flat array of 64-bit floats in volts, interleaved by scan:
`ch_a, ch_b, ch_c, ch_a, ch_b, ch_c, ...`, `scans_per_read` scans long. When the
T7's buffer overflows it inserts `-9999.0` for every missing sample; the
streamer turns those into NaN. There is no timestamp and no sample counter in
the data: the streamer times every sample itself from the moment it started the
stream and the nominal rate, and slowly corrects that timeline against the
computer clock (at most 1 ms a minute).

### What the software sends on

For each read and each channel the streamer publishes one FlatBuffer `Scan`
(`rust-ljm/src/data.fbs`) on `avenars.<site>.<box>.<source>.live.ch<NN>`:

```text
table Scan {
  first_sample_unix_ns: ulong;   // time of the first sample
  sample_interval_ns: ulong;     // 1e9 / rate, rounded to whole ns
  actual_scan_rate_hz: double;
  sequence: ulong;               // read counter, from 0 at each start
  values: [double];              // volts, NaN for a missing sample
}
```

The archiver stores those raw volts in Parquet; calibration (volts to kPa or µε)
and the noise filters are applied later, when data is plotted or exported.

### What the configuration controls

The box configuration in the key-value store (see [LabJack
configuration](kv-config.md)) sets: the enabled inputs, the scan rate, the
scans per read, and on/off. Calibration, units and filters are also in it, but
the T7 never sees them. Nothing in the configuration sets input mode, range or
excitation today; those are fixed in code (single-ended, ±10 V) or in the
external amplifier boards.

### Current load

MU1 records 4 channels at 100 Hz, MU2 4 channels at 2 kHz. Even 32 channels at
8 kHz as 32-bit integers is about 1 MB/s, so bandwidth is not a design
constraint anywhere in this chain.

## What the I-69 records showed

Each of these cost weeks of analysis. The board should make them impossible or
measurable. Details are in [Noise on the I-69 inputs](../noise.md) and the
escalation and strain follow-up reports.

| Problem found | Cause | What the board should do |
|---|---|---|
| A 430 Hz bump, an 870 Hz hump, wandering tones near 150 Hz | Switching-regulator noise near 22 and 30 kHz folded into the record: the T7 has no anti-aliasing filter in stream mode | Anti-aliasing on every input: an analog RC (or better) plus a sigma-delta ADC whose digital filter removes everything above the output band |
| A 10 Hz and an 11.9 Hz square wave on the sensor wiring; the 10 Hz one follows the T7's clock | Something in the box, tied to the acquisition, couples into the inputs | Differential inputs, clean linear analog supplies, no periodic loads or switchers near the front end, a ground plan |
| Strain units unknown (is 1 V = 1 mV/V, or −3.95 mV/V?) | The gauge amplifier's gain and excitation are not documented or measured | Measure bridges ratiometrically and report V/V directly; read back the excitation; switchable shunt-calibration resistors |
| Pressure cells stuck at 3.7 V, one open | A wiring or sensor-supply fault nobody could see from the data | Per-channel excitation voltage and current readback, open-input detection |
| Timestamps that drift and need slewing; the 2200 Hz rate ran 100 ppm off | No sample counter or timestamps from the device; requested rates are rounded inside the T7 | The board counts samples and timestamps them from its own disciplined clock; only exact rates are allowed |
| Channels sampled one after another, with no settling time | The T7 multiplexes one ADC | Simultaneous sampling of all channels |
| 16-bit resolution on a ±10 V range: 0.3 mV per count, about 0.15 µε at the stored strain scale | One fixed range for every sensor | 24-bit ADC with a programmable range per channel |
| A proprietary library (LJM) that has to be installed on each box | Vendor design | A small open protocol over TCP |

## Proposed board requirements

These are the measurement needs as we understand them now. The board group
should push back where something is costly.

**Inputs**

- 8 analog inputs per board (4 in use per box today), each configurable as
  differential voltage, single-ended voltage or bridge (ratiometric).
- Bridge inputs for Geokon 3900 asphalt strain gauges: 350 Ω full bridges,
  resting output up to about ±10 mV/V, signals of a few µε (a few µV/V).
  Excitation 2.5 or 5 V from a low-noise linear source, sensed at the
  connector if possible.
- Voltage inputs for RST LPTPC09-S pressure cells: 0 to about 5.2 V output,
  9 to 28 V sensor supply (12 V or 24 V from a filtered supply), with current
  readback.
- Range per channel from about ±20 mV to ±10 V.
- One switchable shunt-calibration resistor per bridge (about 100 kΩ across one
  arm, roughly 0.87 mV/V on a 350 Ω bridge), controllable while recording.
- Input protection for long field cables (tens of metres): common-mode range,
  surge and ESD protection.

**Conversion**

- 24-bit simultaneous-sampling sigma-delta ADC, so the digital decimation filter
  gives the anti-aliasing, plus a simple analog RC in front.
- Exact output rates from the ADC clock, at least 100, 250, 500, 1000, 2000,
  4000 and 8000 samples per second per channel. 2 kHz is what we use for axle
  pulses (about 12 ms wide); 8 kHz is headroom.
- Candidate parts to evaluate (not a recommendation yet): TI ADS131M08
  (8 channels, simultaneous, internal reference; measure the excitation on a
  spare channel for the bridge ratio), AD7768-4 or AD7768-1, TI ADS1278.

**Timing**

- A sample counter that never resets during a stream.
- A timestamp of the first sample in each frame from a clock disciplined in this
  order of preference: GNSS PPS input, IEEE 1588 on the Ethernet port, or the
  host's clock through `TIME_REQUEST` messages. With a disciplined clock the host
  no longer has to guess sample times.
- An optional sync output (open-drain or an LED in the camera's view) that the
  host can pulse, to line video up with the strain record.

**Diagnostics**

Board temperature, input supply and analog rail voltages, per-channel excitation
voltage and current, open-input detection, overrange flags.

**Interface**

100 Mbit Ethernet to the edge computer (a direct cable today), the protocol
below, and enough RAM to hold at least 10 s of data at the highest rate if the
host stalls. Calibration to engineering units and all noise filtering stay in
our software, so the firmware never needs to know what a sensor is.

## Protocol version 1 (draft)

### Transport and framing

- One TCP connection. The board listens on port 7600 and accepts one host.
  Little-endian, packed structures, no padding.
- Every message is a 16-byte header, a payload, and a CRC-32 over both.

| Header field | Type | Meaning |
|---|---|---|
| `magic` | u32 | `0x51445641`, the bytes `A V D Q` |
| `version` | u8 | 1 |
| `type` | u8 | message type, below |
| `reserved` | u16 | 0 |
| `payload_len` | u32 | payload bytes (at most 1 MiB) |
| `msg_counter` | u32 | +1 per message in this direction on this connection |

A message with a bad magic, version, length or CRC is answered with `ERROR`
(`BAD_MESSAGE`) and the board closes the connection.

### Messages

| Type | Name | Direction | When |
|---|---|---|---|
| 0x01 | `HELLO` | board → host | first message after connecting |
| 0x02 | `CONFIGURE` | host → board | before `START`; not while streaming |
| 0x03 | `CONFIGURE_ACK` | board → host | reply to `CONFIGURE` |
| 0x04 | `START` | host → board | start streaming with the accepted configuration |
| 0x05 | `STOP` | host → board | stop streaming |
| 0x06 | `STOPPED` | board → host | reply to `STOP`, with totals |
| 0x10 | `DATA` | board → host | one frame of samples, while streaming |
| 0x11 | `STATUS` | board → host | once a second, always |
| 0x12 | `ERROR` | board → host | any failure |
| 0x20 / 0x21 | `TIME_REQUEST` / `TIME_REPLY` | host ↔ board | clock sync when there is no PPS or PTP |
| 0x30 | `SET_SHUNT` | host → board | switch one shunt resistor, also while streaming |
| 0x31 | `SET_SYNC_OUT` | host → board | pulse the sync output |

**`HELLO`** says who the board is: model, serial number, hardware and firmware
versions, a `boot_id` that changes on every reset, the number of inputs, ADC bits,
the largest frame, a capability bitmask (simultaneous sampling, PPS, PTP, bridge
excitation, sensor supply, shunt calibration, sync output) and the list of exact
rates it supports, each as a fraction `num / den`.

**`CONFIGURE`** carries one rate from that list, the samples per channel in each
frame, and one entry per enabled input: input number, mode (off, differential,
single-ended, bridge), full-scale range in mV, excitation in mV (0 = off) and the
initial shunt state. Channels not listed are off. The order of the list is the
order of channels in every `DATA` frame.

**`CONFIGURE_ACK`** accepts or rejects the whole configuration (status 0, or an
error code and nothing changed). For every channel it returns the range actually
set and a `scale` and `offset` such that **value = scale × count + offset**, in
volts, or in V/V for bridge channels. It also gives the ADC's group delay, which
the board has already taken out of the timestamps.

**`DATA`** is one frame:

| Field | Type | Meaning |
|---|---|---|
| `stream_id` | u32 | from `START` |
| `frame_index` | u32 | 0, 1, 2 … since `START` |
| `first_sample_index` | u64 | samples per channel since `START`; jumps over a gap |
| `first_sample_time_ns` | u64 | Unix time of the first sample (centre of its conversion) |
| `flags` | u32 | gap before this frame, time valid (clock locked), first frame |
| `overrange_mask` | u32 | bit *n*: channel in list position *n* clipped in this frame |
| `shunt_mask` | u32 | bit *n*: shunt on for channel in position *n* |
| `n_channels`, `n_samples` | u8, u16 | shape of the sample block |
| samples | i32 × n_channels × n_samples | channel-major: all of channel 0, then channel 1 … |

`INT32_MIN` marks a sample that could not be measured. The time of sample *k*
in a frame is `first_sample_time_ns + k × 1e9 × den / num`, computed exactly by
the host.

**`STATUS`** reports board time, uptime, clock state (free running, host sync,
PTP, PPS, holdover) and offset, temperature, supply and analog rail voltages,
send-buffer fill, frames dropped since boot, and per input: open-input and
excitation-fault flags, measured excitation voltage and current.

### Behaviour

- The ADC never waits for the network. If the host is too slow the board drops
  whole frames from the oldest end, sets `GAP_BEFORE` on the next frame and
  advances `first_sample_index` past the missing samples.
- If the TCP connection drops, the board stops streaming and waits for a new
  connection. After any reset it sends a new `boot_id`; the host then configures
  and starts again.
- `CONFIGURE` while streaming is refused (`BUSY`). Changing a setting means
  `STOP`, `CONFIGURE`, `START`, which is what the streamer already does today.
- Shunt resistors can be switched while streaming; each frame says which were on,
  so a calibration step is recorded in place.
- Firmware updates, network settings and factory calibration of the board itself
  are outside this protocol.

## How it plugs into our software

- **Streamer:** a second back end next to the LJM one, selected by
  `source_type` (`labjack` or `avena-daq`). It connects, sends `CONFIGURE` from
  the box configuration, and turns each `DATA` frame into one `Scan` per
  channel: values = scale × count + offset (NaN for `INT32_MIN`), time from
  `first_sample_time_ns` when the clock is locked, otherwise from the existing
  slewed timeline. Bridge channels are stored in mV/V, so a Geokon gauge's
  calibration is just its certificate factor in µε per mV/V.
- **`Scan` message:** add `first_sample_index`, `flags` (gap, overrange, shunt,
  time source) and the input mode at the end of the table. FlatBuffers allows
  added fields, so the archiver, exporter and webapp keep working and can start
  using them later.
- **Configuration:** per channel add `mode`, `range_mv`, `excitation_mv`; add a
  shunt-cal switch in the webapp. Rates offered in the webapp come from the
  board's `HELLO`.
- **Health:** `STATUS` goes into the existing health metrics (excitation,
  open inputs, clock state, drops).
- **Simulator:** before the hardware exists we write a board simulator that
  speaks this protocol (synthetic sensors, gaps, overranges, clock states). The
  firmware group can test against our host code with it, and we can test the
  streamer back end without a board.

## Choices that keep both sides simple

1. **Integer counts plus one scale per channel**, not floating-point volts, and
   no calibration in firmware. The board reports what it measured; our software
   already does units and filtering.
2. **The board owns time.** A sample counter and a timestamp per frame remove
   most of the streamer's clock-correction code and every timing question we had
   to answer from the data.
3. **Only exact rates**, advertised by the board. No rounding surprises.
4. **Push, not poll.** The board sends frames as they fill; the host only reads.
5. **One small open protocol** for set-up, data, status and errors, instead of a
   vendor library and register map. A C header is the specification.
6. **Fixed-size binary structs**, easy on a microcontroller and easy to parse in
   Rust. FlatBuffers or Protobuf on the board would add a dependency for no gain.
7. **Not NATS on the board** in version 1. Publishing straight to NATS is possible
   but moves buffering, configuration and reconnection logic into firmware; the
   streamer already does that well.

## Open questions for the board group

- Channels per board, and whether one board serves a whole box (strain gauges and
  pressure cells together).
- Power budget (the boxes run on solar and battery) and the temperature range
  inside the enclosure.
- Which time reference is realistic: GNSS antenna on the box, PTP from the edge
  computer's Ethernet port, or host sync only.
- Whether the existing gauge amplifier boards (`schematics/sg-gain-link`,
  `epc-gain-link`) stay in front of the new board or are replaced by its own
  front end.
- Connector and cable standard, and lightning and surge protection for the field
  runs.

## Acceptance tests

- Noise floor with shorted inputs and with a 350 Ω dummy bridge, per range.
- Aliasing: inject tones at 1.1, 2.1, 30 and 60 kHz; nothing should appear in a
  2 kHz record above the noise floor.
- Crosstalk between channels, and coupling from the board's own supplies (no
  lines at 10 Hz or any other fixed frequency).
- Timing: with a PPS reference, timestamps within 10 µs of GNSS time; with
  host sync, within 1 ms; no drift of the sample counter against the timestamps.
- Shunt calibration step size against the calculated value, on each bridge input.
- 72 hours of streaming with the host disconnected and reconnected, a full
  buffer, a power cycle and a rate change, checking counters, gaps and flags.
