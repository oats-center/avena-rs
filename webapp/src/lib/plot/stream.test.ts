import { describe, expect, it } from 'vitest';
import { FlatBufferParser } from '../flatbuffer-parser';
import { normalizeCalibration } from '../calibration';
import {
    ScanMessageQueue,
    createLiveChannel,
    drainChannelQueue,
    ingestScan,
    snapshotNewest,
    type DataPoint,
    type LiveChannel
} from './stream';
import { FakeStreamer, encodeScan } from './test-helpers';

const parser = new FlatBufferParser();
const decode = (payload: ArrayBuffer | Uint8Array) => parser.parse(payload);
const identity = normalizeCalibration(undefined);

function expectStrictlyIncreasing(points: DataPoint[]) {
    for (let i = 1; i < points.length; i++) {
        expect(points[i].timestamp).toBeGreaterThan(points[i - 1].timestamp);
    }
}

/** Runs one UI tick the way the plot page does. */
function tick(queue: ScanMessageQueue, channel: number, live: LiveChannel, maxPoints = 1_000_000) {
    return drainChannelQueue(queue, channel, live, decode, identity, maxPoints);
}

describe('FlatBuffer Scan decoding', () => {
    it('decodes a payload built like the streamer builds it, including misaligned views', () => {
        const streamer = new FakeStreamer(100, 100);
        const { fields, payload } = streamer.nextBatch();
        const scan = parser.parse(payload)!;
        expect(scan.firstSampleUnixNs).toBe(fields.firstSampleUnixNs);
        expect(scan.sampleIntervalNs).toBe(10_000_000n);
        expect(Array.from(scan.values)).toEqual(fields.values);

        // A payload at an odd offset inside a larger buffer, as NATS can deliver.
        const shifted = new Uint8Array(payload.length + 3);
        shifted.set(payload, 3);
        const view = shifted.subarray(3);
        expect(Array.from(parser.parse(view)!.values)).toEqual(fields.values);
    });
});

