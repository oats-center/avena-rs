/**
 * Calibration formulas that convert raw LabJack volts into engineering units.
 *
 * Each channel has one calibration, stored inline in the `calibrations` map of its
 * LabJack config in the KV bucket `avenabox`, keyed by channel number. The calibration
 * carries the unit it converts to (`unit`). The archiver stores the active one in each
 * Parquet file's metadata, and the exporter applies it to the CSV `calibrated_value`
 * column (see `rust-ljm/src/calibration.rs`). The dashboard uses this module to clean
 * up edited calibrations, to preview calibrated values and to work out the unit of a
 * channel.
 *
 * @module
 */

/**
 * Calibration formula plus its output unit and an optional name, tagged by `type`.
 *
 * Same JSON shape as the Rust `CalibrationSpec`, for example
 * `{"type":"linear","a":2.0,"b":-1.0,"unit":"kPa"}`. Older configs may carry an `id`
 * (the name of a preset it was copied from) and no `unit`.
 */
export type CalibrationSpec =
  /** Leaves raw values unchanged; the unit is always volts. */
  | {
      /** Optional name from older configs; the exporter writes it in the `calibration_id` CSV column. */
      id?: string;
      /** Formula tag. */
      type: "identity";
      /** Unit of the output. Always `V` when written by the dashboard. */
      unit?: string;
    }
  /** Applies `a * raw + b`. */
  | {
      /** Optional name from older configs; the exporter writes it in the `calibration_id` CSV column. */
      id?: string;
      /** Formula tag. */
      type: "linear";
      /** Slope, in output units per volt. */
      a: number;
      /** Offset, in output units. */
      b: number;
      /** Unit the formula converts to, e.g. `kPa` or `µε`. */
      unit?: string;
    }
  /** Applies `coeffs[i] * raw ** i` and sums each term. */
  | {
      /** Optional name from older configs; the exporter writes it in the `calibration_id` CSV column. */
      id?: string;
      /** Formula tag. */
      type: "polynomial";
      /** Coefficients in ascending power order; `coeffs[0]` is the constant term. */
      coeffs: number[];
      /** Unit the formula converts to, e.g. `kPa` or `µε`. */
      unit?: string;
    };

/** Unit of raw LabJack readings, and of every identity calibration. */
export const RAW_UNIT = "V";

/** Unit of strain produced by {@link strainBridgeCalibration}. */
export const MICROSTRAIN = "µε";

/**
 * Reads a number the way a person would have typed it.
 *
 * @param value - Candidate value.
 * @returns The value if it is a finite number, the parsed value if it is a non-blank
 *   string holding a finite number (such as `"2.5"`), otherwise `undefined`.
 */
function toFiniteNumber(value: unknown): number | undefined {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : undefined;
  }
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}

/**
 * Converts a partial or unknown calibration object into a valid spec.
 *
 * The Rust `CalibrationSpec` accepts only finite JSON numbers and rejects the whole
 * config when a spec is malformed. This function repairs such input instead, keeping
 * as much of what was entered as possible, so that saving from the dashboard writes a
 * spec Rust can read. Valid specs come out unchanged. Rules, by `type`:
 *
 * - Missing input or a non-string `type`: identity.
 * - `linear`: `a` and `b` are kept if they are finite numbers or numeric strings
 *   (stored back as numbers), otherwise `a` becomes 1 and `b` becomes 0.
 * - `polynomial`: coefficients that are finite numbers or numeric strings are kept as
 *   numbers and the rest dropped. An empty list stays empty and, as in Rust, evaluates
 *   to 0. A missing or non-array `coeffs` becomes `[0, 1]`, the identity polynomial.
 * - Any other `type`: identity.
 *
 * A non-blank string `id` or `unit` is carried over (trimmed) in every case except the
 * first; blank or non-string values are left out, so the result has no `undefined`
 * fields. Returns a new object and does not modify `raw`.
 *
 * @param raw - Calibration as read from config or form state, possibly partial.
 * @returns A spec that {@link applyCalibration} can evaluate.
 *
 * @example
 * ```ts
 * normalizeCalibration({ type: "linear", a: "2", unit: "kPa" });
 * // { type: "linear", a: 2, b: 0, unit: "kPa" }
 * normalizeCalibration({ type: "polynomial", coeffs: [] });
 * // { type: "polynomial", coeffs: [] }
 * ```
 */
export function normalizeCalibration(
  raw?: Partial<CalibrationSpec> | null
): CalibrationSpec {
  if (!raw || typeof raw !== "object" || typeof raw.type !== "string") {
    return { type: "identity" };
  }

  const extras: { id?: string; unit?: string } = {};
  const id = cleanText(raw.id);
  if (id) extras.id = id;
  const unit = cleanText(raw.unit);
  if (unit) extras.unit = unit;

  if (raw.type === "linear") {
    return {
      ...extras,
      type: "linear",
      a: toFiniteNumber(raw.a) ?? 1,
      b: toFiniteNumber(raw.b) ?? 0,
    };
  }

  if (raw.type === "polynomial") {
    if (!Array.isArray(raw.coeffs)) {
      return { ...extras, type: "polynomial", coeffs: [0, 1] };
    }
    const coeffs = raw.coeffs
      .map(toFiniteNumber)
      .filter((value): value is number => value !== undefined);
    return { ...extras, type: "polynomial", coeffs };
  }

  return { ...extras, type: "identity" };
}

/**
 * Trims a value if it is a string.
 *
 * @param value - Candidate value.
 * @returns The trimmed string, or `undefined` when it is not a string or is blank.
 */
function cleanText(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed === "" ? undefined : trimmed;
}

