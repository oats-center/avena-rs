import { describe, expect, it } from 'vitest';
import { FlatBufferParser } from '../flatbuffer-parser';
import { normalizeCalibration } from '../calibration';
import { ScanMessageQueue, createLiveChannel, drainChannelQueue, type DataPoint } from './stream';
import {
    advanceTrigger,
    findTriggerCrossing,
    type TriggerCapture,
    type TriggerSettings
} from './trigger';
import { computeValueRange } from './render';
import { FakeStreamer } from './test-helpers';

const parser = new FlatBufferParser();
const identity = normalizeCalibration(undefined);

/**
 * Feeds a streamer through the queue, decoding and `advanceTrigger`, the same calls the
 * plot page makes on each tick. Returns the last capture and every capture change.
 */
function runTrigger(
    streamer: FakeStreamer,
    settings: TriggerSettings,
    messagesPerTick: number,
    ticks: number,
    singleShot = true
) {
    const queue = new ScanMessageQueue();
    const live = createLiveChannel();
    let capture: TriggerCapture | null = null;
    const history: TriggerCapture[] = [];
    const all: DataPoint[] = [];

    for (let t = 0; t < ticks; t++) {
        for (let m = 0; m < messagesPerTick; m++) {
            queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        }
        drainChannelQueue(queue, 0, live, (p) => parser.parse(p), identity, 1_000_000, (start, reset, chunk) => {
            all.push(...chunk);
            const next = advanceTrigger(capture, singleShot, settings, live.buffer, start, reset, chunk);
            if (next && next !== capture) history.push(next);
            capture = next;
        });
    }
    return { capture: capture as TriggerCapture | null, history, all };
}

const runSingleShot = (streamer: FakeStreamer, settings: TriggerSettings, perTick: number, ticks: number) =>
    runTrigger(streamer, settings, perTick, ticks, true);

