import { describe, expect, it } from 'vitest';
import {
    applyCalibration,
    formatCalibration,
    normalizeCalibration,
    resolveCalibrationUnit,
    strainBridgeCalibration
} from './calibration';

describe('normalizeCalibration', () => {
    it('keeps unit and id, trimmed, and leaves out blank ones', () => {
        expect(normalizeCalibration({ id: ' sg194 ', type: 'linear', a: 481.26, b: 1058.722, unit: ' µε ' })).toEqual({
            id: 'sg194',
            type: 'linear',
            a: 481.26,
            b: 1058.722,
            unit: 'µε'
        });
        const bare = normalizeCalibration({ id: '', type: 'linear', a: 1, b: 0, unit: '  ' });
        expect(Object.keys(bare)).toEqual(['type', 'a', 'b']);
    });

    it('repairs bad values as before', () => {
        expect(normalizeCalibration({ type: 'linear', a: '2' } as any)).toEqual({ type: 'linear', a: 2, b: 0 });
        expect(normalizeCalibration({ type: 'polynomial', unit: 'kPa' } as any)).toEqual({ type: 'polynomial', coeffs: [0, 1], unit: 'kPa' });
        expect(normalizeCalibration(null)).toEqual({ type: 'identity' });
        expect(normalizeCalibration({ type: 'weird' } as any)).toEqual({ type: 'identity' });
    });
});

describe('resolveCalibrationUnit', () => {
    it('is V for identity', () => {
        expect(resolveCalibrationUnit({ type: 'identity', unit: 'kPa' }, 'kPa')).toBe('V');
    });

    it("prefers the calibration's unit, then a non-V measurement unit", () => {
        expect(resolveCalibrationUnit({ type: 'linear', a: 1, b: 0, unit: 'µε' }, 'kPa')).toBe('µε');
        expect(resolveCalibrationUnit({ type: 'linear', a: 1, b: 0 }, 'kPa')).toBe('kPa');
        expect(resolveCalibrationUnit({ type: 'linear', a: 1, b: 0 }, 'V')).toBeUndefined();
        expect(resolveCalibrationUnit({ type: 'linear', a: 1, b: 0 }, '')).toBeUndefined();
        expect(resolveCalibrationUnit({ type: 'linear', a: 1, b: 0, unit: 'V' }, 'kPa')).toBe('V');
    });
});

describe('strainBridgeCalibration', () => {
    it('computes a and b from factor, excitation, gain and zero', () => {
        const spec = strainBridgeCalibration({ factor: 481.26, excitation: 10, gain: 100, zero: 0.5 })!;
        expect(spec.type).toBe('linear');
        if (spec.type !== 'linear') return;
        expect(spec.unit).toBe('µε');
        expect(spec.a).toBeCloseTo(481.26, 10);
        expect(spec.b).toBeCloseTo(-240.63, 10);
        // At the zero reading the strain is 0.
        expect(applyCalibration(spec, 0.5)).toBeCloseTo(0, 10);
    });

    it('matches the formula for other gauges and a negative gain', () => {
        const factor = 705.47, excitation = 5, gain = -200, zero = -0.012, raw = 0.37;
        const spec = strainBridgeCalibration({ factor, excitation, gain, zero })!;
        const expected = (factor * 1000 * raw) / (excitation * gain) - (factor * 1000 * zero) / (excitation * gain);
        expect(applyCalibration(spec, raw)).toBeCloseTo(expected, 9);
        expect(spec.type === 'linear' && spec.a).toBeCloseTo(-705.47, 10);
    });

    it('uses a zero offset when no zero reading is given', () => {
        const spec = strainBridgeCalibration({ factor: 481.26, excitation: 2.5, gain: 400 });
        expect(spec).toEqual({ type: 'linear', a: 481.26, b: 0, unit: 'µε' });
    });

    it('returns null for missing or zero inputs', () => {
        expect(strainBridgeCalibration({ factor: NaN, excitation: 10, gain: 100 })).toBeNull();
        expect(strainBridgeCalibration({ factor: 481.26, excitation: 0, gain: 100 })).toBeNull();
        expect(strainBridgeCalibration({ factor: 481.26, excitation: 10, gain: 0 })).toBeNull();
        expect(strainBridgeCalibration({ factor: 481.26, excitation: 10, gain: 100, zero: NaN })).toBeNull();
    });
});

describe('formatCalibration', () => {
    it('writes each formula type', () => {
        expect(formatCalibration({ type: 'identity' })).toBe('y = x');
        expect(formatCalibration({ type: 'linear', a: 481.26, b: 1058.722 })).toBe('y = 481.26·x + 1058.722');
        expect(formatCalibration({ type: 'linear', a: 2, b: -1 })).toBe('y = 2·x − 1');
        expect(formatCalibration({ type: 'polynomial', coeffs: [1, -2, 0.5] })).toBe('y = 1 − 2·x + 0.5·x²');
        expect(formatCalibration({ type: 'polynomial', coeffs: [] })).toBe('y = 0');
    });
});

describe('calibration presets', () => {
    /** Source of every non-test .ts and .svelte file under src/, by path. */
    const sources = import.meta.glob(['/src/**/*.{ts,svelte}', '!/src/**/*.test.ts'], {
        query: '?raw',
        import: 'default',
        eager: true
    }) as Record<string, string>;

    it('are no longer read, written or offered anywhere in the webapp', () => {
        expect(Object.keys(sources)).toContain('/src/lib/components/LabJackConfigModal.svelte');
        const leftovers = Object.entries(sources)
            .filter(([, text]) =>
                /availableCalibrations|onSaveCalibration|presetIdInputs|handleSavePreset|applyPreset|loadCalibrations|"calibration\.\*"|`calibration\.\$\{|Save Preset/.test(text)
            )
            .map(([path]) => path);
        expect(leftovers).toEqual([]);
    });
});
