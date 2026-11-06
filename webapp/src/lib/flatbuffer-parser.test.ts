import { describe, expect, it } from 'vitest';
import { FlatBufferParser } from './flatbuffer-parser';
import { encodeScan } from './plot/test-helpers';

const fields = {
    firstSampleUnixNs: 1_790_000_000_123_456_789n,
    sampleIntervalNs: 1_000_000n,
    actualScanRateHz: 1000,
    sequence: 42n,
    values: [0, -0, 1.5, -9999, Number.NaN, 1e-300, -3.25, Number.MAX_VALUE]
};

/** Copies `payload` into a larger buffer at `offset`, as a view, like a NATS frame slice. */
function viewAt(payload: Uint8Array, offset: number): Uint8Array {
    const backing = new Uint8Array(payload.length + offset + 5);
    backing.set(payload, offset);
    return backing.subarray(offset, offset + payload.length);
}

describe('FlatBufferParser', () => {
    it('decodes the same scan from every alignment of the payload', () => {
        const parser = new FlatBufferParser();
        const payload = encodeScan(fields);
        for (let offset = 0; offset < 16; offset++) {
            const scan = parser.parse(viewAt(payload, offset));
            expect(scan).not.toBeNull();
            expect(scan!.firstSampleUnixNs).toBe(fields.firstSampleUnixNs);
            expect(scan!.sampleIntervalNs).toBe(fields.sampleIntervalNs);
            expect(scan!.actualScanRateHz).toBe(fields.actualScanRateHz);
            expect(scan!.sequence).toBe(fields.sequence);
            expect(Array.from(scan!.values)).toEqual(fields.values);
            expect(Object.is(scan!.values[1], -0)).toBe(true);
        }
    });

    it('decodes a whole ArrayBuffer and leaves the input untouched', () => {
        const payload = encodeScan(fields);
        const copy = payload.slice();
        const scan = new FlatBufferParser().parse(copy.buffer);
        expect(Array.from(scan!.values)).toEqual(fields.values);
        expect(copy).toEqual(payload);
    });

    it('returns values that do not alias the payload', () => {
        const payload = viewAt(encodeScan(fields), 3);
        const scan = new FlatBufferParser().parse(payload)!;
        payload.fill(0);
        expect(Array.from(scan.values)).toEqual(fields.values);
    });

    it('returns null for a scan without values', () => {
        const scan = new FlatBufferParser().parse(encodeScan({ ...fields, values: [] }));
        expect(scan).toBeNull();
    });
});
