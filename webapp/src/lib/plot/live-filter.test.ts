import { describe, expect, it } from 'vitest';
import { normalizeCalibration } from '../calibration';
import type { ChannelFilterSettings } from '../filter-settings';
import { filterCausal, planFilters } from '../filters';
import { FlatBufferParser } from '../flatbuffer-parser';
import { LiveChannelFilter } from './live-filter';
import { ScanMessageQueue, createLiveChannel, drainChannelQueue, type DataPoint, type LiveChannel } from './stream';
import { FakeStreamer } from './test-helpers';

const parser = new FlatBufferParser();
const decode = (payload: ArrayBuffer | Uint8Array) => parser.parse(payload);
const identity = normalizeCalibration(undefined);
const ALL: ChannelFilterSettings = { despike: true, remove_10hz: true, remove_11_9hz: true, highpass_hz: 1, lowpass_hz: 100 };

/** Synthetic 2 kHz strain-like signal: offset, both square waves, spikes, noise. */
function signal(k: number): number {
    const turns = (k * 11.906) / 2000;
    const square = (k % 200 < 100 ? 0.003 : -0.003) + (turns - Math.floor(turns) < 0.5 ? 0.0015 : -0.0015);
    const spike = k % 13 === 5 ? 0.005 : 0;
    const noise = 0.0002 * Math.sin(k * 12.9898) * Math.cos(k * 78.233);
    return 0.5 + square + spike + noise;
}

function run(streamer: FakeStreamer, batches: number, live: LiveChannel, filter: LiveChannelFilter, skip = new Set<number>()) {
    const queue = new ScanMessageQueue(1_000_000);
    for (let b = 0; b < batches; b++) {
        const { payload } = streamer.nextBatch();
        if (!skip.has(b)) queue.push(0, { payload, receivedAt: 0 });
    }
    drainChannelQueue(queue, 0, live, decode, identity, 10_000_000, undefined, Number.POSITIVE_INFINITY, filter);
}

function expectOnceInOrder(points: DataPoint[]) {
    for (let i = 1; i < points.length; i++) expect(points[i].timestamp).toBeGreaterThan(points[i - 1].timestamp);
}

