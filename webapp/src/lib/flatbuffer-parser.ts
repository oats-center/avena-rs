import * as flatbuffers from 'flatbuffers';
import { Scan } from './sampler.js';

export interface ScanData {
    firstSampleUnixNs: bigint;
    sampleIntervalNs: bigint;
    actualScanRateHz: number;
    sequence: bigint;
    values: Float64Array;
}

function nsToMs(timestampNs: bigint): number {
    return Number(timestampNs) / 1_000_000;
}

export class FlatBufferParser {
    parse(buffer: ArrayBuffer | Uint8Array): ScanData | null {
        try {
            const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
            const bb = new flatbuffers.ByteBuffer(bytes);
            const scan = Scan.getRootAsScan(bb);

            const values = scan.valuesArray();
            if (!values || values.length === 0) {
                console.warn('No values found in FlatBuffer');
                return null;
            }

            return {
                firstSampleUnixNs: scan.firstSampleUnixNs(),
                sampleIntervalNs: scan.sampleIntervalNs(),
                actualScanRateHz: scan.actualScanRateHz(),
                sequence: scan.sequence(),
                values
            };
        } catch (error) {
            console.error('FlatBuffer parsing error:', error);
            return null;
        }
    }
}

export function calculateSourceSampleTimestamps(
    firstSampleUnixNs: bigint,
    sampleIntervalNs: bigint,
    valueCount: number
): number[] {
    if (!Number.isFinite(valueCount) || valueCount <= 0) return [];

    const timestamps: number[] = [];
    for (let i = 0; i < valueCount; i++) {
        const timestampNs = firstSampleUnixNs + (sampleIntervalNs * BigInt(i));
        timestamps.push(nsToMs(timestampNs));
    }
    return timestamps;
}
