import { describe, expect, it } from 'vitest';
import {
    channelsMissingUnit,
    DEFAULT_SENSOR_SETTINGS,
    findSensorType,
    normalizeLabJackConfig,
    normalizeSensorSettings,
    sanitizeLabJackConfig,
    sensorFormatForUnit
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

/** A config as the webapp saves it, with the given sensor settings. */
function configWith(sensor: any) {
    return {
        labjack_name: 'lj2',
        asset_number: 1456,
        max_channels: 8,
        site_id: 'i69',
        box_id: 'i69-mu2',
        source_type: 'labjack',
        source_id: 'i69-lj2',
        nats_subject: 'avenars',
        nats_stream: 'labjacks',
        rotate_secs: 60,
        sensor_settings: sensor
    };
}

/** Load from KV (JSON text) and save again, as the /labjacks page does. */
function roundTrip(sensor: any) {
    const loaded = normalizeLabJackConfig(JSON.parse(JSON.stringify(configWith(sensor))))!;
    return JSON.parse(JSON.stringify(sanitizeLabJackConfig(loaded))).sensor_settings;
}

describe('calibration units on save', () => {
    it('keeps an older config with id and measurement_units, adding the unit to the calibration', () => {
        const saved = roundTrip({
            channels_enabled: [0, 6],
            data_formats: ['voltage', 'voltage'],
            measurement_units: ['V', 'kPa'],
            calibrations: {
                '0': { type: 'identity' },
                '6': { id: 'tp3505', type: 'linear', a: 2, b: -1 }
            }
        });
        expect(saved.calibrations).toEqual({
            '0': { type: 'identity', unit: 'V' },
            '6': { id: 'tp3505', type: 'linear', a: 2, b: -1, unit: 'kPa' }
        });
        expect(saved.measurement_units).toEqual(['V', 'kPa']);
        expect(saved.data_formats).toEqual(['voltage', 'pressure']);
    });

    it('keeps the MU2 strain calibration unchanged and labels it µε once the unit is chosen', () => {
        const mu2 = {
            channels_enabled: [6],
            data_formats: ['voltage'],
            measurement_units: ['V'],
            calibrations: { '6': { id: 'sg194', type: 'linear', a: 481.26, b: 1058.722 } }
        };
        // Without a unit the calibration is left as it was and flagged.
        const before = normalizeSensorSettings(mu2);
        expect(channelsMissingUnit(before)).toEqual([6]);
        expect(roundTrip(mu2).calibrations['6']).toEqual({ id: 'sg194', type: 'linear', a: 481.26, b: 1058.722 });

        // After the owner picks µε in the form.
        const picked = { ...mu2, calibrations: { '6': { ...mu2.calibrations['6'], unit: 'µε' } } };
        const saved = roundTrip(picked);
        expect(saved.calibrations['6']).toEqual({ id: 'sg194', type: 'linear', a: 481.26, b: 1058.722, unit: 'µε' });
        expect(saved.measurement_units).toEqual(['µε']);
        expect(saved.data_formats).toEqual(['strain']);
        expect(channelsMissingUnit(normalizeSensorSettings(saved))).toEqual([]);
    });

    it('round-trips a new config with units unchanged', () => {
        const sensor = {
            scans_per_read: 200,
            scan_rate_hz: 1000,
            channels_enabled: [0, 1, 2, 3],
            gains: 1,
            data_formats: ['voltage', 'pressure', 'strain', 'strain'],
            measurement_units: ['V', 'kPa', 'µε', 'mV/V'],
            labjack_on_off: true,
            calibrations: {
                '0': { type: 'identity', unit: 'V' },
                '1': { type: 'polynomial', coeffs: [1, 2, 0.5], unit: 'kPa' },
                '2': { type: 'linear', a: 481.26, b: -240.63, unit: 'µε' },
                '3': { type: 'linear', a: 1, b: 0, unit: 'mV/V' }
            }
        };
        expect(roundTrip(sensor)).toEqual(sensor);
        expect(roundTrip(roundTrip(sensor))).toEqual(sensor);
    });

    it('makes measurement_units follow the calibration unit', () => {
        const saved = roundTrip({
            channels_enabled: [1],
            data_formats: ['pressure'],
            measurement_units: ['PSI'],
            calibrations: { '1': { type: 'linear', a: 3, b: 0, unit: 'kPa' } }
        });
        expect(saved.measurement_units).toEqual(['kPa']);
        expect(saved.data_formats).toEqual(['pressure']);
    });

    it('labels identity channels V and keeps their sensor type', () => {
        const saved = roundTrip({
            channels_enabled: [2],
            data_formats: ['strain'],
            measurement_units: ['µε'],
            calibrations: {}
        });
        expect(saved.calibrations).toEqual({ '2': { type: 'identity', unit: 'V' } });
        expect(saved.measurement_units).toEqual(['V']);
        expect(saved.data_formats).toEqual(['strain']);
    });

    it('leaves calibrations of channels that are not enabled alone', () => {
        const saved = roundTrip({
            channels_enabled: [0],
            calibrations: { '5': { type: 'linear', a: 2, b: 0 } }
        });
        expect(saved.calibrations['5']).toEqual({ type: 'linear', a: 2, b: 0 });
    });
});

describe('sensor types', () => {
    it('offers strain gauges in µε, mV/V and V', () => {
        expect(findSensorType('strain')).toEqual({ format: 'strain', label: 'Strain gauge', units: ['µε', 'mV/V', 'V'] });
        expect(findSensorType('pressure')?.units).toContain('kPa');
        expect(findSensorType('nonsense')).toBeUndefined();
    });

    it('picks the sensor type from the unit', () => {
        expect(sensorFormatForUnit('voltage', 'µε')).toBe('strain');
        expect(sensorFormatForUnit('voltage', 'kPa')).toBe('pressure');
        expect(sensorFormatForUnit('strain', 'V')).toBe('strain');
        expect(sensorFormatForUnit('voltage', 'furlongs')).toBe('voltage');
        expect(sensorFormatForUnit('my-sensor', 'kPa')).toBe('my-sensor');
    });
});

describe('filters on load and save', () => {
    it('keeps a config without filters free of a filters field', () => {
        const saved = roundTrip({ channels_enabled: [6], data_formats: ['voltage'], measurement_units: ['V'] });
        expect('filters' in saved).toBe(false);
    });

    it('keeps the active filters of each channel and drops the rest', () => {
        const saved = roundTrip({
            channels_enabled: [6, 7],
            data_formats: ['strain', 'strain'],
            measurement_units: ['V', 'V'],
            filters: {
                '6': { despike: true, remove_10hz: true, remove_11_9hz: false, highpass_hz: 1, lowpass_hz: null },
                '7': { despike: false, highpass_hz: 0 },
                bad: { despike: true }
            }
        });
        expect(saved.filters).toEqual({ '6': { despike: true, remove_10hz: true, highpass_hz: 1 } });
    });
});
