/**
 * Test helpers that build FlatBuffer `Scan` payloads the way the Rust streamer does
 * (`rust-ljm/src/main.rs`, `sample_with_config`): one `Scan` per channel per read,
 * `first_sample_unix_ns` continuing from the previous batch, a fixed
 * `sample_interval_ns = round(1e9 / rate)`, and a sequence number per read.
 *
 * Only imported by `*.test.ts` files.
 *
 * @module
 */
import * as flatbuffers from 'flatbuffers';
import { Scan } from '../sampler/scan';

/** Fields of one `Scan` message. */
export interface ScanFields {
    firstSampleUnixNs: bigint;
    sampleIntervalNs: bigint;
    actualScanRateHz: number;
    sequence: bigint;
    values: number[];
}

/**
 * Encodes a `Scan` with the generated TypeScript builder.
 *
 * @returns The payload as a view into a larger buffer, like NATS WebSocket payloads.
 */
export function encodeScan(fields: ScanFields): Uint8Array {
    const builder = new flatbuffers.Builder(64);
    const values = Scan.createValuesVector(builder, fields.values);
    const scan = Scan.createScan(
        builder,
        fields.firstSampleUnixNs,
        fields.sampleIntervalNs,
        fields.actualScanRateHz,
        fields.sequence,
        values
    );
    builder.finish(scan);
    return builder.asUint8Array();
}

/** Simulates the streamer's clock and batching for one sampling run. */
export class FakeStreamer {
    readonly intervalNs: bigint;
    private nextFirstNs: bigint;
    sequence = 0n;
    /** Global sample index per run, used to derive deterministic values. */
    sampleIndex = 0;

    /**
     * @param rateHz - Scan rate.
     * @param scansPerRead - Samples per channel per message.
     * @param startNs - First sample time of the run, Unix ns.
     * @param valueAt - Raw value of sample `k` of the run.
     */
    constructor(
        readonly rateHz: number,
        readonly scansPerRead: number,
        startNs: bigint = 1_790_000_000_000_000_000n,
        readonly valueAt: (k: number) => number = (k) => Math.sin(k / 10)
    ) {
        this.intervalNs = BigInt(Math.round(1e9 / rateHz));
        this.nextFirstNs = startNs;
    }

    /** Time of sample `k` of this run, Unix ms, as the page computes it. */
    timeOfSampleMs(firstNs: bigint, i: number): number {
        return Number(firstNs + this.intervalNs * BigInt(i)) / 1e6;
    }

    /**
     * Produces the next read's `Scan` for one channel.
     *
     * @param slewNs - Adjustment added to this batch's first sample time, as the
     *   streamer's clock slew does (at most half an interval).
     */
    nextBatch(slewNs: bigint = 0n): { fields: ScanFields; payload: Uint8Array } {
        const firstNs = this.nextFirstNs + slewNs;
        const values: number[] = [];
        for (let i = 0; i < this.scansPerRead; i++) values.push(this.valueAt(this.sampleIndex + i));
        const fields: ScanFields = {
            firstSampleUnixNs: firstNs,
            sampleIntervalNs: this.intervalNs,
            actualScanRateHz: this.rateHz,
            sequence: this.sequence,
            values
        };
        this.sampleIndex += this.scansPerRead;
        this.sequence += 1n;
        this.nextFirstNs = firstNs + this.intervalNs * BigInt(this.scansPerRead);
        return { fields, payload: encodeScan(fields) };
    }
}
