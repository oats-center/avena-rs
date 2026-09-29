/**
 * Noise filters in the live plot path (see `../filters.ts` for the pipeline).
 *
 * One {@link LiveChannelFilter} per plotted channel with filters on sits between
 * `ingestScan` and the channel buffer (see `drainChannelQueue`). It runs every sample
 * through the causal pipeline and gives each point two more fields: `unfiltered` (the
 * calibrated value) and `filtered`; `value`, which the plot and the trigger read, is
 * one or the other depending on the channel's Filtered / Raw switch. The raw volts
 * stay in `raw`.
 *
 * The despike stage needs `w - 1` samples after a sample before it can finish it, so
 * points leave the filter `w - 1` samples late (4 samples, 2 ms, at 2 kHz; none at
 * rates where despike does not run). Every point still comes out exactly once, in
 * order, with its own timestamp; the last `w - 1` points of a batch come out with the
 * next batch. Gap markers keep their place in the sequence.
 *
 * Timeline rules, matching the stream's:
 *
 * - A gap of up to {@link MAX_FILLED_GAP_SAMPLES} missing samples is filtered through
 *   as `NaN`; a longer one flushes the held points and restarts the despike window and
 *   the high-/low-pass filters, keeping the templates in phase (same run, so the sample
 *   count is known from the timestamps).
 * - A new run (the batch sequence goes back while time goes forward) flushes the held
 *   points and starts every filter over, templates included.
 * - A buffer reset (timeline jumped back) drops the held points and starts over.
 *
 * @module
 */
import { applyCalibration, type CalibrationSpec } from '../calibration';
import type { ChannelFilterSettings } from '../filter-settings';
import {
    LinearFilterChain,
    MAX_FILLED_GAP_SAMPLES,
    planFilters,
    RawFilterChain,
    type FilterPlan
} from '../filters';
import type { DataPoint, IngestResult } from './stream';

/** One point waiting for the despike window, or a gap marker keeping its place. */
interface Pending {
    /** The point, or `null` for a missing sample fed as `NaN` (dropped on output). */
    point: DataPoint | null;
    /** False for a gap marker, which is not a filter input. */
    input: boolean;
}

/** Causal filters of one live channel. */
export class LiveChannelFilter {
    /** Plan for the current sample rate; `null` before the first batch. */
    plan: FilterPlan | null = null;
    /** Whether `value` carries the filtered or the calibrated value. */
    showFiltered = true;
    private raw: RawFilterChain | null = null;
    private linear: LinearFilterChain | null = null;
    private readonly queue: Pending[] = [];
    private head = 0;
    private lastTimestamp: number | null = null;
    private lastSequence: bigint | null = null;
    private readonly outputs: number[] = [];
    private emitted: DataPoint[] = [];

    /**
     * @param settings - The channel's filter settings.
     * @param calibration - The channel's calibration, applied between the raw-volt
     *   stages and the high-/low-pass filters.
     */
    constructor(
        readonly settings: ChannelFilterSettings,
        private readonly calibration: CalibrationSpec
    ) {}

    /** Drops held points and every filter state, for a buffer reset. */
    reset(): void {
        this.queue.length = 0;
        this.head = 0;
        this.raw?.reset();
        this.linear?.reset();
        this.lastTimestamp = null;
        this.lastSequence = null;
    }

    /**
     * Sets which value `value` carries, for the points still to come and, in place,
     * for `buffer` (the channel's live buffer; captures share its points).
     */
    setShowFiltered(show: boolean, buffer: DataPoint[]): void {
        this.showFiltered = show;
        for (const point of buffer) {
            if (point.filtered === undefined) continue;
            point.value = show ? point.filtered : (point.unfiltered ?? point.value);
        }
    }

    private emit(point: DataPoint): void {
        this.emitted.push(point);
    }

    /** Hands the pipeline's outputs to the waiting points, in order. */
    private settle(): void {
        const linear = this.linear!;
        for (const y of this.outputs) {
            while (this.head < this.queue.length && !this.queue[this.head].input) {
                this.emit(this.queue[this.head++].point!);
            }
            const entry = this.queue[this.head++];
            let calibrated = Number.NaN;
            if (entry.point && Number.isFinite(y)) {
                const value = applyCalibration(this.calibration, y);
                calibrated = Number.isFinite(value) ? value : Number.NaN;
            }
            const filtered = linear.process(calibrated);
            if (!entry.point) continue;
            const point = entry.point;
            point.unfiltered = point.value;
            point.filtered = filtered;
            if (this.showFiltered) point.value = filtered;
            this.emit(point);
        }
        this.outputs.length = 0;
        while (this.head < this.queue.length && !this.queue[this.head].input) {
            this.emit(this.queue[this.head++].point!);
        }
        if (this.head > 256 && this.head * 2 > this.queue.length) {
            this.queue.splice(0, this.head);
            this.head = 0;
        }
    }

    /** Emits every held point, finishing the despike window at the last input. */
    private flush(): void {
        if (!this.raw) return;
        this.raw.flush(this.outputs);
        this.settle();
    }

    /**
     * Filters the points of one ingested batch.
     *
     * @param result - From `ingestScan`; its points are changed in place.
     * @param sequence - The batch's sequence number.
     * @param intervalNs - The batch's sample interval.
     * @returns The points ready to append, oldest first: held points of earlier
     *   batches, then this batch's except the last few the despike window holds.
     */
    process(result: IngestResult, sequence: bigint, intervalNs: bigint): DataPoint[] {
        this.emitted = [];
        const points = result.points;
        if (points.length === 0 || intervalNs <= 0n) return this.emitted;
        if (result.reset) this.reset();

        const fs = 1e9 / Number(intervalNs);
        if (!this.plan || this.plan.fs !== fs) {
            this.flush();
            this.plan = planFilters(this.settings, fs);
            this.raw = new RawFilterChain(this.plan);
            this.linear = new LinearFilterChain(this.plan);
            this.lastTimestamp = null;
        }
        const intervalMs = 1000 / fs;

        let start = 0;
        const firstSample = result.gap ? points[1] : points[0];
        if (
            this.lastSequence !== null &&
            sequence < this.lastSequence &&
            this.lastTimestamp !== null &&
            firstSample &&
            firstSample.timestamp > this.lastTimestamp
        ) {
            // A new run: its sample count and phase are unrelated to the last one.
            this.flush();
            this.raw!.reset();
            this.linear!.reset();
            this.lastTimestamp = null;
        }
        if (result.gap) {
            const marker = points[0];
            start = 1;
            const missing =
                this.lastTimestamp !== null && firstSample
                    ? Math.max(0, Math.round((firstSample.timestamp - this.lastTimestamp) / intervalMs) - 1)
                    : 0;
            if (missing > MAX_FILLED_GAP_SAMPLES) {
                this.raw!.breakSegment(missing, this.outputs);
                this.settle();
                this.linear!.reset();
                this.queue.push({ point: marker, input: false });
            } else {
                this.queue.push({ point: marker, input: false });
                for (let k = 0; k < missing; k++) {
                    this.queue.push({ point: null, input: true });
                    this.raw!.push(Number.NaN, this.outputs);
                }
            }
            this.settle();
        }
        for (let i = start; i < points.length; i++) {
            const point = points[i];
            this.queue.push({ point, input: true });
            this.raw!.push(point.raw ?? Number.NaN, this.outputs);
            this.settle();
        }
        this.lastTimestamp = points[points.length - 1].timestamp;
        this.lastSequence = sequence;
        return this.emitted;
    }
}
