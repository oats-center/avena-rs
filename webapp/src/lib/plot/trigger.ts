/**
 * Trigger detection and capture for the live plot.
 *
 * A capture holds exactly the buffered samples with `triggerTime - pre <= t <=
 * triggerTime + post`. It starts from the live buffer when the trigger fires and grows
 * with each later batch until a sample at or past `triggerTime + post` has arrived;
 * then it is complete and nothing more is added. Samples are sorted by time (see
 * `stream.ts`), so the window is found by binary search.
 *
 * `NaN` points (missing samples and gap markers) never fire a trigger, and a crossing
 * is judged against the last present sample before it.
 *
 * @module
 */
import { lowerBound, upperBound, type DataPoint } from './stream';

/** Trigger settings of one channel. */
export interface TriggerSettings {
    /** Edge that fires the trigger. */
    type: 'rising' | 'falling';
    /** Level compared with calibrated values, in the channel's unit. */
    threshold: number;
    /** Share of the capture window before the trigger, percent, 0 to 95. */
    preTriggerPercent: number;
    /** Length of the capture after the trigger, seconds. */
    postTriggerWindowSec: number;
}

/** One trigger capture. */
export interface TriggerCapture {
    /** Samples in `[startTime, endTime]`, sorted by time. */
    data: DataPoint[];
    /** Time of the first sample past the threshold, Unix ms. */
    triggerTime: number;
    /** `triggerTime - preWindowSec`, Unix ms. */
    startTime: number;
    /** `triggerTime + postWindowSec`, Unix ms. */
    endTime: number;
    /** Pre-trigger window, seconds, fixed when the trigger fired. */
    preWindowSec: number;
    /** Post-trigger window, seconds, fixed when the trigger fired. */
    postWindowSec: number;
    /** True once a sample at or after `endTime` has been seen. */
    complete: boolean;
}

/**
 * Converts trigger settings into pre and post window lengths.
 *
 * The post window is at least 0.01 s. The pre-trigger percent is clamped to 0 to 95 and
 * taken as a share of the whole window, so `pre = post * p / (1 - p)`.
 *
 * @param settings - Trigger settings, or `undefined` for the minimums.
 * @returns `preWindowSec` and `postWindowSec`, seconds.
 */
export function getTriggerWindows(settings: TriggerSettings | undefined) {
    const postWindowSec = Math.max(0.01, settings?.postTriggerWindowSec || 0.01);
    const preFraction = Math.min(0.95, Math.max(0, (settings?.preTriggerPercent || 0) / 100));
    const preWindowSec = postWindowSec * (preFraction / (1 - preFraction));
    return { preWindowSec, postWindowSec };
}

/**
 * Tells whether a buffer spans the pre-trigger window.
 *
 * @param data - Points sorted by time.
 * @param settings - Trigger settings.
 * @returns `true` when the newest point is at least `preWindowSec` after the oldest.
 */
export function hasRequiredPreBuffer(data: DataPoint[], settings: TriggerSettings): boolean {
    if (data.length < 2) return false;
    const { preWindowSec } = getTriggerWindows(settings);
    return data[data.length - 1].timestamp - data[0].timestamp >= preWindowSec * 1000;
}

/**
 * Finds the first threshold crossing at or after `fromIndex`.
 *
 * Rising fires when the previous present value is at or below the threshold and the
 * current one is above it; falling is the mirror. A crossing counts only if it is after
 * `notBefore` and the buffer already holds the full pre-trigger window before it.
 *
 * @param buffer - Live buffer, sorted by time.
 * @param fromIndex - First index that may fire.
 * @param settings - Trigger settings.
 * @param notBefore - Crossings at or before this time (Unix ms) are ignored.
 * @returns Index of the crossing sample, or -1.
 */
export function findTriggerCrossing(
    buffer: DataPoint[],
    fromIndex: number,
    settings: TriggerSettings,
    notBefore: number = Number.NEGATIVE_INFINITY
): number {
    if (buffer.length === 0) return -1;
    const threshold = settings.threshold;
    const preMs = getTriggerWindows(settings).preWindowSec * 1000;
    const oldest = buffer[0].timestamp;

    let previous: number | null = null;
    for (let i = Math.max(0, fromIndex) - 1; i >= 0; i--) {
        if (Number.isFinite(buffer[i].value)) {
            previous = buffer[i].value;
            break;
        }
    }

    for (let i = Math.max(0, fromIndex); i < buffer.length; i++) {
        const point = buffer[i];
        if (!Number.isFinite(point.value)) continue;
        if (previous !== null && point.timestamp > notBefore && point.timestamp - preMs >= oldest) {
            const crossed =
                settings.type === 'rising'
                    ? previous <= threshold && point.value > threshold
                    : previous >= threshold && point.value < threshold;
            if (crossed) return i;
        }
        previous = point.value;
    }
    return -1;
}

