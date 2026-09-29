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

    it('uses the configured unit for a calibrated channel and names the preset', () => {
        const info = describeChannelUnit(normalizeCalibration({ id: 'tp3505', type: 'linear', a: 2, b: 1 }), 'kPa');
        expect(info).toEqual({ unit: 'kPa', calibrated: true, tag: 'calibrated: tp3505 → kPa', warning: undefined });
    });

    it('flags a calibrated channel still labelled V', () => {
        const info = describeChannelUnit(normalizeCalibration({ id: 'asgm194', type: 'polynomial', coeffs: [0, 1000] }), 'V');
        expect(info.unit).toBe('V');
        expect(info.calibrated).toBe(true);
        expect(info.tag).toBe('calibrated: asgm194 → V');
        expect(info.warning).toContain('asgm194');
    });

    it('falls back to the calibration type and to V for a blank unit', () => {
        const info = describeChannelUnit(normalizeCalibration({ type: 'linear', a: 3, b: 0 }), ' ');
        expect(info.tag).toBe('calibrated: linear → V');
    });
});