describe('LiveChannelFilter', () => {
    it('gives the causal pipeline output, every sample once and in order, delayed by the despike window', () => {
        const streamer = new FakeStreamer(2000, 100, undefined, signal);
        const live = createLiveChannel();
        const filter = new LiveChannelFilter(ALL, identity);
        run(streamer, 50, live, filter);
        // 5000 samples in, the last 4 held back by the despike window.
        expect(live.buffer).toHaveLength(4996);
        expectOnceInOrder(live.buffer);
        const reference = filterCausal(Array.from({ length: 5000 }, (_, k) => signal(k)), planFilters(ALL, 2000));
        live.buffer.forEach((point, k) => {
            expect(point.unfiltered).toBe(signal(k));
            expect(point.raw).toBe(signal(k));
            expect(point.filtered).toBe(reference[k]);
            expect(point.value).toBe(point.filtered);
        });
    });

    it('keeps the templates in phase over dropped messages and starts over for a new run', () => {
        const streamer = new FakeStreamer(2000, 100, undefined, signal);
        const live = createLiveChannel();
        const filter = new LiveChannelFilter(ALL, identity);
        run(streamer, 30, live, filter, new Set([12, 13]));
        const input = Array.from({ length: 3000 }, (_, k) => (k >= 1200 && k < 1400 ? Number.NaN : signal(k)));
        const reference = filterCausal(input, planFilters(ALL, 2000));
        const samples = live.buffer.filter((p) => Number.isFinite(p.raw));
        expect(live.buffer.filter((p) => !Number.isFinite(p.value) && !Number.isFinite(p.raw))).toHaveLength(1); // the gap marker
        expectOnceInOrder(live.buffer);
        samples.forEach((point) => {
            const k = Math.round((point.timestamp - live.buffer[0].timestamp) / 0.5);
            expect(point.filtered).toBe(reference[k]);
        });

        // A new run later on: sequence back to 0, time forward.
        const next = new FakeStreamer(2000, 100, 1_790_000_100_000_000_000n, (k) => signal(k + 77));
        run(next, 10, live, filter);
        const fresh = filterCausal(Array.from({ length: 1000 }, (_, k) => signal(k + 77)), planFilters(ALL, 2000));
        const newRun = live.buffer.filter((p) => p.timestamp >= 1_790_000_100_000);
        expect(newRun).toHaveLength(996);
        newRun.forEach((point, k) => expect(point.filtered).toBe(fresh[k]));
        // The old run's held samples were flushed before the new run started.
        expect(live.buffer.filter((p) => p.timestamp < 1_790_000_100_000 && Number.isFinite(p.raw))).toHaveLength(2800);
    });

    it('switches the plotted value between filtered and raw in place', () => {
        const streamer = new FakeStreamer(2000, 100, undefined, signal);
        const live = createLiveChannel();
        const filter = new LiveChannelFilter(ALL, identity);
        run(streamer, 5, live, filter);
        filter.setShowFiltered(false, live.buffer);
        live.buffer.forEach((p) => expect(p.value).toBe(p.unfiltered));
        run(streamer, 1, live, filter);
        live.buffer.forEach((p) => expect(p.value).toBe(p.unfiltered));
        filter.setShowFiltered(true, live.buffer);
        live.buffer.forEach((p) => expect(p.value).toBe(p.filtered));
    });

    it('applies the calibration between the raw stages and the high-pass', () => {
        const calibration = normalizeCalibration({ type: 'linear', a: 100, b: -40, unit: 'kPa' });
        const streamer = new FakeStreamer(100, 10, undefined, () => 0.5);
        const live = createLiveChannel();
        const filter = new LiveChannelFilter({ highpass_hz: 1 }, calibration);
        const queue = new ScanMessageQueue(1000);
        for (let b = 0; b < 20; b++) queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
        drainChannelQueue(queue, 0, live, decode, calibration, 10_000, undefined, Number.POSITIVE_INFINITY, filter);
        expect(live.buffer).toHaveLength(200); // no despike, no delay
        for (const p of live.buffer) {
            expect(p.unfiltered).toBeCloseTo(10, 9);
            expect(Math.abs(p.filtered!)).toBeLessThan(1e-9); // high-passed around 0 kPa, not -40
        }
    });

    it('costs little per sample (measured, printed)', () => {
        const seconds = 60;
        const measure = (withFilter: boolean) => {
            const streamer = new FakeStreamer(2000, 200, undefined, signal);
            const queue = new ScanMessageQueue(1_000_000);
            for (let b = 0; b < seconds * 10; b++) queue.push(0, { payload: streamer.nextBatch().payload, receivedAt: 0 });
            const live = createLiveChannel();
            const filter = withFilter ? new LiveChannelFilter(ALL, identity) : null;
            const t0 = performance.now();
            drainChannelQueue(queue, 0, live, decode, identity, 20_000, undefined, Number.POSITIVE_INFINITY, filter);
            return (performance.now() - t0) / (seconds * 2000);
        };
        measure(true); // warm up the JIT
        measure(false);
        const plain = Math.min(measure(false), measure(false), measure(false));
        const filtered = Math.min(measure(true), measure(true), measure(true));
        console.log(
            `live plot path at 2 kHz: ${(plain * 1e6).toFixed(0)} ns/sample plain, ${(filtered * 1e6).toFixed(0)} ns/sample with every filter (+${((filtered - plain) * 1e6).toFixed(0)} ns); one 2 kHz channel costs +${((filtered - plain) * 2000).toFixed(2)} ms of CPU per second`
        );
        expect(filtered - plain).toBeLessThan(0.005); // under 5 µs a sample
    });
});
