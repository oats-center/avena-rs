import { describe, expect, it } from 'vitest';
import {
    DEFAULT_SENSOR_SETTINGS,
    normalizeLabJackConfig,
    normalizeSensorSettings,
    sanitizeLabJackConfig
} from './labjack-config';

describe('normalizeSensorSettings', () => {
    it('fills every field with defaults when given nothing', () => {
        expect(normalizeSensorSettings(undefined)).toEqual({
            scans_per_read: 200,
            scan_rate_hz: 1000,
            channels_enabled: [],
            gains: 1,
            data_formats: [],
            measurement_units: [],
            labjack_on_off: false,
            calibrations: {}
        });
    });

    it('reads the older scan_rate and sampling_rate names', () => {
        const s = normalizeSensorSettings({ scan_rate: 50, sampling_rate: 7000 });
        expect(s.scans_per_read).toBe(50);
        expect(s.scan_rate_hz).toBe(7000);
    });

    it('prefers the new names over the old ones', () => {
        const s = normalizeSensorSettings({ scans_per_read: 10, scan_rate: 50, scan_rate_hz: 20, sampling_rate: 7000 });
        expect(s.scans_per_read).toBe(10);
        expect(s.scan_rate_hz).toBe(20);
    });

    it('replaces non-finite numbers with defaults', () => {
        const s = normalizeSensorSettings({ scans_per_read: 'abc', scan_rate_hz: NaN, gains: 'x' });
        expect(s.scans_per_read).toBe(DEFAULT_SENSOR_SETTINGS.scans_per_read);
        expect(s.scan_rate_hz).toBe(DEFAULT_SENSOR_SETTINGS.scan_rate_hz);
        expect(s.gains).toBe(DEFAULT_SENSOR_SETTINGS.gains);
    });

    it('converts numeric strings', () => {
        expect(normalizeSensorSettings({ scan_rate_hz: '500' }).scan_rate_hz).toBe(500);
    });

    it('pads formats and units to one entry per enabled channel', () => {
        const s = normalizeSensorSettings({
            channels_enabled: [0, 1, 2],
            data_formats: ['pressure'],
            measurement_units: []
        });
        expect(s.data_formats).toEqual(['pressure', 'voltage', 'voltage']);
        expect(s.measurement_units).toEqual(['V', 'V', 'V']);
    });

    it('copies arrays and calibrations instead of sharing them', () => {
        const raw = {
            channels_enabled: [1],
            data_formats: ['voltage'],
            measurement_units: ['V'],
            calibrations: { '1': { type: 'linear', a: 2, b: 0 } }
        };
        const s = normalizeSensorSettings(raw);
        s.channels_enabled.push(2);
        s.calibrations!['2'] = { type: 'identity' };
        expect(raw.channels_enabled).toEqual([1]);
        expect(Object.keys(raw.calibrations)).toEqual(['1']);
    });

    it('ignores calibrations that are not an object and coerces labjack_on_off', () => {
        const s = normalizeSensorSettings({ calibrations: 'bad', labjack_on_off: 1 });
        expect(s.calibrations).toEqual({});
        expect(s.labjack_on_off).toBe(true);
    });

    it('does not change the shared defaults', () => {
        normalizeSensorSettings({ channels_enabled: [0] }).data_formats.push('x');
        expect(DEFAULT_SENSOR_SETTINGS.data_formats).toEqual([]);
    });
});

describe('normalizeLabJackConfig', () => {
    it('returns null for values that are not objects', () => {
        expect(normalizeLabJackConfig(null)).toBeNull();
        expect(normalizeLabJackConfig('text')).toBeNull();
        expect(normalizeLabJackConfig(42)).toBeNull();
    });

    it('fills top-level defaults', () => {
        const c = normalizeLabJackConfig({})!;
        expect(c).toMatchObject({
            labjack_name: 'unknown',
            asset_number: 0,
            max_channels: 8,
            site_id: '',
            box_id: '',
            source_type: 'labjack',
            source_id: '',
            nats_subject: 'avenars',
            nats_stream: 'labjacks',
            rotate_secs: 60
        });
        expect(c.sensor_settings.scan_rate_hz).toBe(1000);
    });

    it('falls back to labjack_name for source_id and converts numbers', () => {
        const c = normalizeLabJackConfig({ labjack_name: 'lj2', asset_number: '1456', rotate_secs: '30' })!;
        expect(c.source_id).toBe('lj2');
        expect(c.asset_number).toBe(1456);
        expect(c.rotate_secs).toBe(30);
    });
});

describe('sanitizeLabJackConfig', () => {
    it('keeps only the known fields and normalizes sensor settings', () => {
        const raw = {
            labjack_name: 'lj1',
            asset_number: '7' as unknown as number,
            max_channels: 8,
            site_id: 'i69',
            box_id: 'mu1',
            source_type: 'labjack',
            source_id: 'lj1',
            nats_subject: 'avenars',
            nats_stream: 'labjacks',
            rotate_secs: 60,
            sensor_settings: { channels_enabled: [0] } as any,
            extra: 'dropped'
        };
        const c = sanitizeLabJackConfig(raw);
        expect(c).not.toHaveProperty('extra');
        expect(c.asset_number).toBe(7);
        expect(c.sensor_settings.data_formats).toEqual(['voltage']);
    });
});
