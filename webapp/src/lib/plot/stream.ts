/**
 * Live-plot data path: from queued NATS `Scan` payloads to a sorted, de-duplicated,
 * calibrated sample buffer per channel.
 *
 * The streamer publishes one FlatBuffer `Scan` per channel per LabJack read, with the
 * time of the first value, the interval between values, and a batch sequence number
 * that restarts at 0 for each sampling run. Within a run, batches are contiguous: the
 * next batch starts one interval after the previous batch's last sample, give or take
 * a slew of at most half an interval, so timestamps are strictly increasing.
 *
 * This module keeps the plot faithful to that stream:
 *
 * - {@link ScanMessageQueue} holds every received payload per channel until the next
 *   UI tick, in arrival order. It is bounded; when full it drops the oldest message and
 *   counts it, so an overload is visible instead of silent.
 * - {@link ingestScan} turns one decoded scan into points. Each sample gets the time
 *   `first + i * interval`, computed in integer nanoseconds. A `NaN` or LJM dummy
 *   (`-9999`) sample becomes a `NaN` point, which the renderer draws as a gap. Missing
 *   time between batches (lost or dropped messages, reconnects) gets a `NaN` gap marker
 *   so the line is not bridged. Duplicate or late batches are skipped, and a timeline
 *   that jumps backward (clock step, new run) resets the buffer instead of mixing two
 *   timelines.
 * - {@link drainChannelQueue} runs the above for every queued message of one channel,
 *   in order, and appends to the channel's buffer.
 *
 * Pure TypeScript with no Svelte or NATS dependency, so it is unit tested directly.
 *
 * @module
 */
import { applyCalibration, type CalibrationSpec } from '../calibration';
import type { ScanData } from '../flatbuffer-parser';

/** One sample as stored in the live buffers and passed to `RealTimePlot`. */
export interface DataPoint {
    /** Sample time from the `Scan` (first sample time plus `i` intervals), Unix ms. */
    timestamp: number;
    /** Calibrated value, or `NaN` for a missing sample or a gap marker. */
    value: number;
    /** Source sample time, Unix ms. Equal to `timestamp`. */
    sourceTimestamp?: number | null;
    /** Browser time the message arrived, Unix ms. Used for the lag readout. */
    receivedAt?: number;
}

/** Value LJM writes into scans it could not fill after a buffer overflow. */
export const LJM_DUMMY_VALUE = -9999;

/**
 * Reports whether a raw reading stands for "no sample".
 *
 * The current streamer already turns LJM's `-9999` dummies into `NaN`; older streamers
 * published `-9999` as is, so both are treated as missing here.
 *
 * @param raw - Raw reading from the `Scan`.
 * @returns `true` for non-finite values and for exactly `-9999`.
 */
export function isMissingRawValue(raw: number): boolean {
    return !Number.isFinite(raw) || raw === LJM_DUMMY_VALUE;
}

/** What the channel's stream looked like after the last accepted batch. */
export interface ChannelStreamState {
    /** Time of the newest sample in the buffer, Unix ms. */
    lastTimestamp: number;
    /** Time of the first sample of the last accepted batch, Unix ms. */
    lastFirstMs: number;
    /** Sequence number of the last accepted batch. */
    lastSequence: bigint;
    /** Sample interval of the last accepted batch, ms. */
    lastIntervalMs: number;
    /** Time covered by the last accepted batch (`values.length * interval`), ms. */
    lastBatchSpanMs: number;
}

/** Result of {@link ingestScan}. */
export interface IngestResult {
    /**
     * Points to append, oldest first. Starts with a `NaN` gap marker when time is
     * missing before this batch.
     */
    points: DataPoint[];
    /** The caller must empty the channel's buffer before appending `points`. */
    reset: boolean;
    /** A gap marker was added because samples are missing before this batch. */
    gap: boolean;
    /** Samples skipped because they were already in the buffer (duplicate or late). */
    skippedSamples: number;
    /** Updated stream state to keep for the next batch. */
    state: ChannelStreamState | null;
}

