import { describe, expect, it } from 'vitest';
import { archiveExportRequestSubject, labjackConfigKey, liveLabJackChannelSubject } from './subjects';

describe('labjackConfigKey', () => {
    it('leaves keys with a full identity unchanged', () => {
        const mu1 = { site_id: 'i69', box_id: 'i69-mu1', source_id: 'i69-lj2', labjack_name: 'LJ2', asset_number: 1456 };
        expect(labjackConfigKey(mu1)).toBe('i69.i69-mu1.i69-lj2.config');
        expect(labjackConfigKey({ ...mu1, box_id: 'i69-mu2' })).toBe('i69.i69-mu2.i69-lj2.config');
        expect(labjackConfigKey({ site_id: 'i69', box_id: 'i69-mu1', source_id: 'i69-lj2' })).toBe('i69.i69-mu1.i69-lj2.config');
    });

    it('falls back like the subjects: source_id, labjack_name, asset<NNN>', () => {
        const base = { site_id: 'i69', box_id: 'i69-mu1', asset_number: 7 };
        expect(labjackConfigKey({ ...base, labjack_name: 'Unit A' })).toBe('i69.i69-mu1.unit-a.config');
        expect(labjackConfigKey(base)).toBe('i69.i69-mu1.asset007.config');
        expect(labjackConfigKey({ ...base, asset_number: 1456 })).toBe('i69.i69-mu1.asset1456.config');
    });

    it('treats empty strings as missing, like Rust', () => {
        const empty = { site_id: '', box_id: '', source_id: '', labjack_name: '', asset_number: 7 };
        expect(labjackConfigKey(empty)).toBe('unknown-site.unknown-box.asset007.config');
    });

    it('uses the same source token as the live and export subjects', () => {
        const config = { nats_subject: 'avenars', site_id: 'i69', box_id: 'i69-mu1', source_id: '', labjack_name: '', asset_number: 42 };
        const source = labjackConfigKey(config).split('.')[2];
        expect(liveLabJackChannelSubject(config, 1).split('.')[3]).toBe(source);
        expect(archiveExportRequestSubject(config).split('.')[3]).toBe(source);
    });

    it('uses unknown-source only without an asset number', () => {
        expect(labjackConfigKey({ site_id: 'a', box_id: 'b' })).toBe('a.b.unknown-source.config');
        expect(labjackConfigKey({ site_id: 'a', box_id: 'b', asset_number: null })).toBe('a.b.unknown-source.config');
        expect(labjackConfigKey({ site_id: 'a', box_id: 'b', asset_number: Number.NaN })).toBe('a.b.unknown-source.config');
    });
});
