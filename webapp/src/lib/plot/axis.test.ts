import { describe, expect, it } from 'vitest';
import { applyAxisLimitInput, parseFiniteInput } from './axis';

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
