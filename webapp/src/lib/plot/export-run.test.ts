import { describe, expect, it } from 'vitest';
import { isExportCancelled, type ExportStreamResult } from '../exporter';
import type { NatsService } from '../nats.svelte';
import { runExportOnOwnConnection } from './export-run';

const payload = { asset: 1, channels: [0], start: '2026-01-01T00:00:00Z', end: '2026-01-01T00:01:00Z' };
const result: ExportStreamResult = { blob: new Blob(['a']), streamed: false, fileName: 'x.csv', size: 1, missingChannels: [] };

function fakeService() {
    const state = { closed: 0 };
    const service = { connection: { close: async () => { state.closed += 1; } } } as unknown as NatsService;
    return { service, state };
}

describe('runExportOnOwnConnection', () => {
    it('downloads on its own connection and closes it afterwards', async () => {
        const { service, state } = fakeService();
        let used: NatsService | null = null;
        const out = await runExportOnOwnConnection({
            openConnection: async () => service,
            subject: 'req',
            payload,
            signal: new AbortController().signal,
            download: async (nats) => { used = nats; expect(state.closed).toBe(0); return result; }
        });
        expect(out).toBe(result);
        expect(used).toBe(service);
        expect(state.closed).toBe(1);
    });

    it('closes the connection when the download fails', async () => {
        const { service, state } = fakeService();
        const err = await runExportOnOwnConnection({
            openConnection: async () => service,
            subject: 'req',
            payload,
            signal: new AbortController().signal,
            download: async () => { throw new Error('boom'); }
        }).catch((e) => e);
        expect(err.message).toBe('boom');
        expect(state.closed).toBe(1);
    });

    it('closes the connection and reports a cancel when aborted while connecting', async () => {
        const { service, state } = fakeService();
        const controller = new AbortController();
        let downloaded = false;
        const err = await runExportOnOwnConnection({
            openConnection: async () => { controller.abort(); return service; },
            subject: 'req',
            payload,
            signal: controller.signal,
            download: async () => { downloaded = true; return result; }
        }).catch((e) => e);
        expect(isExportCancelled(err)).toBe(true);
        expect(downloaded).toBe(false);
        expect(state.closed).toBe(1);
    });

    it('does not connect when already cancelled and reports a failed connection', async () => {
        const controller = new AbortController();
        controller.abort();
        let opened = false;
        const cancelled = await runExportOnOwnConnection({
            openConnection: async () => { opened = true; return null; },
            subject: 'req', payload, signal: controller.signal
        }).catch((e) => e);
        expect(isExportCancelled(cancelled)).toBe(true);
        expect(opened).toBe(false);

        const failed = await runExportOnOwnConnection({
            openConnection: async () => null,
            subject: 'req', payload, signal: new AbortController().signal
        }).catch((e) => e);
        expect(failed.message).toBe('Failed to connect to NATS server for the export');
    });

    it('passes the chunk consumer on to the download', async () => {
        const { service } = fakeService();
        const onChunk = async () => {};
        let passed: unknown = null;
        await runExportOnOwnConnection({
            openConnection: async () => service,
            subject: 'req', payload, signal: new AbortController().signal,
            onChunk,
            download: async (_nats, _subject, _payload, opts) => { passed = opts?.onChunk; return result; }
        });
        expect(passed).toBe(onChunk);
    });
});
