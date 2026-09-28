/**
 * Calibration formulas that convert raw LabJack volts into engineering units.
 *
 * Calibrations live in the `calibrations` map of a LabJack config in the KV bucket
 * `avenabox`, keyed by channel number. The archiver stores the active one in each
 * Parquet file's metadata, and the exporter applies it to the CSV
 * `calibrated_value` column (see `rust-ljm/src/calibration.rs`). The dashboard uses
 * this module to clean up edited calibrations and to preview calibrated values.
 *
 * @module
 */

/**
 * Calibration formula plus an optional name, tagged by `type`.
 *
 * Same JSON shape as the Rust `CalibrationSpec`, for example
 * `{"id":"tp3505","type":"linear","a":2.0,"b":-1.0}`.
 */
export type CalibrationSpec =
  /** Leaves raw values unchanged. */
  | {
      /** Optional name; the exporter writes it in the `calibration_id` CSV column. */
      id?: string;
      /** Formula tag. */
      type: "identity";
    }
  /** Applies `a * raw + b`. */
  | {
      /** Optional name; the exporter writes it in the `calibration_id` CSV column. */
      id?: string;
      /** Formula tag. */
      type: "linear";
      /** Slope, in output units per volt. */
      a: number;
      /** Offset, in output units. */
      b: number;
    }
  /** Applies `coeffs[i] * raw ** i` and sums each term. */
  | {
      /** Optional name; the exporter writes it in the `calibration_id` CSV column. */
      id?: string;
      /** Formula tag. */
      type: "polynomial";
      /** Coefficients in ascending power order; `coeffs[0]` is the constant term. */
      coeffs: number[];
    };

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
 * - Missing input or a non-string `type`: unnamed identity.
 * - `linear`: `a` and `b` are kept if they are finite numbers or numeric strings
 *   (stored back as numbers), otherwise `a` becomes 1 and `b` becomes 0.
 * - `polynomial`: coefficients that are finite numbers or numeric strings are kept as
 *   numbers and the rest dropped. An empty list stays empty and, as in Rust, evaluates
 *   to 0. A missing or non-array `coeffs` becomes `[0, 1]`, the identity polynomial.
 * - Any other `type`: identity.
 *
 * `id` is carried over in every case except the first. Returns a new object and does
 * not modify `raw`.
 *
 * @param raw - Calibration as read from config or form state, possibly partial.
 * @returns A spec that {@link applyCalibration} can evaluate.
 *
 * @example
 * ```ts
 * normalizeCalibration({ type: "linear", a: "2" });
 * // { id: undefined, type: "linear", a: 2, b: 0 }
 * normalizeCalibration({ type: "polynomial", coeffs: [] });
 * // { id: undefined, type: "polynomial", coeffs: [] }
 * ```
 */
export function normalizeCalibration(
  raw?: Partial<CalibrationSpec> | null
): CalibrationSpec {
  if (!raw || typeof raw.type !== "string") {
    return { type: "identity" };
  }

  if (raw.type === "linear") {
    return {
      id: raw.id,
      type: "linear",
      a: toFiniteNumber(raw.a) ?? 1,
      b: toFiniteNumber(raw.b) ?? 0,
    };
  }

  if (raw.type === "polynomial") {
    if (!Array.isArray(raw.coeffs)) {
      return { id: raw.id, type: "polynomial", coeffs: [0, 1] };
    }
    const coeffs = raw.coeffs
      .map(toFiniteNumber)
      .filter((value): value is number => value !== undefined);
    return { id: raw.id, type: "polynomial", coeffs };
  }

  return { id: raw.id, type: "identity" };
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
