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
