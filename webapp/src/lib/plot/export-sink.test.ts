import { describe, expect, it } from 'vitest';
import { downloadExportViaNats, isExportCancelled, type ExportStreamResult } from '../exporter';
import type { NatsService } from '../nats.svelte';
import {
    isPickerCancelled,
    pickSaveFile,
    runAndSaveExport,
    suggestedExportFileName,
    type SaveFileWritable
} from './export-sink';

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
        publish: (subject: string, _data?: Uint8Array, opts?: { headers?: unknown }) =>
            log.push(`publish ${subject.startsWith('_INBOX') ? (opts?.headers ? 'cancel' : 'ack') : subject}`),
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

/** Fake save-dialog file that records writes, close and abort in `log`. */
function fakeFile(log: string[], write?: (data: Uint8Array) => Promise<void>) {
    const written: number[][] = [];
    const file: SaveFileWritable = {
        async write(data) {
            if (write) await write(data);
            written.push([...data]);
            log.push(`write ${data.length}`);
        },
        async close() {
            log.push('close');
        },
        async abort() {
            log.push('abort');
        }
    };
    return { file, written };
}

describe('runAndSaveExport', () => {
    it('writes every chunk in order before it is acknowledged, then closes the file', async () => {
        const { nats, log, frame, json } = fakeNats();
        const { file, written } = fakeFile(log, () => new Promise((resolve) => setTimeout(resolve, 5)));
        const saved: string[] = [];
        const download = runAndSaveExport(
            file,
            (onChunk) => downloadExportViaNats(nats, 'req', payload, { onChunk }),
            (_blob, name) => saved.push(name)
        );
        json('meta', { type: 'meta', fileName: 'x.csv' });
        frame('chunk', new Uint8Array([1, 2, 3]));
        frame(null, new Uint8Array([4]));
        json('summary', { type: 'summary', bytesSent: 4 });
        json('complete', { type: 'complete' });
        const result = await download;
        expect(written).toEqual([[1, 2, 3], [4]]);
        expect(log).toEqual(['publish req', 'write 3', 'publish ack', 'write 1', 'publish ack', 'close']);
        expect(result.size).toBe(4);
        expect(result.fileName).toBe('x.csv');
        expect(result.streamed).toBe(true);
        expect(result.blob.size).toBe(0);
        expect(saved).toEqual([]);
    });

    it('cancels the exporter and discards the file when a write fails', async () => {
        const { nats, log, frame } = fakeNats();
        const { file } = fakeFile(log, async () => {
            throw new Error('disk full');
        });
        const download = runAndSaveExport(file, (onChunk) => downloadExportViaNats(nats, 'req', payload, { onChunk }));
        frame('chunk', new Uint8Array([1]));
        await expect(download).rejects.toThrow('disk full');
        expect(log).toEqual(['publish req', 'publish cancel', 'abort']);
    });

    it('discards the file when the export is cancelled', async () => {
        const { nats, log, frame } = fakeNats();
        const { file } = fakeFile(log);
        const controller = new AbortController();
        const download = runAndSaveExport(file, (onChunk) =>
            downloadExportViaNats(nats, 'req', payload, { onChunk, signal: controller.signal })
        );
        frame('chunk', new Uint8Array([1, 2]));
        await new Promise((resolve) => setTimeout(resolve, 0));
        controller.abort();
        expect(isExportCancelled(await download.catch((e) => e))).toBe(true);
        expect(log).toEqual(['publish req', 'write 2', 'publish ack', 'publish cancel', 'abort']);
    });

    it('saves the finished Blob through the fallback when there is no file', async () => {
        const { nats, log, frame, json } = fakeNats();
        const saved: { name: string; blob: Blob }[] = [];
        let hook: unknown = 'unset';
        const download = runAndSaveExport(
            null,
            (onChunk) => {
                hook = onChunk;
                return downloadExportViaNats(nats, 'req', payload, { onChunk });
            },
            (blob, name) => saved.push({ name, blob })
        );
        json('meta', { type: 'meta', fileName: 'x.csv' });
        frame('chunk', new TextEncoder().encode('a,b\n'));
        json('complete', { type: 'complete' });
        const result: ExportStreamResult = await download;
        expect(hook).toBeUndefined();
        expect(result.streamed).toBe(false);
        expect(saved.map((s) => s.name)).toEqual(['x.csv']);
        expect(saved[0].blob).toBe(result.blob);
        expect(await result.blob.text()).toBe('a,b\n');
        expect(log).toEqual(['publish req', 'publish ack']);
    });
});

describe('export sink helpers', () => {
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
