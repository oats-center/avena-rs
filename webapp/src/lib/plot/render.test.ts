import { describe, expect, it } from 'vitest';
import type { DataPoint } from './stream';
import { computeValueRange, downsampleMinMax, formatTimeTick, latestFinitePoint, selectTimeWindow, splitAtGaps, timeAxisTicks } from './render';

const series = (values: number[], step = 1): DataPoint[] => values.map((value, i) => ({ timestamp: i * step, value }));

describe('render helpers', () => {
    it('computes the range of finite values only', () => {
        expect(computeValueRange(series([1, Number.NaN, -3, 250]))).toEqual({ min: -3, max: 250 });
        expect(computeValueRange(series([Number.NaN]))).toBeNull();
    });

    it('splits the trace at NaN gaps instead of bridging them', () => {
        const segments = splitAtGaps(series([1, 2, Number.NaN, Number.NaN, 3, 4, Number.NaN]));
        expect(segments.map((s) => s.map((p) => p.value))).toEqual([[1, 2], [3, 4]]);
    });

    it('keeps gaps and extremes when downsampling', () => {
        const values: number[] = [];
        for (let i = 0; i < 1000; i++) values.push(i === 500 ? Number.NaN : i === 250 ? 99 : Math.sin(i));
        const reduced = downsampleMinMax(series(values), 50);
        expect(reduced.length).toBeLessThanOrEqual(50 * 2 + 3);
        expect(reduced.some((p) => p.value === 99)).toBe(true);
        const segments = splitAtGaps(reduced);
        expect(segments.length).toBe(2);
        expect(segments[0][segments[0].length - 1].timestamp).toBeLessThan(500);
        expect(segments[1][0].timestamp).toBeGreaterThan(500);
        for (let i = 1; i < reduced.length; i++) expect(reduced[i].timestamp).toBeGreaterThan(reduced[i - 1].timestamp);
    });

    it('selects a time window with optional edge neighbors', () => {
        const data = series([0, 1, 2, 3, 4, 5], 10);
        expect(selectTimeWindow(data, 15, 35).map((p) => p.timestamp)).toEqual([20, 30]);
        expect(selectTimeWindow(data, 15, 35, true).map((p) => p.timestamp)).toEqual([10, 20, 30, 40]);
    });

    it('finds the latest present value', () => {
        expect(latestFinitePoint(series([1, 2, Number.NaN]))?.value).toBe(2);
        expect(latestFinitePoint([])).toBeNull();
    });
});

describe('time axis ticks', () => {
    it('anchors trigger-view ticks at 0 with a round step', () => {
        const ticks = timeAxisTicks(-6.7, 10, 11);
        expect(ticks.step).toBe(2);
        expect(ticks.values).toEqual([-6, -4, -2, 0, 2, 4, 6, 8, 10]);
        expect(ticks.unit).toBe('s');
    });

    it('never labels a tick -0', () => {
        const ticks = timeAxisTicks(-0.04, 0.06, 11);
        expect(ticks.unit).toBe('ms');
        const labels = ticks.values.map((v) => formatTimeTick(v, ticks.step, ticks.unit));
        expect(labels).toContain('0');
        expect(labels.some((l) => l.startsWith('-0'))).toBe(false);
        expect(formatTimeTick(-1e-12, 0.5, 's')).toBe('0.0');
    });

    it('keeps the tick count within the limit and ticks evenly spaced', () => {
        for (const [start, end, max] of [[-1, 0, 11], [-12.5, 0, 8], [-0.4, 0.6, 6], [-3, 7, 4]] as const) {
            const ticks = timeAxisTicks(start, end, max);
            expect(ticks.values.length).toBeLessThanOrEqual(max);
            expect(ticks.values.length).toBeGreaterThanOrEqual(2);
            for (let i = 1; i < ticks.values.length; i++) {
                expect(ticks.values[i] - ticks.values[i - 1]).toBeCloseTo(ticks.step, 9);
            }
            for (const v of ticks.values) {
                expect(Math.abs(v / ticks.step - Math.round(v / ticks.step))).toBeLessThan(1e-9);
            }
        }
    });

    it('formats with decimals that match the step', () => {
        expect(formatTimeTick(-0.2, 0.2, 's')).toBe('-0.2');
        expect(formatTimeTick(1, 0.5, 's')).toBe('1.0');
        expect(formatTimeTick(-0.02, 0.01, 'ms')).toBe('-20');
        expect(formatTimeTick(0.0025, 0.0005, 'ms')).toBe('2.5');
    });
});
