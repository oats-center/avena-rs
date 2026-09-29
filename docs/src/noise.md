# Noise on the I-69 sensor inputs

What the I-69 records carry besides the sensor signal, where each part comes
from, and what signal processing can and cannot remove. Measured on MU1 and MU2
in September 2026 (2 kHz, 2.2 kHz and 2.5 kHz records, the 100 Hz archive, and a
bench test with the streamer stopped). The full analysis is in the strain
follow-up report that accompanies the escalation note.

## Sources

| # | Noise | Cause | Can a filter remove it? |
|---|---|---|---|
| 1 | 10 Hz square wave: ±8 mV on MU1's pressure cells, about 3 mV on the strain gauges | Runs off the LabJack T7's own clock (it stays fixed against the sample count and drifts against UTC at the T7 crystal's 2 ppm) but enters on the sensor wiring: the T7's internal ground and unused inputs are clean. Suspects: a T7 DAC or digital output, the VS (5 V) terminal feeding a sensor board, or a 10 Hz load in the T7 disturbing a shared supply or ground. | Yes. Subtract a template of the 100 ms cycle, aligned on the sample count and rebuilt after each stream start: 27–35 dB. Notch filters do worse and damage vehicle pulses. |
| 2 | Broadband noise that doubles during one half of every 100 ms cycle | The same 10 Hz source switching something on and off | No, it is random. Detection can use only the quiet half of each cycle. |
| 3 | 11.9 Hz square wave (lines at 11.906, 35.72 and 59.53 Hz) | A second periodic source on the wiring, also on the T7 clock | Yes. A second template at its own period, or a narrow comb notch. |
| 4 | 1.9 Hz line on all channels | The beat between sources 1 and 3 (11.906 − 10) | Mostly. It fades once 1 and 3 are removed; a 1–2 Hz high-pass removes the rest without touching axle pulses. |
| 5 | One-sample positive spikes on the strain gauges: about 160 a second at 2 kHz, 15–30 µε, one sample wide at every rate | A very fast switching disturbance caught by about one reading in ten | Yes, but not with a linear filter. A 2.5 ms running minimum followed by a running maximum strips them and keeps 12 ms axle pulses: SG194 noise falls from 1.13 to 0.34 µε (0.22 in 2020). |
| 6 | Broad bump near 430 Hz and a hump near 870 Hz, strongest on PC3517 | A switching regulator near 30.4 kHz (and its 2nd harmonic), folded into the record because the T7 has no anti-aliasing filter in stream mode | Yes in practice: they sit above the vehicle band, so a 100 Hz low-pass removes them. |
| 7 | A peak wandering between 110 and 190 Hz | Aliased from about 22 kHz | Partly: a 100 Hz low-pass takes most of it, but it wanders near the band edge. |
| 8 | Broadband high-frequency noise folded into 0–100 Hz | The same missing anti-aliasing filter | No. Once folded in-band it cannot be told apart from signal. Only an analogue RC low-pass before the T7 fixes it. |
| 9 | Daily drift of 50–90 µε on the strain gauges | The pavement warming and cooling (real strain, not noise) | Yes for traffic work: a high-pass above about 0.5 Hz. Keep it for temperature studies. |
| — | Pressure cells at about 3.7 V; PC3517 open | Excitation or wiring fault at the box or sensor | Nothing to filter; there is no signal. |

## Recommended cleanup for the strain gauges

1. Strip the one-sample spikes (running minimum then maximum over 2.5 ms).
2. Subtract the 10 Hz and 11.9 Hz templates.
3. Band-pass 1–100 Hz (zero phase for stored data).
4. For detection, search only the quiet half of each 100 ms cycle.

This brings SG194 to about 1.5 times its 2020 noise level. What remains is
mostly items 2 and 8, which need hardware changes.

Steps 1 to 3 can be switched on per channel in the box configuration (`filters`,
see [LabJack configuration](reference/kv-config.md#filters)). They apply to the
live plots and, on request, to an extra column of exports; the archive keeps the
raw readings.

## Hardware fixes, in order of value

1. Find and remove the T7-clocked 10 Hz source: check what is connected to the
   T7's VS, DAC and digital-output terminals, and power one sensor board from a
   separate battery or linear supply while recording.
2. Fit an RC low-pass (about 500 Hz to 1 kHz) on each input before the T7.
3. Trace the switching sources near 30 kHz and 22 kHz.