/**
 * Works out the unit a channel's calibrated values are in.
 *
 * - Identity: always {@link RAW_UNIT}, whatever else is configured.
 * - Otherwise the calibration's own `unit` when set.
 * - Otherwise (configs written before calibrations had a unit) the channel's
 *   `measurement_units` entry, unless it is blank or `V`: `V` was the value filled in
 *   when nobody chose a unit, so it says nothing about a calibrated channel.
 *
 * @param calibration - Normalized calibration of the channel.
 * @param measurementUnit - The channel's `measurement_units` entry, if any.
 * @returns The unit, or `undefined` when a calibrated channel has no known unit.
 */
export function resolveCalibrationUnit(
  calibration: CalibrationSpec,
  measurementUnit?: string | null
): string | undefined {
  if (calibration.type === "identity") return RAW_UNIT;
  const own = cleanText(calibration.unit);
  if (own) return own;
  const legacy = cleanText(measurementUnit);
  return legacy && legacy !== RAW_UNIT ? legacy : undefined;
}

/** Inputs of {@link strainBridgeCalibration}, as printed on a strain gauge certificate and set on the amplifier. */
export interface StrainBridgeInputs {
  /** Calibration factor of the gauge, in µε per mV/V (e.g. 481.26). */
  factor: number;
  /** Bridge excitation voltage, V. */
  excitation: number;
  /** Amplifier gain between the bridge and the LabJack; may be negative. */
  gain: number;
  /** Raw LabJack reading at rest, V. Treated as 0 when omitted. */
  zero?: number;
}

/**
 * Builds the linear calibration that turns a strain gauge bridge's amplified output
 * voltage into microstrain.
 *
 * The bridge output is `raw / gain` volts, or `1000 * raw / (excitation * gain)` mV/V,
 * and the gauge certificate's factor converts mV/V to µε. Subtracting the reading at
 * rest gives:
 *
 * `µε = factor * 1000 * (raw - zero) / (excitation * gain)`
 *
 * so `a = factor * 1000 / (excitation * gain)` and `b = -a * zero`.
 *
 * @param inputs - Gauge factor, excitation, gain and optional zero reading.
 * @returns A linear calibration with unit {@link MICROSTRAIN}, or `null` when an input
 *   is not a finite number or `excitation * gain` is 0.
 *
 * @example
 * ```ts
 * strainBridgeCalibration({ factor: 481.26, excitation: 10, gain: 100, zero: 0.5 });
 * // { type: "linear", a: 481.26, b: -240.63, unit: "µε" }
 * ```
 */
export function strainBridgeCalibration(inputs: StrainBridgeInputs): CalibrationSpec | null {
  const { factor, excitation, gain } = inputs;
  const zero = inputs.zero ?? 0;
  if (![factor, excitation, gain, zero].every(Number.isFinite)) return null;
  const scale = excitation * gain;
  if (scale === 0) return null;
  const a = (factor * 1000) / scale;
  // `-a * 0` would be -0; keep a plain 0 offset when there is no zero reading.
  const b = zero === 0 ? 0 : -a * zero;
  return { type: "linear", a, b, unit: MICROSTRAIN };
}

/**
 * Writes a calibration as a formula in `x`, the raw reading in volts.
 *
 * @param spec - Normalized calibration.
 * @returns For example `y = x`, `y = 481.26·x + 1058.722` or `y = 1 − 2·x + 0.5·x²`.
 */
export function formatCalibration(spec: CalibrationSpec): string {
  if (spec.type === "linear") {
    return `y = ${spec.a}·x ${spec.b < 0 ? "−" : "+"} ${Math.abs(spec.b)}`;
  }
  if (spec.type === "polynomial") {
    if (spec.coeffs.length === 0) return "y = 0";
    const power = (i: number) => (i === 0 ? "" : i === 1 ? "·x" : `·x${superscript(i)}`);
    return spec.coeffs
      .map((c, i) =>
        i === 0 ? `y = ${c}${power(i)}` : ` ${c < 0 ? "−" : "+"} ${Math.abs(c)}${power(i)}`
      )
      .join("");
  }
  return "y = x";
}

/**
 * Writes a non-negative integer with Unicode superscript digits.
 *
 * @param n - Exponent.
 * @returns For example `²` for 2 or `¹⁰` for 10.
 */
function superscript(n: number): string {
  const digits = "⁰¹²³⁴⁵⁶⁷⁸⁹";
  return String(n).replace(/\d/g, (d) => digits[Number(d)]);
}

/**
 * Applies a calibration formula to one raw sample value, with the same arithmetic as
 * the Rust `CalibrationSpec::apply` that fills the exported `calibrated_value` column.
 *
 * Non-finite input is not special-cased, again as in Rust: `NaN` stays `NaN` through
 * identity, linear and non-empty polynomial formulas, while an empty polynomial gives
 * -0 for any input. The spec is used as given; pass it through
 * {@link normalizeCalibration} first if it may be incomplete.
 *
 * @param spec - Calibration to apply.
 * @param raw - Raw reading, in volts.
 * @returns The calibrated value. An empty polynomial evaluates to -0.
 *
 * @example
 * ```ts
 * applyCalibration({ type: "linear", a: 2, b: -1 }, 3);          // 5
 * applyCalibration({ type: "polynomial", coeffs: [1, 0, 2] }, 3); // 1 + 2 * 9 = 19
 * ```
 */
export function applyCalibration(spec: CalibrationSpec, raw: number): number {
  if (spec.type === "linear") {
    return spec.a * raw + spec.b;
  }

  if (spec.type === "polynomial") {
    // Rust's float `sum()` starts from -0.0, so an empty polynomial gives -0.
    return spec.coeffs.reduce((acc, coeff, idx) => acc + coeff * raw ** idx, -0);
  }

  return raw;
}
