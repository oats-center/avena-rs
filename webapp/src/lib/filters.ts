/**
 * Noise filters for LabJack readings, applied where data is read (never to the
 * archive). The same pipeline, with the same arithmetic, is implemented in Rust in
 * `rust-ljm/src/filters.rs` for exports; shared test vectors in
 * `testdata/filter-vectors.json` check that both give the same output.
 *
 * Order, per sample:
 *
 * 1. Despike (raw volts): grey opening, a running minimum then a running maximum over
 *    `w = ceil(2.5 ms × fs)` samples. It removes upward spikes narrower than `w` and
 *    keeps wider pulses and anything downward. Output sample `j` needs input up to
 *    `j + w - 1`, so this stage delays its output by `w - 1` samples.
 * 2. Remove the 10 Hz square wave (raw volts): subtract a template of one period,
 *    `fs / 10` samples, indexed by the sample count since the segment started (the
 *    source runs on the LabJack clock). Each phase bin holds a running mean of the
 *    past samples in that bin (cumulative at first, then exponential with a 30 s
 *    memory); the template is the bin mean minus the running mean of all samples, so
 *    the signal level is kept. The template is read before the current sample updates it, so the
 *    stage is causal and adds no delay.
 * 3. Remove the 11.906 Hz square wave the same way (it runs beside stage 2: each
 *    template learns from the sample minus the other's current template). Its period is not a whole number
 *    of samples, so samples go to phase bins by their fractional phase, with four bins
 *    per sample of period (at least 64); a bin not yet reached uses the mean of its
 *    group of four.
 * 4. Calibration (done by the caller, between {@link RawFilterChain} and
 *    {@link LinearFilterChain}).
 * 5. High-pass, then low-pass: 2nd-order Butterworth biquads (bilinear transform with
 *    prewarping). Causal here; exports run them forward and backward.
 *
 * `NaN` samples pass through as `NaN`; the filters hold their state over them and the
 * sample count still advances. A caller breaks the segment on a long gap
 * ({@link RawFilterChain.breakSegment}) or resets on a new run.
 *
 * @module
 */
import type { ChannelFilterSettings } from './filter-settings';
import { normalizeChannelFilters } from './filter-settings';

/** Length of the despike window, ms. Spikes on MU2 are 0.5 to 2 ms wide; axle pulses about 12 ms. */
export const DESPIKE_WINDOW_MS = 2.5;
/** Frequency of the first square wave, Hz. Its period must be a whole number of samples. */
export const TEN_HZ = 10;
/**
 * Frequency of the second square wave, Hz. Measured on MU1 and MU2 (lines at 11.906,
 * 35.72 and 59.53 Hz); it also runs on the LabJack clock.
 */
export const ELEVEN_NINE_HZ = 11.906;
/** Memory of the template running means, seconds. */
export const TEMPLATE_MEMORY_S = 30;
/** Fewest samples per period for a template. */
export const MIN_TEMPLATE_PERIOD_SAMPLES = 4;
/** How far `fs / 10` may be from a whole number of samples. */
export const PERIOD_TOLERANCE_SAMPLES = 0.01;
/** Fewest phase bins for the fractional (11.9 Hz) template. */
export const MIN_FRACTIONAL_BINS = 64;
/** Phase bins per sample of period in the fractional template. */
export const FRACTIONAL_BINS_PER_SAMPLE = 4;
/** Highest usable cutoff, as a fraction of the sample rate. */
export const MAX_CUTOFF_FRACTION = 0.45;
/**
 * Longest run of missing samples that is filled with `NaN` and filtered through. A
 * longer gap breaks the segment: the despike window and the biquads start over.
 */
export const MAX_FILLED_GAP_SAMPLES = 4;

/** What runs for one channel at one sample rate; from {@link planFilters}. */
export interface FilterPlan {
    /** Sample rate, Hz. */
    fs: number;
    /** Despike window in samples, or 0 when off. */
    despikeWindow: number;
    /** 10 Hz period in samples (whole), or 0 when off. */
    period10: number;
    /** 11.9 Hz period in samples (fractional), or 0 when off. */
    period11: number;
    /** Phase bins of the 11.9 Hz template. */
    bins11: number;
    /** High-pass cutoff, Hz, or 0 when off. */
    highpassHz: number;
    /** Low-pass cutoff, Hz, or 0 when off. */
    lowpassHz: number;
    /** Filters that were asked for but cannot run at this rate, with the reason. */
    skipped: string[];
    /** Samples by which the output lags the input (the despike window minus one). */
    delaySamples: number;
    /** True when at least one stage runs. */
    active: boolean;
}

