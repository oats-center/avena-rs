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

// Helper function to parse timestamp and calculate individual sample timestamps
export function calculateSampleTimestamps(
    batchTimestamp: string, 
    values: number[], 
    samplingRate: number,
    previousLastTimestamp?: number
): number[] {
    try {
        // Validate inputs
        if (!batchTimestamp || typeof batchTimestamp !== 'string') {
            throw new Error('Invalid batchTimestamp');
        }
        
        if (!Array.isArray(values) || values.length === 0) {
            throw new Error('Invalid values array');
        }
        
        if (typeof samplingRate !== 'number' || samplingRate <= 0) {
            throw new Error('Invalid samplingRate');
        }

        // Time between individual samples (ms). Example: 7000 Hz ~= 0.143 ms/sample.
        const sampleInterval = 1000 / samplingRate;

        // Anchor the batch to producer timestamp when possible.
        // If producer/browser clocks drift too much, fall back to local time.
        const now = Date.now();
        const parsedBatchTime = new Date(batchTimestamp).getTime();
        const hasParsedBatchTime = Number.isFinite(parsedBatchTime);
        const isReasonableSkew = hasParsedBatchTime
            ? Math.abs(parsedBatchTime - now) <= 2000
            : false;
        const hasPrevious = typeof previousLastTimestamp === 'number' && Number.isFinite(previousLastTimestamp);

        let firstSampleTime = isReasonableSkew
            ? parsedBatchTime - ((values.length - 1) * sampleInterval)
            : now - ((values.length - 1) * sampleInterval);

        // Guarantee monotonic timestamps across received batches for each channel.
        if (hasPrevious) {
            const minNextStart = (previousLastTimestamp as number) + sampleInterval;
            if (firstSampleTime < minNextStart) {
                firstSampleTime = minNextStart;
            }
        }

        return values.map((_, i) => firstSampleTime + (i * sampleInterval));
    } catch (error) {
        // Fallback: continue from prior point when available.
        const sampleInterval = 1000 / samplingRate;
        const start = (typeof previousLastTimestamp === 'number' && Number.isFinite(previousLastTimestamp))
            ? previousLastTimestamp + sampleInterval
            : Date.now() - ((values.length - 1) * sampleInterval);
        return values.map((_, i) => start + (i * sampleInterval));
    }
}
