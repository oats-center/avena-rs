/**
 * Pure helpers for the plot page's axis inputs.
 *
 * Pure TypeScript with no Svelte dependency, so it is unit tested directly.
 *
 * @module
 */

/** The axis limits a user can type on the plot page. */
export interface AxisLimits {
    /** Lower Y limit used when autoscale is off. */
    yMin: number;
    /** Upper Y limit used when autoscale is off. */
    yMax: number;
    /** Width of the X axis in seconds. */
    xWindowSec: number;
}

/**
 * Reads a number typed into an `<input type="number">`.
 *
 * Unlike `parseFloat(v) || fallback`, 0 is a valid value.
 *
 * @param text - Input text.
 * @returns The number, or `null` when the text is blank or not a finite number.
 */
export function parseFiniteInput(text: string): number | null {
    const trimmed = text.trim();
    if (trimmed === "") return null;
    const value = Number(trimmed);
    return Number.isFinite(value) ? value : null;
}

/** Result of {@link applyAxisLimitInput}. */
export type AxisLimitResult =
    | { ok: true; limits: AxisLimits }
    | { ok: false; error: string };

/**
 * Checks a typed axis limit against the other limits.
 *
 * Any finite Y value is accepted, including 0, as long as Y Min stays below Y Max.
 * The X window must be above 0.
 *
 * @param current - Limits in use.
 * @param field - Limit being edited.
 * @param text - Text typed in the input.
 * @returns The new limits, or an error message; on error the caller keeps `current`
 *   and shows its value in the input again.
 */
export function applyAxisLimitInput(
    current: AxisLimits,
    field: keyof AxisLimits,
    text: string
): AxisLimitResult {
    const value = parseFiniteInput(text);
    if (value === null) {
        return { ok: false, error: "Enter a number." };
    }
    const next = { ...current, [field]: value };
    if (field === "xWindowSec" && !(value > 0)) {
        return { ok: false, error: "X Window must be greater than 0 s." };
    }
    if ((field === "yMin" || field === "yMax") && !(next.yMin < next.yMax)) {
        return { ok: false, error: "Y Min must be less than Y Max." };
    }
    return { ok: true, limits: next };
}

/**
 * Picks the manual Y limits used when autoscale is turned off, from the range the plot
 * showed last, so the plot does not jump.
 *
 * The edges are rounded outward to two significant digits beyond the span's order of
 * magnitude (at most six decimals), so the range still covers what was shown and the
 * inputs show short numbers.
 *
 * @param shown - Y range of the last drawn frame, or `null` when nothing was drawn.
 * @param current - Limits in use; kept when there is no usable range.
 * @returns The new `yMin` and `yMax`, with `yMin < yMax`.
 *
 * @example
 * ```ts
 * seedManualYLimits({ low: -2, high: 2 }, { yMin: -1, yMax: 1 });              // { yMin: -2, yMax: 2 }
 * seedManualYLimits({ low: 0.12345, high: 0.30001 }, { yMin: -1, yMax: 1 });   // { yMin: 0.123, yMax: 0.301 }
 * ```
 */
export function seedManualYLimits(
    shown: { low: number; high: number } | null,
    current: { yMin: number; yMax: number }
): { yMin: number; yMax: number } {
    if (!shown || !Number.isFinite(shown.low) || !Number.isFinite(shown.high) || !(shown.low < shown.high)) {
        return { yMin: current.yMin, yMax: current.yMax };
    }
    const span = shown.high - shown.low;
    const decimals = Math.min(6, Math.max(0, Math.ceil(-Math.log10(span)) + 2));
    const factor = Math.pow(10, decimals);
    // Round away from the range, but ignore floating point noise just past a step.
    const yMin = Number((Math.floor(shown.low * factor + 1e-9) / factor).toFixed(decimals));
    const yMax = Number((Math.ceil(shown.high * factor - 1e-9) / factor).toFixed(decimals));
    return yMin < yMax ? { yMin, yMax } : { yMin: current.yMin, yMax: current.yMax };
}