/**
 * Starts a capture around a trigger from the live buffer.
 *
 * @param buffer - Live buffer, sorted by time.
 * @param triggerTime - Time of the crossing sample, Unix ms.
 * @param windows - Pre and post windows, seconds.
 * @returns The capture, complete if the buffer already reaches the window end.
 */
export function startCapture(
    buffer: DataPoint[],
    triggerTime: number,
    windows: { preWindowSec: number; postWindowSec: number }
): TriggerCapture {
    const startTime = triggerTime - windows.preWindowSec * 1000;
    const endTime = triggerTime + windows.postWindowSec * 1000;
    const data = buffer.slice(lowerBound(buffer, startTime), upperBound(buffer, endTime));
    const newest = buffer.length > 0 ? buffer[buffer.length - 1].timestamp : triggerTime;
    return {
        data,
        triggerTime,
        startTime,
        endTime,
        preWindowSec: windows.preWindowSec,
        postWindowSec: windows.postWindowSec,
        complete: newest >= endTime
    };
}

/**
 * Adds a new batch to an open capture.
 *
 * Only points newer than the capture's last point and not past `endTime` are added. The
 * capture is complete once any point at or past `endTime` has been seen.
 *
 * @param capture - Open capture.
 * @param chunk - Points just appended to the live buffer, sorted by time.
 * @returns A new capture object, or `capture` itself when nothing changed.
 */
export function extendCapture(capture: TriggerCapture, chunk: DataPoint[]): TriggerCapture {
    if (capture.complete || chunk.length === 0) return capture;
    const fromIndex =
        capture.data.length > 0
            ? upperBound(chunk, capture.data[capture.data.length - 1].timestamp)
            : lowerBound(chunk, capture.startTime);
    const to = upperBound(chunk, capture.endTime);
    const appended = fromIndex < to ? chunk.slice(fromIndex, to) : [];
    const complete = chunk[chunk.length - 1].timestamp >= capture.endTime;
    if (appended.length === 0 && !complete) return capture;
    return {
        ...capture,
        data: appended.length > 0 ? capture.data.concat(appended) : capture.data,
        complete
    };
}

/**
 * Advances one channel's trigger after a new batch was appended to its live buffer.
 *
 * An open capture takes the batch's samples up to its window end; if the buffer was
 * reset (the timeline jumped back) it is closed as it is. A capture is held while it is
 * collecting, and in single-shot mode until the user re-arms. Otherwise the new samples
 * are searched for a crossing after the end of the previous capture (with the full
 * pre-trigger window buffered), and a crossing starts a new capture from the buffer. In
 * normal mode a capture that completes within the same batch is followed by a search
 * for the next crossing, so the last one found is returned.
 *
 * @param capture - Current capture, or `null` when none was taken since arming.
 * @param singleShot - `true` for single-shot mode, `false` for normal mode.
 * @param settings - Trigger settings.
 * @param buffer - Live buffer, sorted by time, already including the batch.
 * @param chunkStartIndex - Index in `buffer` of the batch's first point.
 * @param reset - The buffer was emptied before this batch.
 * @param chunk - Points just appended.
 * @returns The capture to show: `capture` itself when unchanged, a new object when it
 *   changed, or `null` when there is none.
 */
export function advanceTrigger(
    capture: TriggerCapture | null,
    singleShot: boolean,
    settings: TriggerSettings,
    buffer: DataPoint[],
    chunkStartIndex: number,
    reset: boolean,
    chunk: DataPoint[]
): TriggerCapture | null {
    let current = capture;
    if (current && !current.complete) {
        current = reset ? { ...current, complete: true } : extendCapture(current, chunk);
    }

    const holding = current !== null && (!current.complete || singleShot);
    if (holding) return current;

    const windows = getTriggerWindows(settings);
    let from = chunkStartIndex;
    let notBefore = current && !reset ? current.endTime : Number.NEGATIVE_INFINITY;
    while (from < buffer.length) {
        const index = findTriggerCrossing(buffer, from, settings, notBefore);
        if (index < 0) break;
        current = startCapture(buffer, buffer[index].timestamp, windows);
        if (!current.complete || singleShot) break;
        notBefore = current.endTime;
        from = index + 1;
    }
    return current;
}
