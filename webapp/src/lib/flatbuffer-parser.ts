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
            const input = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
            const bytes =
                input.byteOffset === 0 && input.byteLength === input.buffer.byteLength
                    ? input
                    : new Uint8Array(input);
            const bb = new flatbuffers.ByteBuffer(bytes);
            const scan = Scan.getRootAsScan(bb);

            const valuesLength = scan.valuesLength();
            if (valuesLength === 0) {
                console.warn('No values found in FlatBuffer');
                return null;
            }
            const values = new Float64Array(valuesLength);
            for (let i = 0; i < valuesLength; i++) {
                values[i] = scan.values(i) ?? Number.NaN;
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

function extractValues(scan: Scan): Float64Array | null {
    try {
        const direct = scan.valuesArray();
        if (direct && direct.length > 0) {
            return direct;
        }
    } catch (error) {
        // NATS websocket payloads can arrive as Uint8Array slices whose byteOffset
        // is not 8-byte aligned, which breaks the generated Float64Array view.
        console.warn('Falling back to scalar FlatBuffer decode for misaligned payload.', error);
    }

    const length = scan.valuesLength();
    if (!Number.isFinite(length) || length <= 0) return null;

    const values = new Float64Array(length);
    for (let i = 0; i < length; i++) {
        const value = scan.values(i);
        if (typeof value !== 'number' || !Number.isFinite(value)) {
            return null;
        }
        values[i] = value;
    }

    return values;
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
