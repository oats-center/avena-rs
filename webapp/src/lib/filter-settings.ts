/**
 * Per-channel noise filter settings, stored in a LabJack config under
 * `sensor_settings.filters`, keyed by channel number as a string:
 *
 * ```json
 * "filters": { "6": { "despike": true, "remove_10hz": true, "remove_11_9hz": true,
 *                     "highpass_hz": 1.0, "lowpass_hz": 100.0 } }
 * ```
 *
 * Every field is optional; a missing, `false`, `null`, zero or negative value turns that
 * filter off, and a missing map means no filtering. The filters only change what is
 * read (live plots and exports); the streamer and archiver ignore them and the archive
 * keeps the raw readings. See `docs/src/reference/kv-config.md` and `filters.ts`.
 *
 * @module
 */

/** Filters of one channel. */
export interface ChannelFilterSettings {
    /** Strip narrow upward spikes (grey opening over 2.5 ms). */
    despike?: boolean;
    /** Subtract the 10 Hz square wave locked to the LabJack clock. */
    remove_10hz?: boolean;
    /** Subtract the 11.9 Hz square wave locked to the LabJack clock. */
    remove_11_9hz?: boolean;
    /** High-pass cutoff in Hz (2nd-order Butterworth). */
    highpass_hz?: number;
    /** Low-pass cutoff in Hz (2nd-order Butterworth). */
    lowpass_hz?: number;
}

/** Filters per channel, keyed by channel number as a string. */
export type ChannelFilterMap = Record<string, ChannelFilterSettings>;

/**
 * Reads a cutoff frequency.
 *
 * @param value - Stored value.
 * @returns The value when it is a finite number above 0, else `undefined` (off).
 */
function cutoff(value: unknown): number | undefined {
    const number = typeof value === 'string' && value.trim() !== '' ? Number(value) : value;
    return typeof number === 'number' && Number.isFinite(number) && number > 0 ? number : undefined;
}

/**
 * Cleans one channel's filter settings.
 *
 * Switches count as on only when they are exactly `true`; cutoffs only when they are
 * finite numbers above 0 (numeric strings are accepted). Everything else is dropped.
 *
 * @param raw - Stored value, possibly malformed.
 * @returns The settings with only the active filters, or `undefined` when none is on.
 */
export function normalizeChannelFilters(raw: unknown): ChannelFilterSettings | undefined {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return undefined;
    const source = raw as Record<string, unknown>;
    const settings: ChannelFilterSettings = {};
    if (source.despike === true) settings.despike = true;
    if (source.remove_10hz === true) settings.remove_10hz = true;
    if (source.remove_11_9hz === true) settings.remove_11_9hz = true;
    const highpass = cutoff(source.highpass_hz);
    if (highpass !== undefined) settings.highpass_hz = highpass;
    const lowpass = cutoff(source.lowpass_hz);
    if (lowpass !== undefined) settings.lowpass_hz = lowpass;
    return Object.keys(settings).length > 0 ? settings : undefined;
}

/**
 * Cleans a whole `filters` map: keeps keys that are channel numbers (non-negative
 * integers) whose settings have at least one active filter.
 *
 * @param raw - Stored `sensor_settings.filters`, possibly missing or malformed.
 * @returns A new map, empty when nothing is filtered.
 */
export function normalizeFilterMap(raw: unknown): ChannelFilterMap {
    const map: ChannelFilterMap = {};
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return map;
    for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
        if (!/^\d+$/.test(key)) continue;
        const settings = normalizeChannelFilters(value);
        if (settings) map[String(Number(key))] = settings;
    }
    return map;
}

/**
 * Tells whether any filter is on.
 *
 * @param settings - One channel's settings, or `undefined`.
 */
export function hasActiveFilters(settings: ChannelFilterSettings | undefined): boolean {
    return normalizeChannelFilters(settings) !== undefined;
}

/**
 * Short description of a channel's filters for badges and tooltips.
 *
 * @param settings - One channel's settings.
 * @returns E.g. `despike, −10 Hz, −11.9 Hz, HP 1 Hz, LP 100 Hz`, or `""` when none is on.
 */
export function describeFilters(settings: ChannelFilterSettings | undefined): string {
    const clean = normalizeChannelFilters(settings);
    if (!clean) return '';
    const parts: string[] = [];
    if (clean.despike) parts.push('despike');
    if (clean.remove_10hz) parts.push('−10 Hz');
    if (clean.remove_11_9hz) parts.push('−11.9 Hz');
    if (clean.highpass_hz !== undefined) parts.push(`HP ${clean.highpass_hz} Hz`);
    if (clean.lowpass_hz !== undefined) parts.push(`LP ${clean.lowpass_hz} Hz`);
    return parts.join(', ');
}
