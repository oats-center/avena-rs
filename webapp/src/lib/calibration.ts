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
 * Converts a partial or unknown calibration object into a valid spec.
 *
 * Rules, by `type`:
 *
 * - Missing input or a non-string `type`: unnamed identity.
 * - `linear`: `a` and `b` are kept if they are finite numbers, otherwise `a`
 *   becomes 1 and `b` becomes 0. Numeric strings are not accepted.
 * - `polynomial`: each coefficient is converted with `Number()` and non-finite
 *   results are dropped. An empty result becomes `[0, 1]`, the identity polynomial.
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
 * normalizeCalibration({ type: "linear", a: 2 });
 * // { id: undefined, type: "linear", a: 2, b: 0 }
 * normalizeCalibration({ type: "polynomial", coeffs: [] });
 * // { id: undefined, type: "polynomial", coeffs: [0, 1] }
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
      a: Number.isFinite(raw.a as number) ? Number(raw.a) : 1,
      b: Number.isFinite(raw.b as number) ? Number(raw.b) : 0,
    };
  }

  if (raw.type === "polynomial") {
    const coeffs = Array.isArray(raw.coeffs)
      ? raw.coeffs.map((value) => Number(value)).filter((value) => Number.isFinite(value))
      : [];
    return {
      id: raw.id,
      type: "polynomial",
      coeffs: coeffs.length > 0 ? coeffs : [0, 1],
    };
  }

  return { id: raw.id, type: "identity" };
}

/**
 * Applies a calibration formula to one raw sample value.
 *
 * A non-finite `raw` (`NaN` for a skipped sample, or an infinity) is returned
 * unchanged without evaluating the formula. The spec is used as given; pass it
 * through {@link normalizeCalibration} first if it may be incomplete.
 *
 * @param spec - Calibration to apply.
 * @param raw - Raw reading, in volts.
 * @returns The calibrated value. An empty polynomial evaluates to 0.
 *
 * @example
 * ```ts
 * applyCalibration({ type: "linear", a: 2, b: -1 }, 3);          // 5
 * applyCalibration({ type: "polynomial", coeffs: [1, 0, 2] }, 3); // 1 + 2 * 9 = 19
 * ```
 */
export function applyCalibration(spec: CalibrationSpec, raw: number): number {
  if (!Number.isFinite(raw)) {
    return raw;
  }

  if (spec.type === "linear") {
    return spec.a * raw + spec.b;
  }

  if (spec.type === "polynomial") {
    return spec.coeffs.reduce((acc, coeff, idx) => acc + coeff * raw ** idx, 0);
  }

  return raw;
}