/**
 * Works out which filters run at a sample rate.
 *
 * @param settings - The channel's settings, or `undefined`.
 * @param fs - Sample rate, Hz.
 * @returns The plan. Filters that cannot run are listed in `skipped`.
 */
export function planFilters(settings: ChannelFilterSettings | undefined, fs: number): FilterPlan {
    const clean = normalizeChannelFilters(settings) ?? {};
    const plan: FilterPlan = {
        fs,
        despikeWindow: 0,
        period10: 0,
        period11: 0,
        bins11: 0,
        highpassHz: 0,
        lowpassHz: 0,
        skipped: [],
        delaySamples: 0,
        active: false
    };
    const validRate = Number.isFinite(fs) && fs > 0;
    if (clean.despike) {
        const window = validRate ? Math.ceil((fs * DESPIKE_WINDOW_MS) / 1000 - 1e-9) : 0;
        if (window >= 2) plan.despikeWindow = window;
        else plan.skipped.push(`despike needs more than ${1000 / DESPIKE_WINDOW_MS} Hz`);
    }
    if (clean.remove_10hz) {
        const exact = fs / TEN_HZ;
        const period = Math.round(exact);
        if (validRate && Math.abs(exact - period) <= PERIOD_TOLERANCE_SAMPLES && period >= MIN_TEMPLATE_PERIOD_SAMPLES) {
            plan.period10 = period;
        } else {
            plan.skipped.push(`10 Hz removal needs a rate that is a multiple of 10 Hz, at least ${TEN_HZ * MIN_TEMPLATE_PERIOD_SAMPLES} Hz`);
        }
    }
    if (clean.remove_11_9hz) {
        const period = fs / ELEVEN_NINE_HZ;
        if (validRate && period >= MIN_TEMPLATE_PERIOD_SAMPLES) {
            plan.period11 = period;
            plan.bins11 = Math.max(MIN_FRACTIONAL_BINS, FRACTIONAL_BINS_PER_SAMPLE * Math.ceil(period));
        } else {
            plan.skipped.push(`11.9 Hz removal needs at least ${Math.ceil(ELEVEN_NINE_HZ * MIN_TEMPLATE_PERIOD_SAMPLES)} Hz`);
        }
    }
    if (clean.highpass_hz !== undefined) {
        if (validRate && clean.highpass_hz < MAX_CUTOFF_FRACTION * fs) plan.highpassHz = clean.highpass_hz;
        else plan.skipped.push(`high-pass ${clean.highpass_hz} Hz is above ${MAX_CUTOFF_FRACTION} × the rate`);
    }
    if (clean.lowpass_hz !== undefined) {
        if (validRate && clean.lowpass_hz < MAX_CUTOFF_FRACTION * fs) plan.lowpassHz = clean.lowpass_hz;
        else plan.skipped.push(`low-pass ${clean.lowpass_hz} Hz is not below ${MAX_CUTOFF_FRACTION} × the rate`);
    }
    plan.delaySamples = plan.despikeWindow > 1 ? plan.despikeWindow - 1 : 0;
    plan.active =
        plan.despikeWindow > 0 || plan.period10 > 0 || plan.period11 > 0 || plan.highpassHz > 0 || plan.lowpassHz > 0;
    return plan;
}

/** Normalised biquad coefficients (`a0 = 1`). */
export interface BiquadCoefficients {
    b0: number;
    b1: number;
    b2: number;
    a1: number;
    a2: number;
}

/**
 * 2nd-order Butterworth coefficients by the bilinear transform with prewarping (the
 * audio-EQ cookbook formulas with Q = 1/√2).
 *
 * @param kind - `highpass` or `lowpass`.
 * @param cutoffHz - −3 dB frequency, Hz.
 * @param fs - Sample rate, Hz.
 */
export function butterworth(kind: 'highpass' | 'lowpass', cutoffHz: number, fs: number): BiquadCoefficients {
    const w0 = (2 * Math.PI * cutoffHz) / fs;
    const cosW = Math.cos(w0);
    const alpha = Math.sin(w0) / Math.SQRT2;
    const a0 = 1 + alpha;
    const bMid = kind === 'lowpass' ? 1 - cosW : -(1 + cosW);
    const bEdge = kind === 'lowpass' ? (1 - cosW) / 2 : (1 + cosW) / 2;
    return {
        b0: bEdge / a0,
        b1: bMid / a0,
        b2: bEdge / a0,
        a1: (-2 * cosW) / a0,
        a2: (1 - alpha) / a0
    };
}

