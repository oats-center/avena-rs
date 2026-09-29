/**
 * Pure helpers for the export dialog of the plot page: time inputs, validation, the
 * request payload and the texts shown while downloading.
 *
 * @module
 */
import type { ExportRequestPayload } from '../exporter';
import { normalizeChannelFilters, type ChannelFilterMap } from '../filter-settings';

/** Default length of the export range, ending now, in ms. */
export const DEFAULT_EXPORT_RANGE_MS = 2 * 60 * 1000;

/** Two-digit zero padding. */
const pad = (value: number) => value.toString().padStart(2, '0');

/**
 * Formats a date for a `datetime-local` input, in local time, to the second.
 *
 * @param date - Date to format.
 * @returns `YYYY-MM-DDTHH:mm:ss`.
 */
export function toLocalInputValue(date: Date): string {
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/**
 * Default range of the export form: the last {@link DEFAULT_EXPORT_RANGE_MS} up to
 * `now`, as `datetime-local` values.
 *
 * @param now - End of the range.
 */
export function defaultExportRange(now: Date): { start: string; end: string } {
    return {
        start: toLocalInputValue(new Date(now.getTime() - DEFAULT_EXPORT_RANGE_MS)),
        end: toLocalInputValue(now)
    };
}

/**
 * Describes the browser's time zone, in which the export times are entered.
 *
 * @returns E.g. `Europe/Berlin (UTC+02:00)`.
 */
export function describeLocalTimeZone(): string {
    const name = Intl.DateTimeFormat().resolvedOptions().timeZone || 'local time';
    const offsetMin = -new Date().getTimezoneOffset();
    const sign = offsetMin >= 0 ? '+' : '-';
    const abs = Math.abs(offsetMin);
    return `${name} (UTC${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)})`;
}

/**
 * Shows an export input value as UTC, the time the request is sent in.
 *
 * @param value - `datetime-local` value.
 * @returns E.g. `2026-09-28 12:00:05 UTC`, or `""` when the value is not a date.
 */
export function formatUtcPreview(value: string): string {
    const date = new Date(value);
    if (!value || isNaN(date.getTime())) return '';
    return `${date.toISOString().slice(0, 19).replace('T', ' ')} UTC`;
}

/**
 * Converts a `datetime-local` value, read as local time, to an RFC 3339 UTC string.
 *
 * @param value - Value such as `2025-01-31T14:05` or `2025-01-31T14:05:30`.
 * @returns The time from `Date.toISOString()`, e.g. `2025-01-31T19:05:00.000Z` in UTC-5.
 * @throws Error if the value is not a valid date.
 */
export function toRfc3339(value: string): string {
    const date = new Date(value);
    if (isNaN(date.getTime())) {
        throw new Error('Invalid date/time value');
    }
    return date.toISOString();
}

/**
 * Formats a byte count with 1024-based units.
 *
 * @param value - Size in bytes.
 * @returns E.g. `512 B`, `1.5 KB`, `12.0 MB`. Stops at GB.
 */
export function formatBytes(value: number): string {
    const units = ['B', 'KB', 'MB', 'GB'];
    let size = value;
    let unitIndex = 0;
    while (size >= 1024 && unitIndex < units.length - 1) {
        size /= 1024;
        unitIndex += 1;
    }
    return `${size.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
}

/**
 * Warning shown when the exporter reports channels without samples.
 *
 * @param missing - Channels from the `summary` frame.
 * @returns The warning, or `""` when none are missing.
 */
export function missingChannelsWarning(missing: number[]): string {
    if (missing.length === 0) return '';
    const formatted = missing.map((ch) => ch.toString().padStart(2, '0')).join(', ');
    return `No samples found for channels: ${formatted}. Continuing with remaining channels.`;
}

/** Config fields the export request is built from. */
export interface ExportSource {
    asset_number: number;
    box_id?: string | null;
    labjack_name?: string | null;
}

/** Result of {@link buildExportRequest}. */
export type ExportRequestResult =
    | { ok: true; payload: ExportRequestPayload }
    | { ok: false; error: string };

/**
 * Validates the export form and builds the request.
 *
 * Checks that times and at least one channel are set and that start is not after end.
 * The payload carries the asset, the sorted channels, RFC 3339 start and end,
 * `box_id`, and a file name made from `labjack_name` (spaces become `_`). With
 * `filters`, the ticked channels that have filters on are sent in `filters`, so the
 * exporter adds a `filtered_value` column; without any, the field is left out.
 *
 * @param config - Loaded config, or `null`.
 * @param start - Start `datetime-local` value.
 * @param end - End `datetime-local` value.
 * @param channels - Ticked channels.
 * @param filters - The config's `sensor_settings.filters` when filtered values are
 *   wanted, else `undefined`.
 */
export function buildExportRequest(
    config: ExportSource | null,
    start: string,
    end: string,
    channels: Iterable<number>,
    filters?: ChannelFilterMap
): ExportRequestResult {
    if (!config) return { ok: false, error: 'Configuration not loaded' };
    if (!start || !end) return { ok: false, error: 'Please select a start and end time' };
    const sorted = Array.from(channels).sort((a, b) => a - b);
    if (sorted.length === 0) return { ok: false, error: 'Select at least one channel' };

    let startIso: string;
    let endIso: string;
    try {
        startIso = toRfc3339(start);
        endIso = toRfc3339(end);
    } catch {
        return { ok: false, error: 'Invalid date/time selection' };
    }
    if (new Date(startIso) > new Date(endIso)) {
        return { ok: false, error: 'Start time must be before end time' };
    }

    const requestFilters: ChannelFilterMap = {};
    for (const channel of sorted) {
        const settings = normalizeChannelFilters(filters?.[String(channel)]);
        if (settings) requestFilters[String(channel)] = settings;
    }

    return {
        ok: true,
        payload: {
            asset: config.asset_number,
            channels: sorted,
            start: startIso,
            end: endIso,
            box_id: config.box_id || undefined,
            download_name: config.labjack_name ? `${config.labjack_name.replace(/\s+/g, '_')}.csv` : undefined,
            ...(Object.keys(requestFilters).length > 0 ? { filters: requestFilters } : {})
        }
    };
}