describe('trigger capture', () => {
    // A slow ramp that crosses 0.5 once: value(k) = k / 1000 - 0.5 at 2 kHz crosses
    // at sample 1001 (the first value above 0.5 is at k = 1001).
    const ramp = (k: number) => k / 1000 - 0.5;
    const settings: TriggerSettings = { type: 'rising', threshold: 0.5, preTriggerPercent: 40, postTriggerWindowSec: 0.3 };

    it('captures exactly the samples in [trigger - pre, trigger + post] across message boundaries', () => {
        const streamer = new FakeStreamer(2000, 100, 1_790_000_000_000_000_000n, ramp);
        const { capture, all } = runSingleShot(streamer, settings, 22, 3);
        expect(capture).not.toBeNull();
        const c = capture!;
        expect(c.complete).toBe(true);
        expect(c.triggerTime).toBe(all[1001].timestamp);

        const expected = all.filter((p) => p.timestamp >= c.startTime && p.timestamp <= c.endTime);
        expect(c.data).toEqual(expected);
        // pre = 0.3 * 0.4 / 0.6 = 0.2 s = 400 samples; post = 0.3 s = 600 samples.
        expect(c.data.length).toBe(400 + 1 + 600);
        for (let i = 1; i < c.data.length; i++) expect(c.data[i].timestamp).toBeGreaterThan(c.data[i - 1].timestamp);
    });

    it('computes the y range from all captured samples, including those after the trigger', () => {
        const streamer = new FakeStreamer(2000, 100, 1_790_000_000_000_000_000n, ramp);
        const { capture } = runSingleShot(streamer, settings, 22, 3);
        const range = computeValueRange(capture!.data)!;
        // Ramp values in the window: first at sample 601, last at sample 1601.
        expect(range.min).toBeCloseTo(0.101, 9);
        expect(range.max).toBeCloseTo(1.101, 9);
    });

    it('is still collecting, with only the samples so far, until a sample reaches the window end', () => {
        const streamer = new FakeStreamer(100, 100, 1_790_000_000_000_000_000n, (k) => (k >= 250 ? 1 : 0));
        const s: TriggerSettings = { type: 'rising', threshold: 0.5, preTriggerPercent: 50, postTriggerWindowSec: 1 };
        const partial = runSingleShot(streamer, s, 3, 1);
        expect(partial.capture!.complete).toBe(false);
        expect(partial.capture!.data.length).toBe(100 + 1 + 49); // 1 s pre + trigger + samples 251..299 so far
        const rest = new FakeStreamer(100, 100, 1_790_000_000_000_000_000n, (k) => (k >= 250 ? 1 : 0));
        const full = runSingleShot(rest, s, 4, 1);
        expect(full.capture!.complete).toBe(true);
        expect(full.capture!.data.length).toBe(100 + 1 + 100);
    });

    it('does not fire before the pre-trigger window is buffered, nor on NaN gaps', () => {
        const buffer: DataPoint[] = [
            { timestamp: 0, value: 0 },
            { timestamp: 10, value: 1 },
            { timestamp: 20, value: Number.NaN },
            { timestamp: 30, value: 0 },
            { timestamp: 40, value: Number.NaN },
            { timestamp: 50, value: 1 }
        ];
        const s: TriggerSettings = { type: 'rising', threshold: 0.5, preTriggerPercent: 50, postTriggerWindowSec: 0.02 };
        // pre = 20 ms: the crossing at t=10 is too early; the one at t=50 is judged
        // against the last present value (0 at t=30), skipping the NaN.
        expect(findTriggerCrossing(buffer, 0, s)).toBe(5);
        expect(findTriggerCrossing(buffer, 0, s, 50)).toBe(-1);
    });

    it('fires on a falling edge', () => {
        const buffer: DataPoint[] = [0, 1, 1, 0.2].map((value, i) => ({ timestamp: i * 10, value }));
        const s: TriggerSettings = { type: 'falling', threshold: 0.5, preTriggerPercent: 0, postTriggerWindowSec: 0.01 };
        expect(findTriggerCrossing(buffer, 0, s)).toBe(3);
    });

    it('normal mode: each capture holds exactly its window, and the next trigger waits for the previous window to end', () => {
        // Square wave: 0 for 150 samples, 1 for 150 samples, at 2 kHz (period 0.15 s).
        const square = (k: number) => (Math.floor(k / 150) % 2 === 1 ? 1 : 0);
        const s: TriggerSettings = { type: 'rising', threshold: 0.5, preTriggerPercent: 20, postTriggerWindowSec: 0.2 };
        const streamer = new FakeStreamer(2000, 100, 1_790_000_000_000_000_000n, square);
        const { history, all } = runTrigger(streamer, s, 21, 4, false);

        const completed = history.filter((c) => c.complete);
        expect(completed.length).toBeGreaterThan(3);
        let previousEnd = Number.NEGATIVE_INFINITY;
        for (const c of completed) {
            const expected = all.filter((p) => p.timestamp >= c.startTime && p.timestamp <= c.endTime);
            expect(c.data).toEqual(expected);
            // Rising edges are at samples 150, 450, 750, ...; with a 0.2 s (400 sample) post window the
            // edge 0.15 s later falls inside it, so every other edge fires.
            expect(c.triggerTime).toBeGreaterThan(previousEnd);
            previousEnd = c.endTime;
        }
        const edgeSamples = completed.map((c) => all.findIndex((p) => p.timestamp === c.triggerTime));
        for (const k of edgeSamples) expect(k % 600).toBe(150);
    });

    it('single-shot mode keeps the first capture', () => {
        const square = (k: number) => (Math.floor(k / 150) % 2 === 1 ? 1 : 0);
        const s: TriggerSettings = { type: 'rising', threshold: 0.5, preTriggerPercent: 20, postTriggerWindowSec: 0.05 };
        const streamer = new FakeStreamer(2000, 100, 1_790_000_000_000_000_000n, square);
        const { capture, all } = runTrigger(streamer, s, 21, 3, true);
        expect(capture!.complete).toBe(true);
        expect(all.findIndex((p) => p.timestamp === capture!.triggerTime)).toBe(150);
    });
});
