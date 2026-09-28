import { describe, expect, it } from 'vitest';
import type { DataPoint } from './stream';
import { computeValueRange, downsampleMinMax, latestFinitePoint, selectTimeWindow, splitAtGaps } from './render';

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
