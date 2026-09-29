import { describe, expect, it } from 'vitest';
import { downloadExportViaNats, isExportCancelled } from './exporter';
import type { NatsService } from './nats.svelte';

type Published = { subject: string; data?: Uint8Array; frame?: string; reply?: string };

/** Fake NATS connection: one reply subscription fed by the test, publishes recorded. */
function fakeNats() {
    const queued: unknown[] = [];
    let wake: (() => void) | null = null;
    let closed = false;
    const published: Published[] = [];
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
        publish: (
            subject: string,
            data?: Uint8Array,
            opts?: { reply?: string; headers?: { get(k: string): string } }
        ) =>
            published.push({
                subject,
                data,
                reply: opts?.reply,
                frame: opts?.headers?.get('Avena-Export-Frame') || undefined
            }),
        flush: async () => {}
    };
    const send = (frame: string, body: Uint8Array) => {
        queued.push({ data: body, headers: { get: () => frame } });
        wake?.();
    };
    const json = (frame: string, value: unknown) =>
        send(frame, new TextEncoder().encode(JSON.stringify(value)));
    const ackSubject = () => {
        const request = JSON.parse(new TextDecoder().decode(published[0].data));
        return request.ack_subject as string;
    };
    const cancels = () => published.filter((p) => p.frame === 'cancel');
    return { nats: { connection } as unknown as NatsService, published, send, json, ackSubject, cancels };
}

const payload = { asset: 1, channels: [0], start: '2026-01-01T00:00:00Z', end: '2026-01-01T00:01:00Z' };
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('export cancel message', () => {
    it('publishes one cancel on the ack subject when aborted', async () => {
        const fake = fakeNats();
        const controller = new AbortController();
        const download = downloadExportViaNats(fake.nats, 'req', payload, { signal: controller.signal });
        fake.json('meta', { type: 'meta', fileName: 'x.csv' });
        fake.send('chunk', new Uint8Array([1, 2]));
        await tick();
        controller.abort();
        fake.send('chunk', new Uint8Array([3]));
        expect(isExportCancelled(await download.catch((e) => e))).toBe(true);

        const acks = fake.published.filter((p) => p.subject === fake.ackSubject());
        expect(acks.map((p) => p.frame ?? 'ack')).toEqual(['ack', 'cancel']);
        const cancel = fake.cancels()[0];
        expect(JSON.parse(new TextDecoder().decode(cancel.data))).toEqual({ type: 'cancel' });
    });

    it('sends no cancel after a complete export', async () => {
        const fake = fakeNats();
        const download = downloadExportViaNats(fake.nats, 'req', payload);
        fake.send('chunk', new TextEncoder().encode('a,b\n'));
        fake.json('summary', { type: 'summary', bytesSent: 4, missingChannels: [] });
        fake.json('complete', { type: 'complete' });
        const result = await download;
        expect(result.size).toBe(4);
        expect(fake.cancels()).toEqual([]);
    });

    it('sends no cancel after an exporter error frame', async () => {
        const fake = fakeNats();
        const download = downloadExportViaNats(fake.nats, 'req', payload);
        fake.json('error', { type: 'error', message: 'boom' });
        await expect(download).rejects.toThrow('boom');
        expect(fake.cancels()).toEqual([]);
    });

    it('cancels the exporter when the client gives up on an idle timeout', async () => {
        const fake = fakeNats();
        const download = downloadExportViaNats(fake.nats, 'req', payload, { idleTimeoutMs: 20 });
        await expect(download).rejects.toThrow('Timed out');
        expect(fake.cancels().map((p) => p.subject)).toEqual([fake.ackSubject()]);
    });

    it('sends nothing when aborted before the request', async () => {
        const fake = fakeNats();
        const controller = new AbortController();
        controller.abort();
        const err = await downloadExportViaNats(fake.nats, 'req', payload, { signal: controller.signal }).catch(
            (e) => e
        );
        expect(isExportCancelled(err)).toBe(true);
        expect(fake.published).toEqual([]);
    });
});
