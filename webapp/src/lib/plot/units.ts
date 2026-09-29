/**
 * Which unit a live-plot channel's values are in, from its calibration and its
 * configured measurement unit.
 *
 * The plot shows calibrated values (see `stream.ts`). A channel with the identity
 * calibration shows raw LabJack volts whatever unit is configured. A calibrated
 * channel shows values in its configured `measurement_units` entry, the only place the
 * config names the output unit.
 *
 * @module
 */
import type { CalibrationSpec } from '../calibration';

/** Unit description of one channel, from {@link describeChannelUnit}. */
export interface ChannelUnitInfo {
    /** Unit of the plotted values, for the axis title, Latest, threshold and badges. */
    unit: string;
    /** `true` when a non-identity calibration converts the raw volts. */
    calibrated: boolean;
    /** Short tag, e.g. `raw volts` or `calibrated: asgm194 → kPa`. */
    tag: string;
    /** Set when the configured unit and the calibration do not agree. */
    warning?: string;
}

/**
 * Describes the unit of a channel's plotted values.
 *
 * - Identity calibration: values are raw volts, so the unit is `V`. A configured unit
 *   other than `V` is reported in `warning`, since the values are not in it.
 * - Any other calibration: the unit is the configured one (blank counts as `V`). The
 *   tag names the calibration (its preset id, else its type). If that unit is `V`, a
 *   `warning` asks to check it, since most calibrations convert volts to another
 *   quantity and `V` is also the value filled in when no unit was chosen.
 *
 * @param calibration - Normalized calibration of the channel.
 * @param measurementUnit - The channel's `measurement_units` entry, if any.
 * @returns The unit, whether the values are calibrated, a tag and an optional warning.
 */
export function describeChannelUnit(
    calibration: CalibrationSpec,
    measurementUnit: string | undefined | null
): ChannelUnitInfo {
    const configured = (measurementUnit ?? '').trim();
    if (calibration.type === 'identity') {
        const warning = configured && configured !== 'V'
            ? `The configured unit is ${configured}, but the channel has no calibration, so values are raw volts.`
            : undefined;
        return { unit: 'V', calibrated: false, tag: 'raw volts', warning };
    }

    const name = calibration.id?.trim() || calibration.type;
    const unit = configured || 'V';
    const warning = unit === 'V'
        ? `Calibration ${name} is applied but the channel's unit is V. If the calibration converts to another quantity, set the unit in the LabJack config.`
        : undefined;
    return { unit, calibrated: true, tag: `calibrated: ${name} → ${unit}`, warning };
}
