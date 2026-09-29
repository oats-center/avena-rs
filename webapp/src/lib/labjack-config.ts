/**
 * LabJack config documents as stored in KV bucket `avenabox` under
 * `<site>.<box>.<source>.config`, and the helpers that fill in missing fields.
 *
 * See `docs/src/reference/kv-config.md` for the fields.
 */
import {
    normalizeCalibration,
    resolveCalibrationUnit,
    type CalibrationSpec
} from "./calibration";

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
    /**
     * Sensor type of each enabled channel (see {@link SENSOR_TYPES}), same order as
     * `channels_enabled`. Label only.
     */
    data_formats: string[];
    /**
     * Unit of each enabled channel after calibration, same order. A copy of the
     * calibration's `unit` kept for older readers; the calibration's `unit` wins.
     */
    measurement_units: string[];
    /** `false` stops streaming. This is a setting, not a live status. */
    labjack_on_off: boolean;
    /** Volts-to-units conversion and its unit per channel, keyed by channel number as a string. */
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
 * Converts the numeric top-level fields with `Number()`, normalizes the sensor
 * settings with {@link normalizeSensorSettings} and writes each enabled channel's unit
 * into its calibration and `measurement_units` with {@link syncChannelCalibrations}. Only the fields listed in
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
        sensor_settings: syncChannelCalibrations(normalizeSensorSettings(raw.sensor_settings))
    };
}

/** A kind of sensor offered for a channel, stored in `data_formats`. */
export interface SensorType {
    /** Value stored in `data_formats`. */
    format: string;
    /** Name shown in the config form. */
    label: string;
    /** Units offered for a calibrated channel of this type; the first is the default. */
    units: string[];
}

/**
 * Sensor types offered in the config form. A channel whose calibration is identity is
 * in volts whatever its type.
 */
export const SENSOR_TYPES: readonly SensorType[] = Object.freeze([
    { format: "voltage", label: "Voltage", units: ["V", "mV"] },
    { format: "strain", label: "Strain gauge", units: ["µε", "mV/V", "V"] },
    { format: "pressure", label: "Pressure", units: ["kPa", "Pa", "bar", "PSI"] },
    { format: "temperature", label: "Temperature", units: ["°C"] },
    { format: "current", label: "Current", units: ["A", "mA"] },
    { format: "resistance", label: "Resistance", units: ["Ω"] }
]);

/**
 * Looks up a sensor type by its `data_formats` value.
 *
 * @param format - Stored `data_formats` entry.
 * @returns The type, or `undefined` for a value not in {@link SENSOR_TYPES}.
 */
export function findSensorType(format: string | undefined): SensorType | undefined {
    return SENSOR_TYPES.find((type) => type.format === format);
}

/**
 * Picks the `data_formats` value that matches a calibrated channel's unit.
 *
 * Keeps `format` when its sensor type offers `unit`, or when `format` is not a known
 * type (a custom value from KV is left alone). Otherwise returns the first sensor type
 * that offers `unit`, or `format` unchanged when none does.
 *
 * @param format - Current `data_formats` entry.
 * @param unit - Unit of the channel's calibration.
 * @returns The sensor type format to store.
 */
export function sensorFormatForUnit(format: string | undefined, unit: string): string {
    const current = findSensorType(format);
    if (format && !current) return format;
    if (current?.units.includes(unit)) return current.format;
    return SENSOR_TYPES.find((type) => type.units.includes(unit))?.format ?? format ?? "voltage";
}

/**
 * Makes each enabled channel's calibration, unit and sensor type agree, in place.
 *
 * For every enabled channel, the calibration is normalized (a missing one becomes
 * identity) and its unit set from {@link resolveCalibrationUnit}: `V` for identity,
 * else the calibration's own unit, else a non-`V` `measurement_units` entry from an
 * older config. That unit is copied to `measurement_units` so older readers see it,
 * and for a calibrated channel `data_formats` is moved to a sensor type offering it
 * (see {@link sensorFormatForUnit}). A calibrated channel with no known unit keeps its
 * calibration without `unit` and its old labels; the form does not save such a channel.
 * An `id` from an older config is kept. Calibrations of channels that are not enabled
 * are left as they are.
 *
 * @param sensor - Normalized sensor settings; modified.
 * @returns `sensor`, for chaining.
 */
export function syncChannelCalibrations(sensor: SensorSettings): SensorSettings {
    const calibrations = { ...(sensor.calibrations ?? {}) };
    sensor.channels_enabled.forEach((channel, index) => {
        const key = String(channel);
        const calibration = normalizeCalibration(calibrations[key]);
        const unit = resolveCalibrationUnit(calibration, sensor.measurement_units[index]);
        if (unit === undefined) {
            calibrations[key] = calibration;
            return;
        }
        calibrations[key] = { ...calibration, unit };
        sensor.measurement_units[index] = unit;
        if (calibration.type !== "identity") {
            sensor.data_formats[index] = sensorFormatForUnit(sensor.data_formats[index], unit);
        }
    });
    sensor.calibrations = calibrations;
    return sensor;
}

/**
 * Lists enabled channels that have a calibration but no unit for it.
 *
 * @param sensor - Sensor settings.
 * @returns Channel numbers, in `channels_enabled` order.
 */
export function channelsMissingUnit(sensor: SensorSettings): number[] {
    return sensor.channels_enabled.filter((channel, index) => {
        const calibration = normalizeCalibration(sensor.calibrations?.[String(channel)]);
        return resolveCalibrationUnit(calibration, sensor.measurement_units[index]) === undefined;
    });
}