/**
 * Largest difference, in ms, between a batch's first-sample time and the time its
 * sequence number implies on the previous run's timeline, for the batch to be taken as
 * a late or repeated batch of that run. Slewing moves a run's timeline by at most 1 ms
 * per minute, so anything further off belongs to a different timeline.
 */
const SAME_RUN_TOLERANCE_MS = 250;

/**
 * Gap threshold, in sample intervals. Consecutive batches within a run are one interval
 * apart, plus or minus a slew of at most half an interval, so up to 1.5 intervals is
 * normal. One missing sample makes it 2. The threshold sits between the two so that
 * rounding at the slew limit never draws a false gap.
 */
const GAP_THRESHOLD_INTERVALS = 1.75;

/**
 * Converts one decoded scan into plot points and decides how it joins the buffer.
 *
 * Rules, against the previous accepted batch (`state`):
 *
 * - First batch, or the batch starts after the newest buffered sample: append. If it
 *   starts more than 1.5 intervals after it, a `NaN` gap marker goes first.
 * - The batch starts at or before the newest buffered sample and its sequence and time
 *   fit the previous run's timeline (a repeated or late batch): samples already covered
 *   are skipped, newer ones (if any) appended.
 * - Otherwise the timeline moved backward (a clock step, or a new run whose clock is
 *   behind): `reset` is set and the batch starts a new buffer.
 *
 * @param state - State after the previous accepted batch, or `null` for none.
 * @param scan - Decoded `Scan`.
 * @param calibration - Calibration of the channel, applied to every present sample.
 * @param receivedAt - Browser time the message arrived, Unix ms.
 * @returns Points, flags and the new state.
 */
export function ingestScan(
    state: ChannelStreamState | null,
    scan: ScanData,
    calibration: CalibrationSpec,
    receivedAt: number
): IngestResult {
    const count = scan.values.length;
    const intervalNs = scan.sampleIntervalNs;
    const empty: IngestResult = { points: [], reset: false, gap: false, skippedSamples: 0, state };
    if (count === 0) return empty;
    if (intervalNs <= 0n && count > 1) {
        // Samples cannot be placed in time; the streamer never sends this.
        return { ...empty, skippedSamples: count };
    }

    const intervalMs = Number(intervalNs) / 1_000_000;
    const timeAt = (i: number) => Number(scan.firstSampleUnixNs + intervalNs * BigInt(i)) / 1_000_000;
    const firstMs = timeAt(0);
    const sequence = scan.sequence;

    let reset = false;
    let gap = false;
    let startIndex = 0;

    if (state) {
        if (firstMs > state.lastTimestamp) {
            const gapIntervals = (firstMs - state.lastTimestamp) / Math.max(intervalMs, state.lastIntervalMs);
            gap = gapIntervals > GAP_THRESHOLD_INTERVALS;
        } else {
            const expectedFirstMs =
                state.lastFirstMs + Number(sequence - state.lastSequence) * state.lastBatchSpanMs;
            const sameTimeline = Math.abs(firstMs - expectedFirstMs) <= SAME_RUN_TOLERANCE_MS;
            if (sameTimeline && sequence <= state.lastSequence) {
                // Repeated or late batch: keep only samples newer than the buffer.
                while (startIndex < count && timeAt(startIndex) <= state.lastTimestamp) startIndex++;
                if (startIndex === count) {
                    return { ...empty, skippedSamples: count };
                }
            } else {
                reset = true;
            }
        }
    }

    const points: DataPoint[] = [];
    if (gap && state) {
        const markerTime = state.lastTimestamp + state.lastIntervalMs;
        points.push({ timestamp: markerTime, value: Number.NaN, sourceTimestamp: markerTime, receivedAt });
    }

    for (let i = startIndex; i < count; i++) {
        const timestamp = timeAt(i);
        const raw = scan.values[i];
        let value = Number.NaN;
        if (!isMissingRawValue(raw)) {
            const calibrated = applyCalibration(calibration, raw);
            value = Number.isFinite(calibrated) ? calibrated : Number.NaN;
        }
        points.push({ timestamp, value, sourceTimestamp: timestamp, receivedAt });
    }

    const nextState: ChannelStreamState = {
        lastTimestamp: timeAt(count - 1),
        lastFirstMs: firstMs,
        lastSequence: sequence,
        lastIntervalMs: intervalMs,
        lastBatchSpanMs: count * intervalMs
    };

    return { points, reset, gap, skippedSamples: startIndex, state: nextState };
}

