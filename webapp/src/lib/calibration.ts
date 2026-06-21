/** Calibration definition stored in dashboard config and Parquet metadata. */
export type CalibrationSpec =
  /** Leaves raw values unchanged. */
  | {
      id?: string;
      type: "identity";
    }
  /** Applies `a * raw + b`. */
  | {
      id?: string;
      type: "linear";
      a: number;
      b: number;
    }
  /** Applies `coeffs[i] * raw ** i` and sums each term. */
  | {
      id?: string;
      type: "polynomial";
      coeffs: number[];
    };

/**
 * Converts a partial or unknown calibration object into a valid spec.
 *
 * Missing, malformed, or unsupported calibration data becomes identity
 * calibration so UI previews and exports can proceed safely.
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

/** Applies a calibration formula to one raw sample value. */
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
