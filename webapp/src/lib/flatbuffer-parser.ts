import * as flatbuffers from 'flatbuffers';
import { Scan } from './sampler.js';

export interface ScanData {
    firstSampleUnixNs: bigint;
    sampleIntervalNs: bigint;
    actualScanRateHz: number;
    sequence: bigint;
    values: number[];
}

function nsToMs(timestampNs: bigint): number {
    return Number(timestampNs) / 1_000_000;
}

export class FlatBufferParser {
    parse(buffer: ArrayBuffer): ScanData | null {
        try {
            const bb = new flatbuffers.ByteBuffer(new Uint8Array(buffer));
            const scan = Scan.getRootAsScan(bb);

            const valuesArray = scan.valuesArray();
            const values = valuesArray ? Array.from(valuesArray) : [];
            if (values.length === 0) {
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
