import { describe, expect, it } from 'vitest';
import { downloadExportViaNats } from '../exporter';
import type { NatsService } from '../nats.svelte';
import { isChunkFrame, isPickerCancelled, pickSaveFile, suggestedExportFileName, tapExportChunks } from './export-sink';

/** Fake connection whose one reply subscription is fed by the test, logging publishes. */
function fakeNats() {
    const queued: unknown[] = [];
    let wake: (() => void) | null = null;
    let closed = false;
    const log: string[] = [];
    const sub = {
        unsubscribe() {
            closed = true;
            wake?.();
        },
        async *[Symbol.asyncIterator]() {
            while (true) {
                if (queued.length > 0) {
                    yield queued.shift();
                    continue;
                }
                if (closed) return;
                await new Promise<void>((resolve) => (wake = resolve));
                wake = null;
            }
        }
    };
    const connection = {
        subscribe: () => sub,
        publish: (subject: string) => log.push(`publish ${subject.startsWith('_INBOX') ? 'ack' : subject}`),
        flush: async () => {}
    };
    const frame = (name: string | null, data: Uint8Array) => {
        queued.push({ data, headers: name === null ? undefined : { get: (h: string) => (h === 'Avena-Export-Frame' ? name : '') } });
        wake?.();
    };
    const json = (name: string, body: object) => frame(name, new TextEncoder().encode(JSON.stringify(body)));
    return { nats: { connection } as unknown as NatsService, log, frame, json };
}

const payload = { asset: 7, channels: [0], start: '2026-09-28T12:00:00Z', end: '2026-09-28T12:02:00Z' };

describe('tapExportChunks', () => {
    it('writes every chunk in order before it is acknowledged', async () => {
        const { nats, log, frame, json } = fakeNats();
        const written: number[][] = [];
        const tapped = tapExportChunks(nats, async (data) => {
            await new Promise((resolve) => setTimeout(resolve, 5));
            written.push([...data]);
            log.push(`write ${data.length}`);
        });
        const download = downloadExportViaNats(tapped, 'req', payload);
        json('meta', { type: 'meta', fileName: 'x.csv' });
        frame('chunk', new Uint8Array([1, 2, 3]));
        frame(null, new Uint8Array([4]));
        json('summary', { type: 'summary', bytesSent: 4 });
        json('complete', { type: 'complete' });
        const result = await download;
        expect(written).toEqual([[1, 2, 3], [4]]);
        expect(log).toEqual(['publish req', 'write 3', 'publish ack', 'write 1', 'publish ack']);
        expect(result.size).toBe(4);
        expect(result.fileName).toBe('x.csv');
    });

    it('fails the export when a write fails', async () => {
        const { nats, frame } = fakeNats();
        const tapped = tapExportChunks(nats, async () => {
            throw new Error('disk full');
        });
        const download = downloadExportViaNats(tapped, 'req', payload);
        frame('chunk', new Uint8Array([1]));
        await expect(download).rejects.toThrow('disk full');
    });
});

describe('export sink helpers', () => {
    it('recognises chunk frames', () => {
        const headers = (name: string, extra: object = {}) => ({ get: () => name, ...extra });
        expect(isChunkFrame({})).toBe(true);
        expect(isChunkFrame({ headers: headers('') })).toBe(true);
        expect(isChunkFrame({ headers: headers('chunk') })).toBe(true);
        expect(isChunkFrame({ headers: headers('meta') })).toBe(false);
        expect(isChunkFrame({ headers: headers('', { code: 503, hasError: true }) })).toBe(false);
    });

    it('suggests the request name or the exporter default', () => {
        expect(suggestedExportFileName({ ...payload, download_name: 'LJ2.csv' })).toBe('LJ2.csv');
        expect(suggestedExportFileName(payload)).toBe('labjack_asset007_20260928T120000_20260928T120200.csv');
    });

    it('opens the save dialog only where the browser has one', async () => {
        expect(pickSaveFile('a.csv', {})).toBeNull();
        let options: unknown = null;
        const handle = { createWritable: async () => ({}) };
        const scope = { showSaveFilePicker: async (o: unknown) => { options = o; return handle; } };
        await expect(pickSaveFile('a.csv', scope)).resolves.toBe(handle);
        expect(options).toMatchObject({ suggestedName: 'a.csv' });
        const abort = new Error('x');
        abort.name = 'AbortError';
        expect(isPickerCancelled(abort)).toBe(true);
        expect(isPickerCancelled(new Error('x'))).toBe(false);
    });
});
