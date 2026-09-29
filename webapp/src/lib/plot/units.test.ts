import { describe, expect, it } from 'vitest';
import { normalizeCalibration } from '../calibration';
import { describeChannelUnit } from './units';

describe('describeChannelUnit', () => {
    it('labels an uncalibrated channel as raw volts whatever unit is configured', () => {
        expect(describeChannelUnit(normalizeCalibration(undefined), 'V')).toEqual({
            unit: 'V',
            calibrated: false,
            tag: 'raw volts',
            warning: undefined
        });
        const kpa = describeChannelUnit(normalizeCalibration({ type: 'identity' }), 'kPa');
        expect(kpa.unit).toBe('V');
        expect(kpa.warning).toContain('kPa');
    });

    it('ignores a unit stored on an identity calibration', () => {
        const info = describeChannelUnit(normalizeCalibration({ type: 'identity', unit: 'kPa' }), 'V');
        expect(info.unit).toBe('V');
        expect(info.calibrated).toBe(false);
    });

    it("uses the calibration's own unit", () => {
        const info = describeChannelUnit(normalizeCalibration({ type: 'linear', a: 2, b: 1, unit: 'kPa' }), 'V');
        expect(info).toEqual({ unit: 'kPa', calibrated: true, tag: 'calibrated → kPa' });
    });

    it('prefers the calibration unit over measurement_units', () => {
        const info = describeChannelUnit(normalizeCalibration({ type: 'linear', a: 481.26, b: 1058.722, unit: 'µε' }), 'PSI');
        expect(info.unit).toBe('µε');
        expect(info.warning).toBeUndefined();
    });

    it('accepts V when it is the calibration unit chosen explicitly', () => {
        const info = describeChannelUnit(normalizeCalibration({ type: 'linear', a: 0.5, b: 0, unit: 'V' }), 'V');
        expect(info).toEqual({ unit: 'V', calibrated: true, tag: 'calibrated → V' });
    });

    it('falls back to measurement_units for an older calibration without a unit', () => {
        const info = describeChannelUnit(normalizeCalibration({ id: 'tp3505', type: 'linear', a: 2, b: 1 }), 'kPa');
        expect(info).toEqual({ unit: 'kPa', calibrated: true, tag: 'calibrated → kPa' });
    });

    it('warns when an older calibrated channel has no unit (V or blank)', () => {
        for (const configured of ['V', ' ', undefined]) {
            const info = describeChannelUnit(normalizeCalibration({ id: 'asgm194', type: 'polynomial', coeffs: [0, 1000] }), configured);
            expect(info.unit).toBe('V');
            expect(info.calibrated).toBe(true);
            expect(info.tag).toBe('calibrated: polynomial → unit not set');
            expect(info.warning).toContain('no unit');
        }
    });
});
