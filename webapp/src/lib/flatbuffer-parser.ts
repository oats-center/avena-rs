import * as flatbuffers from 'flatbuffers';
import { Scan } from './sampler.js';

/** Decoded LabJack scan batch from the generated FlatBuffer schema. */
export interface ScanData {
    /** Unix nanosecond timestamp for the first sample in the batch. */
    firstSampleUnixNs: bigint;
    /** Fixed interval between samples in the batch, in nanoseconds. */
    sampleIntervalNs: bigint;
    /** Actual scan rate reported by the LabJack stream. */
    actualScanRateHz: number;
    /** Monotonic per-run sequence number assigned by the streamer. */
    sequence: bigint;
    /** Channel values contained in this scan batch. */
    values: Float64Array;
}

/** Converts a Unix nanosecond timestamp to JavaScript milliseconds. */
function nsToMs(timestampNs: bigint): number {
    return Number(timestampNs) / 1_000_000;
}

/** Parser for streamer FlatBuffer scan payloads received over NATS. */
export class FlatBufferParser {
    /**
     * Decodes a FlatBuffer scan payload into plot-ready sample data.
     *
     * Returns `null` when the payload cannot be decoded or has no values.
     */
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

/**
 * Extracts sample values from a generated `Scan` object.
 *
 * The generated vector view is fast but can fail on misaligned websocket
 * buffers, so this falls back to scalar access when needed.
 */
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

/**
 * Calculates JavaScript millisecond timestamps for every value in a scan batch.
 */
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
