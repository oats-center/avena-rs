import { describe, expect, it } from 'vitest';
import { describeFilters, hasActiveFilters, normalizeChannelFilters, normalizeFilterMap } from './filter-settings';

describe('normalizeChannelFilters', () => {
    it('treats missing, false, null, zero and negative values as off', () => {
        expect(normalizeChannelFilters(undefined)).toBeUndefined();
        expect(normalizeChannelFilters({})).toBeUndefined();
        expect(
            normalizeChannelFilters({ despike: false, remove_10hz: null, highpass_hz: 0, lowpass_hz: -5 })
        ).toBeUndefined();
    });

    it('keeps true switches and positive cutoffs, including numeric strings', () => {
        expect(
            normalizeChannelFilters({ despike: true, remove_11_9hz: true, highpass_hz: '1.5', lowpass_hz: 100, extra: 1 })
        ).toEqual({ despike: true, remove_11_9hz: true, highpass_hz: 1.5, lowpass_hz: 100 });
    });

    it('does not take truthy non-boolean switches as on', () => {
        expect(normalizeChannelFilters({ despike: 'yes', remove_10hz: 1 })).toBeUndefined();
    });
});

describe('normalizeFilterMap', () => {
    it('keeps channel keys with active filters only', () => {
        expect(
            normalizeFilterMap({ '6': { despike: true }, '07': { lowpass_hz: 50 }, '8': { despike: false }, x: { despike: true } })
        ).toEqual({ '6': { despike: true }, '7': { lowpass_hz: 50 } });
        expect(normalizeFilterMap(null)).toEqual({});
        expect(normalizeFilterMap([1, 2])).toEqual({});
    });
});

describe('describeFilters', () => {
    it('lists the active filters in pipeline order', () => {
        expect(
            describeFilters({ lowpass_hz: 100, despike: true, highpass_hz: 1, remove_10hz: true, remove_11_9hz: true })
        ).toBe('despike, −10 Hz, −11.9 Hz, HP 1 Hz, LP 100 Hz');
        expect(describeFilters(undefined)).toBe('');
        expect(hasActiveFilters({ despike: true })).toBe(true);
        expect(hasActiveFilters({ despike: false })).toBe(false);
    });
});