/**
 * One biquad section, transposed direct form II. The first finite sample after a
 * reset sets the state to the steady state for that value (as if it had been constant
 * forever), so a high-pass does not ring on the signal's offset. `NaN` in gives `NaN`
 * out and leaves the state alone.
 */
export class Biquad {
    private z1 = 0;
    private z2 = 0;
    private started = false;

    constructor(private readonly c: BiquadCoefficients) {}

    /** Forgets the state; the next finite sample starts it again. */
    reset(): void {
        this.started = false;
        this.z1 = 0;
        this.z2 = 0;
    }

    /** Filters one sample. */
    process(x: number): number {
        if (!Number.isFinite(x)) return Number.NaN;
        const c = this.c;
        if (!this.started) {
            const gain = (c.b0 + c.b1 + c.b2) / (1 + c.a1 + c.a2);
            const y0 = gain * x;
            this.z2 = c.b2 * x - c.a2 * y0;
            this.z1 = c.b1 * x - c.a1 * y0 + this.z2;
            this.started = true;
        }
        const y = c.b0 * x + this.z1;
        this.z1 = c.b1 * x - c.a1 * y + this.z2;
        this.z2 = c.b2 * x - c.a2 * y;
        return y;
    }
}

/**
 * Streaming grey opening (running minimum, then running maximum) over `w` samples.
 *
 * Output `j` is the largest of the minima of the `w`-sample windows that contain `j`;
 * windows are cut at the ends of the segment and `NaN` samples are left out of them. It
 * is emitted once input `j + w - 1` has arrived, or by {@link flush}.
 */
class Despiker {
    /** Recent inputs, indexed by sample number modulo `2w`. */
    private readonly xs: Float64Array;
    /** Window minima `e[k]`, indexed by `k` modulo `w` (k may be negative). */
    private readonly es: Float64Array;
    /** Inputs taken since the segment started. */
    private count = 0;
    /** Outputs emitted since the segment started. */
    private emitted = 0;

    constructor(private readonly w: number) {
        this.xs = new Float64Array(2 * w);
        this.es = new Float64Array(w);
    }

    /** Minimum of the finite inputs in `[from, to]`, cut to `[0, count - 1]`; `NaN` if none. */
    private windowMin(from: number, to: number): number {
        let min = Number.NaN;
        const start = Math.max(0, from);
        const end = Math.min(this.count - 1, to);
        for (let m = start; m <= end; m++) {
            const x = this.xs[m % this.xs.length];
            if (x === x && !(x >= min)) min = x;
        }
        return min;
    }

    private storeMin(k: number, value: number): void {
        this.es[((k % this.w) + this.w) % this.w] = value;
    }

    /** Emits output `j` (needs the inputs up to `j + w - 1`, or the segment's end). */
    private emit(j: number, out: number[]): void {
        const w = this.w;
        if (j === 0) {
            for (let k = -(w - 1); k < 0; k++) this.storeMin(k, this.windowMin(k, k + w - 1));
        }
        this.storeMin(j, this.windowMin(j, j + w - 1));
        const x = this.xs[j % this.xs.length];
        let y = Number.NaN;
        if (x === x) {
            for (let k = j - w + 1; k <= j; k++) {
                const e = this.es[((k % w) + w) % w];
                if (e === e && !(e <= y)) y = e;
            }
        }
        out.push(y);
        this.emitted = j + 1;
    }

    /** Takes one input and appends the output it completes, if any, to `out`. */
    push(x: number, out: number[]): void {
        this.xs[this.count % this.xs.length] = x;
        this.count++;
        const j = this.count - this.w;
        if (j >= 0) this.emit(j, out);
    }

    /** Appends the outputs still held back, with the windows cut at the last input, and starts over. */
    flush(out: number[]): void {
        for (let j = this.emitted; j < this.count; j++) this.emit(j, out);
        this.reset();
    }

    /** Drops everything held and starts over. */
    reset(): void {
        this.count = 0;
        this.emitted = 0;
    }
}

