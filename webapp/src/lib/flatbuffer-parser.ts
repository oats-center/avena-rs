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
    samplingRate: number
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
        
        // For real-time plotting, use current time as baseline
        // The FlatBuffer timestamp might be from when data was collected, not received
        const now = Date.now();
        const batchTime = new Date(batchTimestamp).getTime();
        
        if (isNaN(batchTime)) {
            throw new Error('Failed to parse timestamp');
        }
        
        // Calculate time between individual samples (in milliseconds)
        // samplingRate is the total sampling frequency, so time between samples is 1000/samplingRate ms
        // e.g., 7000 Hz = 0.143ms between samples
        const sampleInterval = 1000 / samplingRate;
        
        // Generate timestamps for each sample within this batch
        // The samples in the FlatBuffer are consecutive samples taken at sampleInterval intervals
        // For real-time plotting, we want the most recent sample to be at "now"
        const timestamps: number[] = [];
        for (let i = 0; i < values.length; i++) {
            // Calculate timestamp for each sample within the batch
            // Start from current time and go backwards for each sample (most recent first)
            const timestamp = now - ((values.length - 1 - i) * sampleInterval);
            if (isNaN(timestamp)) {
                throw new Error('Invalid timestamp calculated');
            }
            timestamps.push(timestamp);
        }
        
        return timestamps;
    } catch (error) {
        // Fallback to current time with proper sample intervals
        const now = Date.now();
        const sampleInterval = 1000 / samplingRate;
        return values.map((_, i) => now + (i * sampleInterval));
    }
}