import { describe, expect, it } from 'vitest';
import storedVectors from '../../../testdata/filter-vectors.json';
import type { ChannelFilterSettings } from './filter-settings';
import {
    Biquad,
    butterworth,
    filterCausal,
    planFilters,
    RawFilterChain,
    type FilterPlan
} from './filters';

/**
 * Shared test vectors, also read by `rust-ljm/src/filters.rs`. Regenerate with
 * `UPDATE_FILTER_VECTORS=1 pnpm vitest run src/lib/filters.test.ts` after a deliberate
 * change to the pipeline, and check that the Rust tests still pass.
 */
const VECTORS_URL = new URL('../../../testdata/filter-vectors.json', import.meta.url);

/** One synthetic signal and what the causal pipeline gives for it. */
interface VectorCase {
    name: string;
    fs: number;
    settings: ChannelFilterSettings;
    /** Raw samples in µV (integers); `null` is a missing sample. */
    input_uv: (number | null)[];
    /** The two square waves alone, µV, so checks can tell how much of them is left. */
    squares_uv: number[];
    /** Sample indices of the one-sample spikes. */
    spikes: number[];
    /** Height of each spike, µV. */
    spike_uv: number[];
    /** Centre sample of each 12 ms pulse. */
    pulses: number[];
    /** Pulse height, µV. */
    pulse_uv: number;
    /** Samples from the start that the checks skip while the templates learn. */
    warmup: number;
    /** Missing-sample runs as `[start, length]`. */
    gaps: [number, number][];
    /** Output of `filterCausal` in µV, rounded to 1e-9 µV; `null` for `NaN`. */
    expected_causal_uv: (number | null)[];
}