/**
 * Running template of a periodic waveform in phase bins. Each bin keeps a running mean
 * of the samples that fell in it, and a running mean of all samples is kept beside
 * them: cumulative for the first samples, then exponential, both with a memory of
 * {@link TEMPLATE_MEMORY_S}. The template of a bin is its mean minus the overall mean,
 * so subtracting it removes the periodic part and keeps the signal level. A bin with no
 * sample yet has no template.
 */
class PeriodicTemplate {
    private readonly means: Float64Array;
    private readonly counts: Float64Array;
    private readonly coarseMeans: Float64Array;
    private readonly coarseCounts: Float64Array;
    private mean = 0;
    private count = 0;

    /**
     * @param bins - Number of phase bins.
     * @param group - Bins per coarse bin. A bin with no sample yet uses its coarse bin's
     *   mean, so a fine template works from the first cycle. 1 for none.
     * @param alpha - Smallest weight of a new sample in the overall mean; a bin's weight
     *   is this times the number of bins, so every mean has the same memory.
     */
    constructor(
        private readonly bins: number,
        private readonly group: number,
        private readonly alpha: number
    ) {
        this.means = new Float64Array(bins);
        this.counts = new Float64Array(bins);
        const coarse = group > 1 ? Math.ceil(bins / group) : 0;
        this.coarseMeans = new Float64Array(coarse);
        this.coarseCounts = new Float64Array(coarse);
    }

    /** Template of `bin`, from the samples learnt so far; 0 while nothing covers it. */
    template(bin: number): number {
        if (this.counts[bin] > 0) return this.means[bin] - this.mean;
        if (this.group > 1) {
            const coarse = Math.floor(bin / this.group);
            if (this.coarseCounts[coarse] > 0) return this.coarseMeans[coarse] - this.mean;
        }
        return 0;
    }

    /** Adds a finite sample to its bin's mean and to the overall mean. */
    learn(x: number, bin: number): void {
        const binCount = this.counts[bin] + 1;
        this.counts[bin] = binCount;
        this.means[bin] += Math.max(1 / binCount, this.alpha * this.bins) * (x - this.means[bin]);
        if (this.group > 1) {
            const coarse = Math.floor(bin / this.group);
            const coarseCount = this.coarseCounts[coarse] + 1;
            this.coarseCounts[coarse] = coarseCount;
            this.coarseMeans[coarse] +=
                Math.max(1 / coarseCount, this.alpha * this.coarseMeans.length) * (x - this.coarseMeans[coarse]);
        }
        this.count += 1;
        this.mean += Math.max(1 / this.count, this.alpha) * (x - this.mean);
    }

    reset(): void {
        this.means.fill(0);
        this.counts.fill(0);
        this.coarseMeans.fill(0);
        this.coarseCounts.fill(0);
        this.mean = 0;
        this.count = 0;
    }
}

/**
 * Despike and template stages, on raw volts. Takes one input at a time and appends
 * outputs, in input order, to the array it is given: nothing for the first
 * `delaySamples` inputs of a segment, then one per input; {@link flush} appends the
 * rest.
 */
export class RawFilterChain {
    readonly plan: FilterPlan;
    /** Despike stage, or `null` when off. @internal */
    private readonly despiker: Despiker | null;
    /** 10 Hz template, or `null` when off. @internal */
    private readonly template10: PeriodicTemplate | null;
    /** 11.9 Hz template, or `null` when off. @internal */
    private readonly template11: PeriodicTemplate | null;
    private readonly ratio11: number;
    /** Sample number since the start of the run, for the template phase. */
    private index = 0;
    /** Scratch array for the despiker's outputs. */
    private readonly pending: number[] = [];

    constructor(plan: FilterPlan) {
        this.plan = plan;
        this.despiker = plan.despikeWindow > 1 ? new Despiker(plan.despikeWindow) : null;
        // A bin is updated fs / bins times a second, so these weights give each running
        // mean a memory of TEMPLATE_MEMORY_S.
        const alpha = 1 / (TEMPLATE_MEMORY_S * plan.fs);
        this.template10 =
            plan.period10 > 0 ? new PeriodicTemplate(plan.period10, 1, alpha) : null;
        this.template11 = plan.period11 > 0 ? new PeriodicTemplate(plan.bins11, FRACTIONAL_BINS_PER_SAMPLE, alpha) : null;
        this.ratio11 = ELEVEN_NINE_HZ / plan.fs;
    }

