/**
 * Pure helpers for drawing a channel trace: value range, visible window, min/max
 * downsampling and splitting at gaps.
 *
 * A `NaN` value marks a missing sample or a gap in the stream (see `stream.ts`). It
 * is never drawn and never bridged: the trace is split into separate segments there.
 *
 * @module
 */
import { lowerBound, upperBound, type DataPoint } from './stream';

/**
 * Returns the smallest and largest finite value in a list of points.
 *
 * @param points - Samples to scan. Non-finite values are skipped.
 * @returns `{ min, max }`, or `null` if there are no finite values.
 */
export function computeValueRange(points: DataPoint[]): { min: number; max: number } | null {
    let min = Number.POSITIVE_INFINITY;
    let max = Number.NEGATIVE_INFINITY;
    for (const point of points) {
        const value = point.value;
        if (!Number.isFinite(value)) continue;
        if (value < min) min = value;
        if (value > max) max = value;
    }
    if (min > max) return null;
    return { min, max };
}

/**
 * Returns the points with `start <= t <= end`, optionally with one neighbor on each
 * side so a line can run to the edge of the plot (the canvas clips it there).
 *
 * @param data - Points sorted by time.
 * @param start - Window start, Unix ms.
 * @param end - Window end, Unix ms.
 * @param withNeighbors - Also keep the closest point outside each edge.
 * @returns A new array.
 */
export function selectTimeWindow(
    data: DataPoint[],
    start: number,
    end: number,
    withNeighbors: boolean = false
): DataPoint[] {
    let from = lowerBound(data, start);
    let to = upperBound(data, end);
    if (withNeighbors) {
        from = Math.max(0, from - 1);
        to = Math.min(data.length, to + 1);
    }
    return data.slice(from, to);
}

/**
 * Reduces a sorted series to at most two points per bucket (the bucket's minimum and
 * maximum, in time order), keeping every gap.
 *
 * Runs of present values are reduced separately; a `NaN` point is kept between runs,
 * so a gap survives downsampling. Series that already fit are returned unchanged.
 *
 * @param data - Points sorted by time.
 * @param bucketCount - Number of buckets, usually the plot width in pixels.
 * @returns The reduced series, or `data` itself.
 */
export function downsampleMinMax(data: DataPoint[], bucketCount: number): DataPoint[] {
    const buckets = Math.max(1, Math.floor(bucketCount));
    if (data.length <= buckets * 2) return data;

    const bucketSize = Math.ceil(data.length / buckets);
    const reduced: DataPoint[] = [];

    for (let start = 0; start < data.length; start += bucketSize) {
        const end = Math.min(data.length, start + bucketSize);
        let minPoint: DataPoint | undefined;
        let maxPoint: DataPoint | undefined;

        for (let i = start; i < end; i++) {
            const point = data[i];
            if (!Number.isFinite(point.value)) {
                pushMinMax(reduced, minPoint, maxPoint);
                minPoint = undefined;
                maxPoint = undefined;
                const last = reduced[reduced.length - 1];
                if (!last || Number.isFinite(last.value)) reduced.push(point);
                continue;
            }
            if (minPoint === undefined || point.value < minPoint.value) minPoint = point;
            if (maxPoint === undefined || point.value > maxPoint.value) maxPoint = point;
        }
        pushMinMax(reduced, minPoint, maxPoint);
    }

    return reduced;
}

/** Appends a bucket's minimum and maximum in time order (one point if they coincide). */
function pushMinMax(out: DataPoint[], minPoint: DataPoint | undefined, maxPoint: DataPoint | undefined): void {
    if (!minPoint || !maxPoint) return;
    if (minPoint === maxPoint) out.push(minPoint);
    else if (minPoint.timestamp <= maxPoint.timestamp) out.push(minPoint, maxPoint);
    else out.push(maxPoint, minPoint);
}

/**
 * Splits a sorted series into runs of present values, dropping the `NaN` points that
 * separate them. Each run is drawn as one connected line.
 *
 * @param data - Points sorted by time.
 * @returns Runs, each with at least one point.
 */
