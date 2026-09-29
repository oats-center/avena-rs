/**
 * Parsing of the plot page's route parameters.
 *
 * @module
 */

/**
 * Parses the `asset_number` route parameter.
 *
 * Only plain decimal digits are accepted, so `"12abc"`, `"-3"`, `"1.5"` and `""` are
 * rejected instead of being read as a different number by `parseInt`.
 *
 * @param raw - Parameter text from the URL.
 * @returns The asset number, or `null` when the text is not a non-negative integer.
 */
export function parseAssetNumberParam(raw: string | undefined | null): number | null {
    const text = (raw ?? '').trim();
    if (!/^\d+$/.test(text)) return null;
    const value = Number(text);
    return Number.isSafeInteger(value) ? value : null;
}
