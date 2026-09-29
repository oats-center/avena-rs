import { describe, expect, it } from 'vitest';
import { applyAxisLimitInput, parseFiniteInput, seedManualYLimits } from './axis';

const limits = { yMin: -1, yMax: 1, xWindowSec: 2 };

describe('parseFiniteInput', () => {
    it('accepts 0 and negative numbers', () => {
        expect(parseFiniteInput('0')).toBe(0);
        expect(parseFiniteInput(' -2.5 ')).toBe(-2.5);
        expect(parseFiniteInput('1e3')).toBe(1000);
    });

    it('rejects blank and non-numeric text', () => {
        expect(parseFiniteInput('')).toBeNull();
        expect(parseFiniteInput('  ')).toBeNull();
        expect(parseFiniteInput('abc')).toBeNull();
        expect(parseFiniteInput('Infinity')).toBeNull();
    });
});

describe('applyAxisLimitInput', () => {
    it('keeps Y Min 0 and Y Max 0 as typed', () => {
        expect(applyAxisLimitInput(limits, 'yMin', '0')).toEqual({ ok: true, limits: { ...limits, yMin: 0 } });
        expect(applyAxisLimitInput(limits, 'yMax', '0')).toEqual({ ok: true, limits: { ...limits, yMax: 0 } });
    });

    it('rejects Y Min at or above Y Max', () => {
        expect(applyAxisLimitInput(limits, 'yMin', '1').ok).toBe(false);
        expect(applyAxisLimitInput(limits, 'yMax', '-3').ok).toBe(false);
    });

    it('requires an X window above 0', () => {
        expect(applyAxisLimitInput(limits, 'xWindowSec', '0').ok).toBe(false);
        expect(applyAxisLimitInput(limits, 'xWindowSec', '-1').ok).toBe(false);
        expect(applyAxisLimitInput(limits, 'xWindowSec', '0.05')).toEqual({
            ok: true,
            limits: { ...limits, xWindowSec: 0.05 }
        });
    });

    it('rejects text that is not a number', () => {
        expect(applyAxisLimitInput(limits, 'yMin', '')).toEqual({ ok: false, error: 'Enter a number.' });
    });
});

describe('seedManualYLimits', () => {
    const current = { yMin: -1, yMax: 1 };
    it('keeps round auto ranges as they are', () => {
        expect(seedManualYLimits({ low: -2, high: 2 }, current)).toEqual({ yMin: -2, yMax: 2 });
        expect(seedManualYLimits({ low: 3.6, high: 3.84 }, current)).toEqual({ yMin: 3.6, yMax: 3.84 });
        expect(seedManualYLimits({ low: -500, high: 1500 }, current)).toEqual({ yMin: -500, yMax: 1500 });
    });
    it('rounds outward to a sensible precision', () => {
        expect(seedManualYLimits({ low: 0.12345, high: 0.30001 }, current)).toEqual({ yMin: 0.123, yMax: 0.301 });
        expect(seedManualYLimits({ low: 101.2345, high: 187.891 }, current)).toEqual({ yMin: 101.2, yMax: 187.9 });
        expect(seedManualYLimits({ low: 0.1 + 0.2, high: 0.7 }, current)).toEqual({ yMin: 0.3, yMax: 0.7 });
    });
    it('keeps the current limits without a usable range', () => {
        expect(seedManualYLimits(null, current)).toEqual(current);
        expect(seedManualYLimits({ low: 1, high: 1 }, current)).toEqual(current);
        expect(seedManualYLimits({ low: Number.NaN, high: 1 }, current)).toEqual(current);
    });
});