/**
 * Appends points to a buffer and drops the oldest ones beyond `maxPoints`.
 *
 * @param buffer - Channel buffer, sorted by time. Changed in place.
 * @param points - New points, all newer than the buffer's last point.
 * @param maxPoints - Largest buffer length kept.
 * @returns Number of old points dropped.
 */
export function appendToBuffer(buffer: DataPoint[], points: DataPoint[], maxPoints: number): number {
    for (const point of points) buffer.push(point);
    const excess = buffer.length - Math.max(1, Math.floor(maxPoints));
    if (excess > 0) {
        buffer.splice(0, excess);
        return excess;
    }
    return 0;
}

/**
 * Finds the first point at or after a time by binary search.
 *
 * @param data - Points sorted by `timestamp`.
 * @param time - Time in Unix ms.
 * @returns Index of the first point with `timestamp >= time`, or `data.length`.
 */
export function lowerBound(data: DataPoint[], time: number): number {
    let low = 0;
    let high = data.length;
    while (low < high) {
        const mid = (low + high) >>> 1;
        if (data[mid].timestamp < time) low = mid + 1;
        else high = mid;
    }
    return low;
}

/**
 * Finds the first point after a time by binary search.
 *
 * @param data - Points sorted by `timestamp`.
 * @param time - Time in Unix ms.
 * @returns Index of the first point with `timestamp > time`, or `data.length`.
 */
export function upperBound(data: DataPoint[], time: number): number {
    let low = 0;
    let high = data.length;
    while (low < high) {
        const mid = (low + high) >>> 1;
        if (data[mid].timestamp <= time) low = mid + 1;
        else high = mid;
    }
    return low;
}

/**
 * Copies the newest `keepMs` of a buffer, plus the one point before that span so the
 * line reaches the left edge of the plot.
 *
 * @param data - Points sorted by time.
 * @param keepMs - Span to keep, measured back from the newest point, ms.
 * @returns A new array.
 */
export function snapshotNewest(data: DataPoint[], keepMs: number): DataPoint[] {
    if (data.length === 0) return [];
    const endTime = data[data.length - 1].timestamp;
    const startIndex = Math.max(0, lowerBound(data, endTime - keepMs) - 1);
    return data.slice(startIndex);
}

/** One received, not yet decoded live message. */
export interface QueuedScanMessage {
    /** Raw FlatBuffer `Scan` bytes. */
    payload: ArrayBuffer | Uint8Array;
    /** Browser time the message arrived, Unix ms. */
    receivedAt: number;
}

/**
 * Per-channel FIFO of received messages waiting for the next UI tick.
 *
 * Every message is kept, in arrival order, up to `maxPerChannel`. Only when a channel's
 * queue is full (the page could not decode for a long time, for example while the tab
 * was in the background and timers were throttled) is the oldest message dropped; each
 * drop is counted in {@link droppedMessages} so the page can show it.
 */
export class ScanMessageQueue {
    private queues = new Map<number, QueuedScanMessage[]>();
    /** Messages dropped per channel because the queue was full. Never reset by drains. */
    readonly droppedMessages = new Map<number, number>();

    /**
     * @param maxPerChannel - Largest number of messages held per channel.
     */
    constructor(public maxPerChannel: number = 2000) {}

    /**
     * Adds a message to the end of a channel's queue, dropping the oldest when full.
     *
     * @returns `false` if an older message had to be dropped.
     */
    push(channel: number, message: QueuedScanMessage): boolean {
        let queue = this.queues.get(channel);
        if (!queue) {
            queue = [];
            this.queues.set(channel, queue);
        }
        queue.push(message);
        if (queue.length > this.maxPerChannel) {
            const excess = queue.length - this.maxPerChannel;
            queue.splice(0, excess);
            this.droppedMessages.set(channel, (this.droppedMessages.get(channel) ?? 0) + excess);
            return false;
        }
        return true;
    }

