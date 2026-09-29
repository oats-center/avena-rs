/**
 * LabJack config documents as stored in KV bucket `avenabox` under
 * `<site>.<box>.<source>.config`, and the helpers that fill in missing fields.
 *
 * See `docs/src/reference/kv-config.md` for the fields.
 */
import type { CalibrationSpec } from "./calibration";

/** `sensor_settings` of a LabJack config document. */
export interface SensorSettings {
    /** Scans per read, and so samples per published message on each channel. */
    scans_per_read: number;
    /** Scans per second, per channel. */
    scan_rate_hz: number;
    /** Analog inputs to stream (`AIN<n>`). */
    channels_enabled: number[];
    /** Edited in the webapp but not used by the streamer. */
    gains: number;
    /** What each enabled channel measures, same order as `channels_enabled`. Label only. */
    data_formats: string[];
    /** Unit of each enabled channel after calibration, same order. Label only. */
    measurement_units: string[];
    /** `false` stops streaming. This is a setting, not a live status. */
    labjack_on_off: boolean;
    /** Volts-to-units conversion per channel, keyed by channel number as a string. */
    calibrations?: Record<string, CalibrationSpec>;
}

/** One LabJack config document. */
export interface LabJackConfig {
    /** Display name; must be unique (case-insensitive) when adding. */
    labjack_name: string;
    /** Asset number, > 0; must be unique when adding. Used in the archive path. */
    asset_number: number;
    /** Number of inputs offered as channel toggles, 1 to 16. Not used by the streamer. */
    max_channels: number;
    /** Site name, first subject token. */
    site_id?: string;
    /** Edge node name, second subject token. */
    box_id?: string;
    /** Kind of source, normally `labjack`. */
    source_type?: string;
    /** Name of this LabJack in subjects, third token. */
    source_id?: string;
    /** Subject root, normally `avenars`. Labeled "NATS Root" in the edit form. */
    nats_subject: string;
    /** JetStream stream for live samples, normally `labjacks`. */
    nats_stream: string;
    /** Archive file window, seconds. */
    rotate_secs: number;
    /** What to record. */
    sensor_settings: SensorSettings;
}

/**
 * Fallback values used by {@link normalizeSensorSettings} for missing or invalid fields.
 */
export const DEFAULT_SENSOR_SETTINGS: Readonly<SensorSettings> = Object.freeze({
    scans_per_read: 200,
    scan_rate_hz: 1000,
    channels_enabled: [],
    gains: 1,
    data_formats: [],
    measurement_units: [],
    labjack_on_off: false,
    calibrations: {}
});

/**
 * Builds a complete sensor settings object from a raw `sensor_settings` value.
 *
 * Reads the older field names `scan_rate` (for `scans_per_read`) and `sampling_rate`
 * (for `scan_rate_hz`) when the new ones are absent. Missing or non-finite numbers take
 * the values in {@link DEFAULT_SENSOR_SETTINGS}. `data_formats` and `measurement_units`
 * are padded with `"voltage"` and `"V"` to one entry per enabled channel. Arrays and
 * `calibrations` are shallow copies.
 *
 * @param rawSensor - Parsed `sensor_settings` from KV or from the edit modal. May be
 *   `undefined` or partial.
 * @returns A new settings object with every field set.
 */
export function normalizeSensorSettings(rawSensor: any): SensorSettings {
    const sensor: SensorSettings = {
        scans_per_read: Number(
            rawSensor?.scans_per_read ?? rawSensor?.scan_rate ?? DEFAULT_SENSOR_SETTINGS.scans_per_read
        ),
        scan_rate_hz: Number(
            rawSensor?.scan_rate_hz ?? rawSensor?.sampling_rate ?? DEFAULT_SENSOR_SETTINGS.scan_rate_hz
        ),
        channels_enabled: Array.isArray(rawSensor?.channels_enabled) ? [...rawSensor.channels_enabled] : [],
        gains: Number(rawSensor?.gains ?? DEFAULT_SENSOR_SETTINGS.gains),
        data_formats: Array.isArray(rawSensor?.data_formats) ? [...rawSensor.data_formats] : [],
        measurement_units: Array.isArray(rawSensor?.measurement_units) ? [...rawSensor.measurement_units] : [],
        labjack_on_off: Boolean(rawSensor?.labjack_on_off),
        calibrations:
            rawSensor?.calibrations && typeof rawSensor.calibrations === "object"
                ? { ...rawSensor.calibrations }
                : {}
    };

    if (!Number.isFinite(sensor.scans_per_read)) sensor.scans_per_read = DEFAULT_SENSOR_SETTINGS.scans_per_read;
    if (!Number.isFinite(sensor.scan_rate_hz)) sensor.scan_rate_hz = DEFAULT_SENSOR_SETTINGS.scan_rate_hz;
    if (!Number.isFinite(sensor.gains)) sensor.gains = DEFAULT_SENSOR_SETTINGS.gains;
    while (sensor.data_formats.length < sensor.channels_enabled.length) sensor.data_formats.push("voltage");
    while (sensor.measurement_units.length < sensor.channels_enabled.length) sensor.measurement_units.push("V");

    return sensor;
}

/**
 * Fills in defaults for a config read from KV.
 *
 * Defaults: `labjack_name` `"unknown"`, `asset_number` 0, `max_channels` 8, empty
 * `site_id` and `box_id`, `source_type` `"labjack"`, `source_id` falls back to
 * `labjack_name`, `nats_subject` `"avenars"`, `nats_stream` `"labjacks"`, `rotate_secs`
 * 60.
 *
 * @param raw - Parsed JSON value of a `*.*.*.config` key.
 * @returns The normalized config, or `null` when `raw` is not an object.
 */
export function normalizeLabJackConfig(raw: any): LabJackConfig | null {
    if (!raw || typeof raw !== "object") return null;

    return {
        labjack_name: raw.labjack_name ?? "unknown",
        asset_number: Number(raw.asset_number ?? 0),
        max_channels: Number(raw.max_channels ?? 8),
        site_id: raw.site_id ?? "",
        box_id: raw.box_id ?? "",
        source_type: raw.source_type ?? "labjack",
        source_id: raw.source_id ?? raw.labjack_name ?? "",
        nats_subject: raw.nats_subject ?? "avenars",
        nats_stream: raw.nats_stream ?? "labjacks",
        rotate_secs: Number(raw.rotate_secs ?? 60),
        sensor_settings: normalizeSensorSettings(raw.sensor_settings ?? {})
    };
}

/**
 * Copies a config from the edit modal into the shape written to KV.
 *
 * Converts the numeric top-level fields with `Number()` and normalizes the sensor
 * settings with {@link normalizeSensorSettings}. Only the fields listed in
 * {@link LabJackConfig} are kept; any other field on `raw` is dropped.
 *
 * @param raw - Config returned by `LabJackConfigModal`.
 * @returns A new config object.
 */
export function sanitizeLabJackConfig(raw: LabJackConfig): LabJackConfig {
    return {
        labjack_name: raw.labjack_name,
        asset_number: Number(raw.asset_number),
        max_channels: Number(raw.max_channels),
        site_id: raw.site_id,
        box_id: raw.box_id,
        source_type: raw.source_type,
        source_id: raw.source_id,
        nats_subject: raw.nats_subject,
        nats_stream: raw.nats_stream,
        rotate_secs: Number(raw.rotate_secs),
        sensor_settings: normalizeSensorSettings(raw.sensor_settings)
    };
}
