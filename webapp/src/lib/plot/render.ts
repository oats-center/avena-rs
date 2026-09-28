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
