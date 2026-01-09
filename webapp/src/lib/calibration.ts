export type CalibrationSpec =
  | {
      id?: string;
      type: "identity";
    }
  | {
      id?: string;
      type: "linear";
      a: number;
      b: number;
    }
  | {
      id?: string;
      type: "polynomial";
      coeffs: number[];
    };

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