export function splitAtGaps(data: DataPoint[]): DataPoint[][] {
    const segments: DataPoint[][] = [];
    let current: DataPoint[] = [];
    for (const point of data) {
        if (!Number.isFinite(point.value) || !Number.isFinite(point.timestamp)) {
            if (current.length > 0) segments.push(current);
            current = [];
            continue;
        }
        current.push(point);
    }
    if (current.length > 0) segments.push(current);
    return segments;
}

/**
 * Returns the newest point with a present value.
 *
 * @param data - Points sorted by time.
 * @returns The point, or `null` if none has a finite value.
 */
export function latestFinitePoint(data: DataPoint[]): DataPoint | null {
    for (let i = data.length - 1; i >= 0; i--) {
        if (Number.isFinite(data[i].value)) return data[i];
    }
    return null;
}

/**
 * Rounds a raw step up to 1, 2 or 5 times a power of ten.
 *
 * @param raw - Raw step, above 0.
 * @returns The rounded step, or 1 for non-finite or non-positive input.
 */
export function niceStepUp(raw: number): number {
    if (!Number.isFinite(raw) || raw <= 0) return 1;
    const exponent = Math.floor(Math.log10(raw));
    const base = Math.pow(10, exponent);
    const fraction = raw / base;
    if (fraction <= 1) return base;
    if (fraction <= 2) return 2 * base;
    if (fraction <= 5) return 5 * base;
    return 10 * base;
}

/** Tick positions and labels for a time axis, from {@link timeAxisTicks}. */
export interface TimeAxisTicks {
    /** Distance between ticks, seconds. */
    step: number;
    /** Tick values in seconds, ascending. Always multiples of `step`, so 0 is a tick when in range. */
    values: number[];
    /** `ms` when the step is below 0.1 s, else `s`. Used for every label on the axis. */
    unit: 'ms' | 's';
}

/**
 * Picks round time ticks for the range `[start, end]` seconds.
 *
 * The step is 1, 2 or 5 times a power of ten, chosen so that at most `maxTicks`
 * ticks fit. Ticks are multiples of the step, so they are anchored at 0 (the newest
 * sample in continuous mode, the trigger in frozen mode) and evenly spaced.
 *
 * @param start - Range start, seconds.
 * @param end - Range end, seconds. Swapped with `start` if smaller.
 * @param maxTicks - Most ticks wanted, at least 2.
 * @returns The ticks. Empty values for an empty or non-finite range.
 */
export function timeAxisTicks(start: number, end: number, maxTicks: number): TimeAxisTicks {
    const low = Math.min(start, end);
    const high = Math.max(start, end);
    const span = high - low;
    if (!Number.isFinite(span) || span <= 0) return { step: 1, values: [], unit: 's' };

    const intervals = Math.max(1, Math.floor(maxTicks) - 1);
    let step = niceStepUp(span / intervals);
    // Floating point can leave one tick too many; widen once if so.
    const count = (s: number) => Math.floor(high / s + 1e-9) - Math.ceil(low / s - 1e-9) + 1;
    if (count(step) > Math.max(2, Math.floor(maxTicks))) step = niceStepUp(step * 1.5);

    const values: number[] = [];
    const first = Math.ceil(low / step - 1e-9);
    const last = Math.floor(high / step + 1e-9);
    for (let i = first; i <= last; i++) {
        // `i * step` avoids accumulating error; `+ 0` turns -0 into 0.
        values.push(Number((i * step).toPrecision(12)) + 0);
    }
    return { step, values, unit: step < 0.1 ? 'ms' : 's' };
}

/**
 * Formats a time tick with just enough decimals for its step, never as `-0`.
 *
 * @param value - Tick value, seconds.
 * @param step - Tick step from {@link timeAxisTicks}, seconds.
 * @param unit - Label unit from {@link timeAxisTicks}.
 * @returns The number without unit, e.g. `-500` (ms) or `1.5` (s).
 */