    /** Removes and returns all queued messages of a channel, oldest first. */
    drain(channel: number): QueuedScanMessage[] {
        const queue = this.queues.get(channel);
        if (!queue || queue.length === 0) return [];
        this.queues.set(channel, []);
        return queue;
    }

    /** Number of messages waiting for a channel. */
    size(channel: number): number {
        return this.queues.get(channel)?.length ?? 0;
    }

    /**
     * Drops the queued messages and the drop counter of one channel, for a channel that
     * starts over (for example after it is selected again).
     */
    clearChannel(channel: number): void {
        this.queues.delete(channel);
        this.droppedMessages.delete(channel);
    }

    /** Drops every queued message and resets the drop counters. */
    clear(): void {
        this.queues.clear();
        this.droppedMessages.clear();
    }
}

/** Per-channel counters the page shows, accumulated by {@link drainChannelQueue}. */
export interface ChannelStreamStats {
    /** Messages decoded and applied. */
    messages: number;
    /** Payloads that failed to decode. */
    decodeErrors: number;
    /** Gap markers added (missing time between batches). */
    gaps: number;
    /** Buffer resets because the timeline jumped backward. */
    resets: number;
    /** Samples skipped as duplicates or late arrivals. */
    skippedSamples: number;
}

/** Returns a zeroed {@link ChannelStreamStats}. */
export function emptyStreamStats(): ChannelStreamStats {
    return { messages: 0, decodeErrors: 0, gaps: 0, resets: 0, skippedSamples: 0 };
}

/** Mutable per-channel state used by {@link drainChannelQueue}. */
export interface LiveChannel {
    /** Samples sorted by time, oldest first. */
    buffer: DataPoint[];
    /** Stream state after the last accepted batch. */
    stream: ChannelStreamState | null;
    /** Counters for display. */
    stats: ChannelStreamStats;
}

/** Creates an empty {@link LiveChannel}. */
export function createLiveChannel(): LiveChannel {
    return { buffer: [], stream: null, stats: emptyStreamStats() };
}

/**
 * Decodes and applies every queued message of one channel, in arrival order.
 *
 * For each message: decode, {@link ingestScan}, empty the buffer on a reset, append the
 * points, trim to `maxPoints`, then call `onChunk` so trigger logic sees each batch
 * right after it is in the buffer.
 *
 * @param queue - Message queue.
 * @param channel - Channel number.
 * @param live - Channel state; its buffer is changed in place.
 * @param decode - Payload decoder, e.g. `FlatBufferParser.parse`.
 * @param calibration - Calibration of the channel.
 * @param maxPoints - Largest buffer length.
 * @param onChunk - Called after each applied batch with the index in `live.buffer` of the
 *   batch's first point and whether the buffer was reset first.
 * @returns Number of messages taken from the queue.
 */
export function drainChannelQueue(
    queue: ScanMessageQueue,
    channel: number,
    live: LiveChannel,
    decode: (payload: ArrayBuffer | Uint8Array) => ScanData | null,
    calibration: CalibrationSpec,
    maxPoints: number,
    onChunk?: (chunkStartIndex: number, reset: boolean, chunk: DataPoint[]) => void
): number {
    const messages = queue.drain(channel);
    for (const message of messages) {
        let scan: ScanData | null = null;
        try {
            scan = decode(message.payload);
        } catch {
            scan = null;
        }
        if (!scan) {
            live.stats.decodeErrors++;
            continue;
        }

        const result = ingestScan(live.stream, scan, calibration, message.receivedAt);
        live.stats.messages++;
        live.stats.skippedSamples += result.skippedSamples;
        live.stream = result.state;
        if (result.points.length === 0) continue;

        if (result.reset) {
            live.buffer.length = 0;
            live.stats.resets++;
        }
        if (result.gap) live.stats.gaps++;

        appendToBuffer(live.buffer, result.points, maxPoints);
        const chunkStartIndex = Math.max(0, live.buffer.length - result.points.length);
        onChunk?.(chunkStartIndex, result.reset, result.points);
    }
    return messages.length;
}
