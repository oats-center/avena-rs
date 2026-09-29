import { describe, expect, it } from 'vitest';
import {
    buildExportRequest,
    defaultExportRange,
    formatBytes,
    formatUtcPreview,
    missingChannelsWarning,
    toLocalInputValue,
    toRfc3339
} from './export-form';

describe('export form helpers', () => {
    it('formats local input values to the second', () => {
        expect(toLocalInputValue(new Date(2026, 0, 2, 3, 4, 5))).toBe('2026-01-02T03:04:05');
        const range = defaultExportRange(new Date(2026, 0, 2, 3, 4, 5));
        expect(range).toEqual({ start: '2026-01-02T03:02:05', end: '2026-01-02T03:04:05' });
    });
    it('converts to UTC', () => {
        const local = toLocalInputValue(new Date(Date.UTC(2026, 8, 28, 12, 0, 5)));
        expect(toRfc3339(local)).toBe('2026-09-28T12:00:05.000Z');
        expect(formatUtcPreview(local)).toBe('2026-09-28 12:00:05 UTC');
        expect(formatUtcPreview('')).toBe('');
        expect(formatUtcPreview('nonsense')).toBe('');
        expect(() => toRfc3339('nonsense')).toThrow();
    });
    it('formats sizes', () => {
        expect(formatBytes(512)).toBe('512 B');
        expect(formatBytes(1536)).toBe('1.5 KB');
        expect(formatBytes(12 * 1024 * 1024)).toBe('12.0 MB');
        expect(formatBytes(3 * 1024 ** 4)).toBe('3072.0 GB');
    });
    it('warns about missing channels', () => {
        expect(missingChannelsWarning([])).toBe('');
        expect(missingChannelsWarning([3, 12])).toBe('No samples found for channels: 03, 12. Continuing with remaining channels.');
    });
});

describe('buildExportRequest', () => {
    const config = { asset_number: 7, box_id: 'i69-mu1', labjack_name: 'LJ 2  north' };
    const start = '2026-09-28T12:00:00';
    const end = '2026-09-28T12:02:00';
    it('validates in order', () => {
        expect(buildExportRequest(null, start, end, [1])).toEqual({ ok: false, error: 'Configuration not loaded' });
        expect(buildExportRequest(config, '', end, [1])).toEqual({ ok: false, error: 'Please select a start and end time' });
        expect(buildExportRequest(config, start, end, [])).toEqual({ ok: false, error: 'Select at least one channel' });
        expect(buildExportRequest(config, 'x', end, [1])).toEqual({ ok: false, error: 'Invalid date/time selection' });
        expect(buildExportRequest(config, end, start, [1])).toEqual({ ok: false, error: 'Start time must be before end time' });
    });
    it('builds the payload', () => {
        const result = buildExportRequest(config, start, end, new Set([9, 2]));
        expect(result).toEqual({
            ok: true,
            payload: {
                asset: 7,
                channels: [2, 9],
                start: toRfc3339(start),
                end: toRfc3339(end),
                box_id: 'i69-mu1',
                download_name: 'LJ_2_north.csv'
            }
        });
        const bare = buildExportRequest({ asset_number: 7, box_id: '', labjack_name: '' }, start, start, [1]);
        expect(bare.ok && bare.payload.box_id).toBeUndefined();
        expect(bare.ok && bare.payload.download_name).toBeUndefined();
    });
});