/** Small deterministic PRNG (mulberry32). */
function prng(seed: number): () => number {
    let a = seed >>> 0;
    return () => {
        a = (a + 0x6d2b79f5) >>> 0;
        let t = a;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

/**
 * Builds a synthetic record like the I-69 strain channels: 0.5 V offset, 10 Hz square
 * wave ±3 mV locked to the sample count, 11.906 Hz square wave ±1.5 mV, 0.2 mV noise,
 * one-sample upward spikes, three 12 ms half-sine pulses of 20 mV and two gaps.
 */
function makeCase(name: string, fs: number, seconds: number, settings: ChannelFilterSettings, seed: number) {
    const n = Math.round(fs * seconds);
    const random = prng(seed);
    const gauss = () => Math.sqrt(-2 * Math.log(1 - random())) * Math.cos(2 * Math.PI * random());
    const pulses = (fs >= 1000 ? [2.47, 3.03, 3.41] : [12.3, 18.5, 25.7]).map((t) => Math.round(t * fs));
    const gaps: [number, number][] =
        fs >= 1000 ? [[Math.round(2.2 * fs), 3], [Math.round(3.6 * fs), 40]] : [[Math.round(15 * fs), 2], [Math.round(21 * fs), 10]];
    const spikeRate = fs >= 1000 ? 0.08 : 0.02;
    const pulseUv = 20000;
    const input: (number | null)[] = [];
    const squares: number[] = [];
    const spikes: number[] = [];
    const spikeHeights: number[] = [];
    const period10 = fs / 10;
    for (let i = 0; i < n; i++) {
        const turns11 = (i * 11.906) / fs;
        const sq = (i % period10 < period10 / 2 ? 3000 : -3000) + (turns11 - Math.floor(turns11) < 0.5 ? 1500 : -1500);
        let pulse = 0;
        for (const c of pulses) {
            const t = (i - c) / fs;
            if (Math.abs(t) < 0.006) pulse += pulseUv * Math.cos((Math.PI * t) / 0.012);
        }
        let spike = 0;
        const nearPulse = pulses.some((c) => Math.abs(i - c) < 0.03 * fs);
        if (!nearPulse && random() < spikeRate) {
            spike = 3000 + Math.round(5000 * random());
            spikes.push(i);
            spikeHeights.push(spike);
        }
        squares.push(sq);
        input.push(Math.round(500000 + sq + pulse + spike + 200 * gauss()));
    }
    for (const [start, length] of gaps) {
        for (let i = start; i < start + length; i++) input[i] = null;
    }
    return {
        name,
        fs,
        settings,
        input_uv: input,
        squares_uv: squares,
        spikes,
        spike_uv: spikeHeights,
        pulses,
        pulse_uv: pulseUv,
        warmup: Math.round(fs * (fs >= 1000 ? 2 : 10)),
        gaps
    };
}

const toVolts = (values: (number | null)[]) => values.map((v) => (v === null ? Number.NaN : v / 1e6));

function makeVectors(): VectorCase[] {
    const all: ChannelFilterSettings = { despike: true, remove_10hz: true, remove_11_9hz: true, highpass_hz: 1, lowpass_hz: 100 };
    return [makeCase('2khz', 2000, 4, all, 1), makeCase('100hz', 100, 30, all, 2)].map((c) => {
        const output = filterCausal(toVolts(c.input_uv), planFilters(c.settings, c.fs));
        return {
            ...c,
            expected_causal_uv: Array.from(output, (y) => (Number.isFinite(y) ? Number((y * 1e6).toFixed(9)) : null))
        };
    });
}

async function loadVectors(): Promise<VectorCase[]> {
    // Node modules are loaded by name so the webapp needs no Node type definitions.
    const env = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env;
    if (env?.UPDATE_FILTER_VECTORS === '1') {
        const fsModule = 'node:fs';
        const urlModule = 'node:url';
        const { writeFileSync } = await import(/* @vite-ignore */ fsModule);
        const { fileURLToPath } = await import(/* @vite-ignore */ urlModule);
        const cases = makeVectors();
        const body = {
            description:
                'Shared test vectors for webapp/src/lib/filters.ts and rust-ljm/src/filters.rs. Generated by filters.test.ts (UPDATE_FILTER_VECTORS=1).',
            cases
        };
        writeFileSync(fileURLToPath(VECTORS_URL), JSON.stringify(body) + '\n');
        return cases;
    }
    return (storedVectors as unknown as { cases: VectorCase[] }).cases;
}

/** Indices the quality checks use: after the warm-up, away from pulses and gaps. */
function evaluationMask(c: VectorCase): boolean[] {
    const mask = c.input_uv.map((v, i) => v !== null && i >= c.warmup);
    const clear = (from: number, to: number) => {
        for (let i = Math.max(0, from); i < Math.min(mask.length, to); i++) mask[i] = false;
    };
    for (const p of c.pulses) clear(p - Math.round(0.08 * c.fs), p + Math.round(0.08 * c.fs));
    for (const [start, length] of c.gaps) clear(start - Math.round(0.05 * c.fs), start + length + Math.round(0.3 * c.fs));
    return mask;
}

/**
 * Remaining square wave and spikes, dB below the originals, and the pulse heights kept
 * (fraction). Spikes are measured against a run without them.
 */
function quality(c: VectorCase, run: (volts: number[]) => ArrayLike<number>) {
    const full = run(toVolts(c.input_uv));
    const clean = run(toVolts(c.input_uv.map((v, i) => (v === null ? null : v - c.squares_uv[i]))));
    const mask = evaluationMask(c);
    let power = 0;
    let residual = 0;
    for (let i = 0; i < mask.length; i++) {
        if (!mask[i]) continue;
        power += (c.squares_uv[i] / 1e6) ** 2;
        residual += (full[i] - clean[i]) ** 2;
    }
    const pulseKept = c.pulses.map((p) => {
        const w = Math.round(0.01 * c.fs);
        let peak = -Infinity;
        for (let i = p - w; i <= p + w; i++) peak = Math.max(peak, full[i]);
        const base: number[] = [];
        for (let i = p - Math.round(0.07 * c.fs); i < p - Math.round(0.03 * c.fs); i++) base.push(full[i]);
        base.sort((a, b) => a - b);
        return (peak - base[base.length >> 1]) / (c.pulse_uv / 1e6);
    });
    // What is left of each spike: the output minus the output without the spikes.
    const withoutSpikes = [...c.input_uv];
    c.spikes.forEach((s, k) => {
        const v = withoutSpikes[s];
        if (v !== null) withoutSpikes[s] = v - c.spike_uv[k];
    });
    const smooth = run(toVolts(withoutSpikes));
    // A spike next to a square-wave edge merges into the edge (the opening moves the
    // edge by a sample), so only spikes on a flat stretch are checked.
    const flat = (s: number) => {
        for (let i = s - 6; i <= s + 6; i++) if (c.squares_uv[i] !== c.squares_uv[s]) return false;
        return true;
    };
    let spikePower = 0;
    let spikeResidual = 0;
    let spikeCount = 0;
    c.spikes.forEach((s, k) => {
        if (!mask[s] || !flat(s)) return;
        spikeCount++;
        spikePower += (c.spike_uv[k] / 1e6) ** 2;
        spikeResidual += (full[s] - smooth[s]) ** 2;
    });
    const spikeDb = 10 * Math.log10(spikePower / spikeResidual);
    return { reductionDb: 10 * Math.log10(power / residual), pulseKept, spikeCount, spikeDb };
}

describe('planFilters', () => {
    const all: ChannelFilterSettings = { despike: true, remove_10hz: true, remove_11_9hz: true, highpass_hz: 1, lowpass_hz: 100 };

    it('runs everything at 2 kHz with a 5-sample despike window', () => {
        const plan = planFilters(all, 2000);
        expect(plan).toMatchObject({ despikeWindow: 5, period10: 200, bins11: 672, highpassHz: 1, lowpassHz: 100, delaySamples: 4, active: true });
        expect(plan.period11).toBeCloseTo(167.98, 2);
        expect(plan.skipped).toEqual([]);
    });

    it('skips what cannot run at 100 Hz and says why', () => {
        const plan = planFilters(all, 100);
        expect(plan).toMatchObject({ despikeWindow: 0, period10: 10, highpassHz: 1, lowpassHz: 0, delaySamples: 0 });
        expect(plan.skipped).toHaveLength(2);
    });

    it('skips the 10 Hz template when the period is not a whole number of samples', () => {
        expect(planFilters({ remove_10hz: true }, 2205).period10).toBe(0);
        expect(planFilters({ remove_10hz: true }, 2200.0013).period10).toBe(220);
        expect(planFilters({ remove_10hz: true }, 30).period10).toBe(0);
    });

    it('is inactive without settings', () => {
        expect(planFilters(undefined, 2000).active).toBe(false);
    });
});

describe('Biquad', () => {
    it('starts in steady state, so a constant passes a low-pass and vanishes in a high-pass', () => {
        const low = new Biquad(butterworth('lowpass', 10, 1000));
        const high = new Biquad(butterworth('highpass', 1, 1000));
        for (let i = 0; i < 100; i++) {
            expect(low.process(3.7)).toBeCloseTo(3.7, 12);
            expect(Math.abs(high.process(3.7))).toBeLessThan(1e-12);
        }
    });

    it('passes NaN through without touching its state', () => {
        const low = new Biquad(butterworth('lowpass', 10, 1000));
        low.process(1);
        expect(low.process(Number.NaN)).toBeNaN();
        expect(low.process(1)).toBeCloseTo(1, 12);
    });

    it('is −3 dB at the cutoff', () => {
        const fs = 1000;
        const f = 50;
        const low = new Biquad(butterworth('lowpass', f, fs));
        let peak = 0;
        for (let i = 0; i < 4000; i++) {
            const y = low.process(Math.sin((2 * Math.PI * f * i) / fs));
            if (i > 2000) peak = Math.max(peak, Math.abs(y));
        }
        expect(peak).toBeCloseTo(Math.SQRT1_2, 2);
    });
});

describe('despike', () => {
    const plan = planFilters({ despike: true }, 2000);

    it('removes upward spikes narrower than the window and keeps wider pulses and dips', () => {
        const x = new Array(60).fill(0);
        x[10] = 5;
        x[20] = 5;
        x[21] = 5;
        for (let i = 30; i < 36; i++) x[i] = 4; // 6 samples: wider than the 5-sample window
        x[45] = -5;
        const y = filterCausal(x, plan);
        expect(y[10]).toBe(0);
        expect(y[20]).toBe(0);
        expect(y[21]).toBe(0);
        expect(Array.from(y.slice(30, 36))).toEqual([4, 4, 4, 4, 4, 4]);
        expect(y[45]).toBe(-5);
    });

    it('delays by the window minus one and emits every sample once, in order, after a flush', () => {
        const chain = new RawFilterChain(plan);
        const out: number[] = [];
        for (let i = 0; i < 4; i++) chain.push(i, out);
        expect(out).toEqual([]);
        chain.push(4, out);
        expect(out).toHaveLength(1);
        chain.flush(out);
        expect(out).toEqual([0, 1, 2, 3, 4]);
    });

    it('matches a grey opening with nearest-edge padding', () => {
        const random = prng(7);
        const x = Array.from({ length: 200 }, () => random());
        const y = filterCausal(x, plan);
        const w = 5;
        const at = (i: number) => x[Math.min(x.length - 1, Math.max(0, i))];
        for (let j = 0; j < x.length; j++) {
            let best = -Infinity;
            for (let k = j - w + 1; k <= j; k++) {
                let min = Infinity;
                for (let m = k; m < k + w; m++) min = Math.min(min, at(m));
                best = Math.max(best, min);
            }
            expect(y[j]).toBe(best);
        }
    });
});

describe('shared test vectors', async () => {
    const cases = await loadVectors();

    for (const c of cases) {
        it(`${c.name}: the causal pipeline reproduces the stored output`, () => {
            const output = filterCausal(toVolts(c.input_uv), planFilters(c.settings, c.fs));
            expect(output.length).toBe(c.expected_causal_uv.length);
            for (let i = 0; i < output.length; i++) {
                const expected = c.expected_causal_uv[i];
                if (expected === null) {
                    expect(output[i]).toBeNaN();
                    continue;
                }
                const want = expected / 1e6;
                const scale = Math.max(Math.abs(want), Math.abs(output[i]), 1e-3);
                expect(Math.abs(output[i] - want)).toBeLessThanOrEqual(1e-9 * scale);
            }
        });

        it(`${c.name}: the causal pipeline removes the interference and keeps the pulses`, () => {
            const plan: FilterPlan = planFilters(c.settings, c.fs);
            const q = quality(c, (volts) => filterCausal(volts, plan));
            expect(q.reductionDb).toBeGreaterThanOrEqual(25);
            for (const kept of q.pulseKept) expect(Math.abs(kept - 1)).toBeLessThan(0.15);
            if (plan.despikeWindow > 0) {
                expect(q.spikeCount).toBeGreaterThan(50);
                expect(q.spikeDb).toBeGreaterThanOrEqual(20);
            }
        });
    }
});