describe('queue and decode order', () => {
    for (const [rate, scansPerRead, messages] of [
        [100, 100, 30],
        [2000, 100, 25],
        [2000, 50, 60]
    ] as const) {
        it(`${rate} Hz, ${scansPerRead} scans/read: a burst of ${messages} messages in one tick reaches the buffer exactly once, in order`, () => {
            const streamer = new FakeStreamer(rate, scansPerRead);
            const queue = new ScanMessageQueue();
            const live = createLiveChannel();
            const expected: number[] = [];

            for (let m = 0; m < messages; m++) {
                const { fields, payload } = streamer.nextBatch();
                for (let i = 0; i < scansPerRead; i++) expected.push(streamer.timeOfSampleMs(fields.firstSampleUnixNs, i));
                queue.push(0, { payload, receivedAt: 0 });
            }
            expect(queue.size(0)).toBe(messages);

            expect(tick(queue, 0, live)).toBe(messages);
            expect(queue.size(0)).toBe(0);
            expect(live.buffer.map((p) => p.timestamp)).toEqual(expected);
            expect(live.buffer.map((p) => p.value)).toEqual(
                expected.map((_, k) => Math.sin(k / 10))
            );
            expectStrictlyIncreasing(live.buffer);
            expect(live.stats).toMatchObject({ gaps: 0, resets: 0, skippedSamples: 0, decodeErrors: 0 });
        });
    }

    it('keeps every sample across many ticks, including with clock slews', () => {
        const streamer = new FakeStreamer(2000, 100);
        const queue = new ScanMessageQueue();
        const live = createLiveChannel();
        let expectedCount = 0;
        for (let t = 0; t < 10; t++) {
            for (let m = 0; m < 22; m++) {
                // Slew of up to +/- a quarter interval, as the streamer's clock can do.
                const slew = m % 7 === 0 ? (t % 2 === 0 ? 125_000n : -125_000n) : 0n;
                queue.push(3, { payload: streamer.nextBatch(slew).payload, receivedAt: 0 });
                expectedCount += 100;
            }
            tick(queue, 3, live);
        }
        expect(live.buffer.length).toBe(expectedCount);
        expectStrictlyIncreasing(live.buffer);
        expect(live.stats.gaps).toBe(0);
    });

    it('drops the oldest messages only when the queue is full, and counts them', () => {
        const streamer = new FakeStreamer(100, 100);
        const queue = new ScanMessageQueue(5);
        for (let m = 0; m < 8; m++) queue.push(1, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        expect(queue.size(1)).toBe(5);
        expect(queue.droppedMessages.get(1)).toBe(3);
        const live = createLiveChannel();
        tick(queue, 1, live);
        expect(live.buffer.length).toBe(500);
        expect(live.buffer[0].timestamp).toBe(streamer.timeOfSampleMs(1_790_000_000_000_000_000n, 300));
    });

    it('trims the buffer to maxPoints, dropping the oldest samples', () => {
        const streamer = new FakeStreamer(100, 100);
        const queue = new ScanMessageQueue();
        const live = createLiveChannel();
        for (let m = 0; m < 5; m++) queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        tick(queue, 0, live, 250);
        expect(live.buffer.length).toBe(250);
        expect(live.buffer[live.buffer.length - 1].timestamp).toBe(
            streamer.timeOfSampleMs(1_790_000_000_000_000_000n, 499)
        );
    });

    it('clears one channel without touching the others', () => {
        const streamer = new FakeStreamer(100, 10);
        const queue = new ScanMessageQueue(2);
        for (let m = 0; m < 3; m++) queue.push(1, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        queue.push(2, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        queue.clearChannel(1);
        expect(queue.size(1)).toBe(0);
        expect(queue.droppedMessages.get(1)).toBeUndefined();
        expect(queue.size(2)).toBe(1);
    });

    it('counts undecodable payloads without stopping the queue', () => {
        const streamer = new FakeStreamer(100, 10);
        const queue = new ScanMessageQueue();
        const live = createLiveChannel();
        queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        queue.push(0, { payload: new Uint8Array([1, 2]), receivedAt: 0 });
        queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        tick(queue, 0, live);
        expect(live.buffer.length).toBe(20);
        expect(live.stats.decodeErrors).toBe(1);
    });
});

describe('gaps, duplicates and timeline jumps', () => {
    it('inserts one NaN gap marker when messages are missing', () => {
        const streamer = new FakeStreamer(100, 100);
        const queue = new ScanMessageQueue();
        const live = createLiveChannel();
        queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        streamer.nextBatch(); // lost in transport
        queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        tick(queue, 0, live);
        expect(live.buffer.length).toBe(201);
        const nanIndexes = live.buffer.flatMap((p, i) => (Number.isNaN(p.value) ? [i] : []));
        expect(nanIndexes).toEqual([100]);
        expectStrictlyIncreasing(live.buffer);
        expect(live.stats.gaps).toBe(1);
    });

    it('skips a repeated message (for example redelivered after a reconnect)', () => {
        const streamer = new FakeStreamer(100, 100);
        const queue = new ScanMessageQueue();
        const live = createLiveChannel();
        const a = streamer.nextBatch().payload;
        const b = streamer.nextBatch().payload;
        for (const p of [a, b, b, a]) queue.push(0, { payload: p, receivedAt: 0 });
        tick(queue, 0, live);
        expect(live.buffer.length).toBe(200);
        expectStrictlyIncreasing(live.buffer);
        expect(live.stats.skippedSamples).toBe(200);
        expect(live.stats.resets).toBe(0);
    });

    it('accepts a new run whose timeline continues forward (sequence restarts at 0)', () => {
        const run1 = new FakeStreamer(100, 100, 1_790_000_000_000_000_000n);
        const run2 = new FakeStreamer(100, 100, 1_790_000_005_000_000_000n);
        const queue = new ScanMessageQueue();
        const live = createLiveChannel();
        for (let m = 0; m < 3; m++) queue.push(0, { payload: run1.nextBatch().payload, receivedAt: 0 });
        for (let m = 0; m < 2; m++) queue.push(0, { payload: run2.nextBatch().payload, receivedAt: 0 });
        tick(queue, 0, live);
        expect(live.buffer.length).toBe(501);
        expectStrictlyIncreasing(live.buffer);
        expect(live.stats.resets).toBe(0);
    });

    it('resets the buffer when the timeline steps backward instead of mixing timelines', () => {
        const run1 = new FakeStreamer(100, 100, 1_790_000_010_000_000_000n);
        const run2 = new FakeStreamer(100, 100, 1_790_000_005_000_000_000n);
        const queue = new ScanMessageQueue();
        const live = createLiveChannel();
        for (let m = 0; m < 3; m++) queue.push(0, { payload: run1.nextBatch().payload, receivedAt: 0 });
        for (let m = 0; m < 2; m++) queue.push(0, { payload: run2.nextBatch().payload, receivedAt: 0 });
        tick(queue, 0, live);
        expect(live.stats.resets).toBe(1);
        expect(live.buffer.length).toBe(200);
        expect(live.buffer[0].timestamp).toBe(1_790_000_005_000);
        expectStrictlyIncreasing(live.buffer);
    });
});

describe('values', () => {
    const scanOf = (values: number[]) =>
        parser.parse(
            encodeScan({
                firstSampleUnixNs: 1_000_000_000n,
                sampleIntervalNs: 1_000_000n,
                actualScanRateHz: 1000,
                sequence: 0n,
                values
            })
        )!;

    it('turns NaN and LJM -9999 dummies into NaN points (gaps), keeping their time slot', () => {
        const result = ingestScan(null, scanOf([1, Number.NaN, -9999, 2]), identity, 0);
        expect(result.points.map((p) => p.timestamp)).toEqual([1000, 1001, 1002, 1003]);
        expect(result.points.map((p) => p.value)).toEqual([1, Number.NaN, Number.NaN, 2]);
    });

    it('keeps legitimate values with magnitude of 100 or more', () => {
        const result = ingestScan(null, scanOf([150.5, -250, 99.9, 1e6]), identity, 0);
        expect(result.points.map((p) => p.value)).toEqual([150.5, -250, 99.9, 1e6]);
    });

    it('applies the calibration to raw values at plot time', () => {
        const linear = normalizeCalibration({ type: 'linear', a: 2, b: -1 });
        const result = ingestScan(null, scanOf([0, 3, Number.NaN, 60]), linear, 0);
        expect(result.points.map((p) => p.value)).toEqual([-1, 5, Number.NaN, 119]);
        const poly = normalizeCalibration({ type: 'polynomial', coeffs: [1, 0, 2] });
        expect(ingestScan(null, scanOf([3]), poly, 0).points[0].value).toBe(19);
    });
});

describe('snapshotNewest', () => {
    it('keeps the newest span plus one point before it', () => {
        const data: DataPoint[] = Array.from({ length: 100 }, (_, i) => ({ timestamp: i * 10, value: i }));
        const snap = snapshotNewest(data, 200);
        expect(snap[0].timestamp).toBe(780);
        expect(snap[snap.length - 1].timestamp).toBe(990);
        expect(snapshotNewest([], 100)).toEqual([]);
    });
});
