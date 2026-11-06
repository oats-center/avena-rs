/**
 * Decoder for the FlatBuffer `Scan` messages on live LabJack subjects.
 *
 * The streamer publishes one `Scan` per channel per read on
 * `avenars.<site>.<box>.<source>.live.chNN` (schema in `rust-ljm/src/data.fbs`,
 * bindings generated into `sampler/`). Each holds raw volts for one channel plus
 * the time of the first value and the interval between values. This module turns a
 * payload into {@link ScanData} and computes per-sample times.
 *
 * @module
 */
import * as flatbuffers from 'flatbuffers';
import { Scan } from './sampler.js';

/** One decoded FlatBuffer `Scan`: a batch of samples from one channel. */
export interface ScanData {
    /** Time of `values[0]`, Unix epoch, in nanoseconds. */
    firstSampleUnixNs: bigint;
    /** Time between consecutive values, in nanoseconds. */
    sampleIntervalNs: bigint;
    /** Scan rate the LabJack reported, in Hz. */
    actualScanRateHz: number;
    /** Batch counter; starts at 0 when the stream starts. A gap means lost messages. */
    sequence: bigint;
    /** Raw readings for one channel, in volts. `NaN` marks a skipped sample. */
    values: Float64Array;
}

/**
 * vtable slot of the `values` field of `Scan` (fifth field: `4 + 2 * 4`), as used by
 * the generated `values()` and `valuesArray()` accessors.
 */
const SCAN_VALUES_VTABLE_OFFSET = 12;

/**
 * `true` on little-endian hosts (every current browser platform). FlatBuffers stores
 * floats little-endian, so the `values` vector can then be copied byte for byte.
 */
const HOST_IS_LITTLE_ENDIAN = new Uint8Array(new Uint16Array([1]).buffer)[0] === 1;

/**
 * Copies the `values` vector of a `Scan` into a new array.
 *
 * On a little-endian host the vector's bytes are copied in one go, whatever the
 * payload's alignment; otherwise each value is read with the generated scalar
 * accessor. Both give the same numbers.
 *
 * @param scan - Root `Scan` of `bb`.
 * @param bb - Buffer holding the payload.
 * @param length - `scan.valuesLength()`, greater than 0.
 * @returns The values, in order.
 */
function copyValues(scan: Scan, bb: flatbuffers.ByteBuffer, length: number): Float64Array {
    const values = new Float64Array(length);
    if (HOST_IS_LITTLE_ENDIAN) {
        const start = bb.__vector(scan.bb_pos + bb.__offset(scan.bb_pos, SCAN_VALUES_VTABLE_OFFSET));
        const bytes = bb.bytes();
        if (start >= 0 && start + length * 8 <= bytes.length) {
            new Uint8Array(values.buffer).set(bytes.subarray(start, start + length * 8));
            return values;
        }
    }
    for (let i = 0; i < length; i++) {
        values[i] = scan.values(i) ?? Number.NaN;
    }
    return values;
}

/**
 * Converts a Unix nanosecond timestamp to JavaScript milliseconds.
 *
 * @param timestampNs - Unix epoch time in nanoseconds.
 * @returns Unix epoch time in milliseconds, with the sub-millisecond part kept as a
 *   fraction. Converting to a `number` first limits precision to about 256 ns for
 *   current dates.
 */
function nsToMs(timestampNs: bigint): number {
    return Number(timestampNs) / 1_000_000;
}

/** Stateless decoder for FlatBuffer `Scan` payloads received over NATS. */
export class FlatBufferParser {
    /**
     * Decodes a FlatBuffer `Scan` payload.
     *
     * The input may be a view onto part of a larger buffer, as NATS WebSocket payloads
     * are; it is decoded in place, without copying the payload. The values are copied
     * out in one block (see `copyValues`) instead of through the generated
     * `Float64Array` view, which fails on payloads that are not 8-byte aligned.
     *
     * The payload is not verified against the schema. Errors are logged to the
     * console.
     *
     * @param buffer - Message payload bytes.
     * @returns The decoded scan, or `null` if it has no values or decoding throws.
     *
     * @example
     * ```ts
     * const parser = new FlatBufferParser();
     * const scan = parser.parse(msg.data);
     * if (scan) console.log(scan.values.length, scan.firstSampleUnixNs);
     * ```
     */
    parse(buffer: ArrayBuffer | Uint8Array): ScanData | null {
        try {
            // ByteBuffer offsets are relative to the view, so a slice of a larger
            // buffer decodes in place.
            const bytes = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
            const bb = new flatbuffers.ByteBuffer(bytes);
            const scan = Scan.getRootAsScan(bb);

            const valuesLength = scan.valuesLength();
            if (valuesLength === 0) {
                console.warn('No values found in FlatBuffer');
                return null;
            }
            const values = copyValues(scan, bb, valuesLength);

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
 * Tries the generated `Float64Array` view first. That view fails on payloads whose
 * byte offset is not 8-byte aligned, so on an error or an empty result this falls
 * back to reading values one at a time.
 *
 * @remarks
 * Not called anywhere at present; {@link FlatBufferParser.parse} reads values
 * itself. The scalar fallback returns `null` on any non-finite value, so a batch
 * containing a skipped (`NaN`) sample yields `null` there, while the direct view
 * returns it as is.
 *
 * @param scan - Decoded `Scan` table.
 * @returns The values, or `null` if there are none or the fallback finds a
 *   non-finite value.
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
 * Calculates the time of every value in a scan batch.
 *
 * The time of `values[i]` is `firstSampleUnixNs + i * sampleIntervalNs`, computed
 * in `bigint` and then converted to milliseconds.
 *
 * @param firstSampleUnixNs - Time of `values[0]`, Unix epoch, in nanoseconds.
 * @param sampleIntervalNs - Time between consecutive values, in nanoseconds.
 * @param valueCount - Number of values in the batch.
 * @returns Unix epoch times in milliseconds (with fractions), one per value, or an
 *   empty array if `valueCount` is not a positive finite number.
 *
 * @example
 * ```ts
 * const times = calculateSourceSampleTimestamps(
 *   scan.firstSampleUnixNs, scan.sampleIntervalNs, scan.values.length);
 * ```
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
