// FlatBuffer parser for LabJack data using generated TypeScript bindings
import * as flatbuffers from 'flatbuffers';
import { Scan } from './scan.js';

export interface ScanData {
    timestamp: string;
    values: number[];
}

export class FlatBufferParser {
    parse(buffer: ArrayBuffer): ScanData | null {
        try {
            // Create ByteBuffer from ArrayBuffer
            const bb = new flatbuffers.ByteBuffer(new Uint8Array(buffer));
            
            // Parse using generated Scan class
            const scan = Scan.getRootAsScan(bb);
            
            // Extract timestamp
            const timestamp = scan.timestamp();
            if (!timestamp) {
                console.warn('No timestamp found in FlatBuffer');
                return null;
            }
            
            // Extract values array
            const valuesLength = scan.valuesLength();
            if (valuesLength === 0) {
                console.warn('No values found in FlatBuffer');
                return null;
            }
            
            const values: number[] = [];
            for (let i = 0; i < valuesLength; i++) {
                const value = scan.values(i);
                if (value !== null) {
                    values.push(value);
                }
            }
            
            return { timestamp, values };
            
        } catch (error) {
            console.error('FlatBuffer parsing error:', error);
            return null;
        }
    }
}

function getSampleIntervalMs(samplingRate: number): number {
    if (typeof samplingRate !== 'number' || !Number.isFinite(samplingRate) || samplingRate <= 0) {
        throw new Error('Invalid samplingRate');
    }
    return 1000 / samplingRate;
}

function hasNumericSamples(values: number[]): boolean {
    return Array.isArray(values) && values.length > 0;
}

// Build plot timestamps anchored to browser receive time.
// This keeps waveform spacing accurate while allowing transport delay.
export function calculateReceiveSampleTimestamps(
    values: number[],
    samplingRate: number,
    receivedAt: number = Date.now()
): number[] {
    if (!hasNumericSamples(values)) return [];

    try {
        const sampleInterval = getSampleIntervalMs(samplingRate);
        const firstSampleTime = receivedAt - ((values.length - 1) * sampleInterval);
        return values.map((_, i) => firstSampleTime + (i * sampleInterval));
    } catch {
        const fallbackInterval = Number.isFinite(samplingRate) && samplingRate > 0
            ? 1000 / samplingRate
            : 1;
        const start = receivedAt - ((values.length - 1) * fallbackInterval);
        return values.map((_, i) => start + (i * fallbackInterval));
    }
}

// Build source timestamps from producer batch timestamp for lag visibility.
export function calculateSourceSampleTimestamps(
    batchTimestamp: string,
    values: number[],
    samplingRate: number
): Array<number | null> {
    if (!hasNumericSamples(values)) return [];

    try {
        const sampleInterval = getSampleIntervalMs(samplingRate);
        const parsedBatchTime = new Date(batchTimestamp).getTime();
        if (!Number.isFinite(parsedBatchTime)) {
            return values.map(() => null);
        }

        const firstSampleTime = parsedBatchTime - ((values.length - 1) * sampleInterval);
        return values.map((_, i) => firstSampleTime + (i * sampleInterval));
    } catch {
        return values.map(() => null);
    }
}