    /**
     * Runs the template stages on one despiked sample and appends it. Both templates are
     * read before the sample updates them. Each learns from the sample minus the other's
     * current template, so neither soaks up the other wave while they are young.
     */
    private finish(x: number, out: number[]): void {
        const n = this.index++;
        const t10 = this.template10;
        const t11 = this.template11;
        const bin10 = t10 ? n % this.plan.period10 : 0;
        let bin11 = 0;
        if (t11) {
            const turns = n * this.ratio11;
            bin11 = Math.min(this.plan.bins11 - 1, Math.floor((turns - Math.floor(turns)) * this.plan.bins11));
        }
        const template10 = t10 ? t10.template(bin10) : 0;
        const template11 = t11 ? t11.template(bin11) : 0;
        if (Number.isFinite(x)) {
            t10?.learn(x - template11, bin10);
            t11?.learn(x - template10, bin11);
        }
        out.push(x - template10 - template11);
    }

    /** Takes one raw sample (`NaN` for a missing one) and appends the outputs it completes. */
    push(x: number, out: number[]): void {
        if (!this.despiker) {
            this.finish(x, out);
            return;
        }
        const pending = this.pending;
        pending.length = 0;
        this.despiker.push(x, pending);
        for (let i = 0; i < pending.length; i++) this.finish(pending[i], out);
    }

    /** Appends the outputs still held back by the despike window. */
    flush(out: number[]): void {
        if (!this.despiker) return;
        const pending = this.pending;
        pending.length = 0;
        this.despiker.flush(pending);
        for (let i = 0; i < pending.length; i++) this.finish(pending[i], out);
    }

    /**
     * Ends the segment before `missing` absent samples of the same run: appends the held
     * outputs and restarts the despike window, and advances the sample count so the
     * templates, which are kept, stay in phase.
     */
    breakSegment(missing: number, out: number[]): void {
        this.flush(out);
        this.index += Math.max(0, Math.floor(missing));
    }

    /** Drops everything, including the templates, for a new run. */
    reset(): void {
        this.despiker?.reset();
        this.template10?.reset();
        this.template11?.reset();
        this.index = 0;
    }
}

/** High-pass then low-pass, causal, on calibrated values. */
export class LinearFilterChain {
    private readonly highpass: Biquad | null;
    private readonly lowpass: Biquad | null;

    constructor(plan: FilterPlan) {
        this.highpass = plan.highpassHz > 0 ? new Biquad(butterworth('highpass', plan.highpassHz, plan.fs)) : null;
        this.lowpass = plan.lowpassHz > 0 ? new Biquad(butterworth('lowpass', plan.lowpassHz, plan.fs)) : null;
    }

    /** Filters one sample. */
    process(x: number): number {
        let y = x;
        if (this.highpass) y = this.highpass.process(y);
        if (this.lowpass) y = this.lowpass.process(y);
        return y;
    }

    /** Forgets the state (after a long gap or a new run). */
    reset(): void {
        this.highpass?.reset();
        this.lowpass?.reset();
    }
}

/**
 * Runs the causal pipeline over a whole array, with no calibration (volts in, volts
 * out). Missing samples are `NaN`; a run of more than {@link MAX_FILLED_GAP_SAMPLES}
 * `NaN` breaks the segment as a live gap would. Used by the tests and the shared test
 * vectors.
 *
 * @param values - Raw samples, in order.
 * @param plan - From {@link planFilters}.
 * @returns One output per input, aligned with the input (the despike delay removed).
 */
export function filterCausal(values: ArrayLike<number>, plan: FilterPlan): Float64Array {
    const raw = new RawFilterChain(plan);
    const linear = new LinearFilterChain(plan);
    const result = new Float64Array(values.length);
    const out: number[] = [];
    let written = 0;
    const drain = () => {
        for (const y of out) result[written++] = linear.process(y);
        out.length = 0;
    };
    let i = 0;
    while (i < values.length) {
        if (Number.isFinite(values[i])) {
            raw.push(values[i], out);
            drain();
            i++;
            continue;
        }
        let end = i;
        while (end < values.length && !Number.isFinite(values[end])) end++;
        const missing = end - i;
        if (missing <= MAX_FILLED_GAP_SAMPLES) {
            for (let k = 0; k < missing; k++) raw.push(Number.NaN, out);
            drain();
        } else {
            // Emit what the despike window held, then skip the gap keeping the phase.
            raw.breakSegment(missing, out);
            drain();
            for (let k = 0; k < missing; k++) result[written++] = Number.NaN;
            linear.reset();
        }
        i = end;
    }
    raw.flush(out);
    drain();
    return result;
}
