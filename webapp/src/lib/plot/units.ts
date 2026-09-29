/**
 * Which unit a live-plot channel's values are in, from its calibration.
 *
 * The plot shows calibrated values (see `stream.ts`). A channel with the identity
 * calibration shows raw LabJack volts. A calibrated channel shows values in the unit
 * stored with its calibration (`calibrations[ch].unit`); configs written before
 * calibrations carried a unit fall back to the channel's `measurement_units` entry
 * (see `resolveCalibrationUnit`).
 *
 * @module
 */
import { normalizeCalibration, RAW_UNIT, resolveCalibrationUnit, type CalibrationSpec } from '../calibration';

/** Unit description of one channel, from {@link describeChannelUnit}. */
export interface ChannelUnitInfo {
    /** Unit of the plotted values, for the axis title, Latest, threshold and badges. */
    unit: string;
    /** `true` when a non-identity calibration converts the raw volts. */
    calibrated: boolean;
    /** Short tag, e.g. `raw volts` or `calibrated → kPa`. */
    tag: string;
    /** Set when the unit of the values is not known or not what the config says. */
    warning?: string;
}

/**
 * Describes the unit of a channel's plotted values.
 *
 * - Identity calibration: values are raw volts, so the unit is `V`. A configured unit
 *   other than `V` (possible only in older configs) is reported in `warning`, since
 *   the values are not in it.
 * - Any other calibration: the unit from {@link resolveCalibrationUnit}. Only an older
 *   config whose calibration has no unit and whose `measurement_units` entry is blank
 *   or `V` has none; the values are then labelled `V` and `warning` asks for the unit
 *   to be set in the LabJack config.
 *
 * @param calibration - Normalized calibration of the channel.
 * @param measurementUnit - The channel's `measurement_units` entry, if any.
 * @returns The unit, whether the values are calibrated, a tag and an optional warning.
 */
export function describeChannelUnit(
    calibration: CalibrationSpec,
    measurementUnit: string | undefined | null
): ChannelUnitInfo {
    if (calibration.type === 'identity') {
        const configured = (measurementUnit ?? '').trim();
        const warning = configured && configured !== RAW_UNIT
            ? `The configured unit is ${configured}, but the channel has no calibration, so values are raw volts.`
            : undefined;
        return { unit: RAW_UNIT, calibrated: false, tag: 'raw volts', warning };
    }

    const unit = resolveCalibrationUnit(calibration, measurementUnit);
    if (unit === undefined) {
        return {
            unit: RAW_UNIT,
            calibrated: true,
            tag: `calibrated: ${calibration.type} → unit not set`,
            warning: 'This channel has a calibration but no unit. Edit the LabJack config and choose the unit the calibration converts to.'
        };
    }
    return { unit, calibrated: true, tag: `calibrated → ${unit}` };
}

/** Fields of `sensor_settings` that describe a channel's unit. */
export interface ChannelUnitSource {
    channels_enabled: number[];
    measurement_units: string[];
    calibrations?: Record<string, Partial<CalibrationSpec>>;
}

/**
 * Describes the unit of one channel of a config, from its calibration and its
 * `measurement_units` entry (see {@link describeChannelUnit}).
 *
 * @param settings - `sensor_settings` of the config, or `null` before it loads.
 * @param channel - LabJack channel number.
 * @returns The unit description. Raw volts before the config loads.
 */
export function channelUnitInfo(settings: ChannelUnitSource | null | undefined, channel: number): ChannelUnitInfo {
    const calibration = normalizeCalibration(settings?.calibrations?.[String(channel)]);
    const index = settings?.channels_enabled.indexOf(channel) ?? -1;
    const measurementUnit = index >= 0 ? settings?.measurement_units[index] : undefined;
    return describeChannelUnit(calibration, measurementUnit);
}