export function formatTimeTick(value: number, step: number, unit: 'ms' | 's'): string {
    const scaled = unit === 'ms' ? value * 1000 : value;
    const scaledStep = unit === 'ms' ? step * 1000 : step;
    const decimals = scaledStep >= 1 ? 0 : Math.min(6, Math.ceil(-Math.log10(scaledStep) - 1e-9));
    const text = scaled.toFixed(decimals);
    return Number(text) === 0 ? (0).toFixed(decimals) : text;
}

/**
 * Picks time ticks for an axis `widthPx` wide.
 *
 * Starts from about one tick per `preferredSpacingPx`. When that gives fewer than
 * `minTicks` ticks (a narrow plot, where the 1-2-5 rounding can leave only the two
 * ends), it tries denser steps and keeps the densest one whose labels still fit
 * between neighbouring ticks with `gapPx` to spare, stopping once `minTicks` is
 * reached. Labels never overlap: a denser step is used only if its widest label fits.
 *
 * @param start - Range start, seconds.
 * @param end - Range end, seconds.
 * @param widthPx - Width of the axis, CSS pixels.
 * @param measure - Width of a label in CSS pixels, in the tick font.
 * @param options - `preferredSpacingPx` (default 70), `minTicks` (default 4) and
 *   `gapPx`, the least space between two labels (default 12).
 * @returns The ticks, as from {@link timeAxisTicks}.
 */
export function fitTimeTicks(
    start: number,
    end: number,
    widthPx: number,
    measure: (label: string) => number,
    options: { preferredSpacingPx?: number; minTicks?: number; gapPx?: number } = {}
): TimeAxisTicks {
    const preferred = options.preferredSpacingPx ?? 70;
    const minTicks = options.minTicks ?? 4;
    const gap = options.gapPx ?? 12;
    const width = Math.max(0, widthPx);
    const baseMax = Math.max(2, Math.floor(width / preferred) + 1);
    let best = timeAxisTicks(start, end, baseMax);
    const span = Math.abs(end - start);
    if (best.values.length >= minTicks || !(span > 0) || width <= 0) return best;

    const fits = (axis: TimeAxisTicks) => {
        const spacing = (width * axis.step) / span;
        const widest = Math.max(0, ...axis.values.map((v) => measure(formatTimeTick(v, axis.step, axis.unit))));
        return widest + gap <= spacing;
    };
    for (let maxTicks = baseMax + 1; maxTicks <= baseMax + 10; maxTicks++) {
        const candidate = timeAxisTicks(start, end, maxTicks);
        if (candidate.values.length <= best.values.length) continue;
        if (!fits(candidate)) break;
        best = candidate;
        if (best.values.length >= minTicks) break;
    }
    return best;
}

/** Box of a label drawn on the canvas, CSS pixels. */
export interface LabelBox {
    x: number;
    y: number;
    width: number;
    height: number;
}

/**
 * Places the trigger level label in the top margin, above the plot area, so it never
 * covers the trace.
 *
 * The box starts at the left edge of the plot area and is centred vertically in the
 * top margin. It is never wider than the plot area and never taller than the margin;
 * text that does not fit is clipped by the caller.
 *
 * @param plotLeft - Left edge of the plot area.
 * @param plotRight - Right edge of the plot area.
 * @param marginTop - Height of the top margin (the plot area starts here).
 * @param textWidth - Measured width of the label text.
 * @param padding - Horizontal space between the text and the box edges.
 * @param height - Preferred box height.
 * @returns The box, with `width` and `height` of at least 0.
 */
export function thresholdLabelBox(
    plotLeft: number,
    plotRight: number,
    marginTop: number,
    textWidth: number,
    padding: number = 6,
    height: number = 18
): LabelBox {
    const available = Math.max(0, plotRight - plotLeft);
    const boxHeight = Math.max(0, Math.min(height, marginTop - 2));
    return {
        x: plotLeft,
        y: Math.max(0, (marginTop - boxHeight) / 2),
        width: Math.min(available, Math.max(0, textWidth) + 2 * padding),
        height: boxHeight
    };
}
