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

describe('export onChunk hook', () => {
    it('awaits onChunk before acknowledging each chunk', async () => {
        const fake = fakeNats();
        const events: string[] = [];
        const acks = () => fake.published.filter((p) => p.subject !== 'req' && !p.frame).length;
        const download = downloadExportViaNats(fake.nats, 'req', payload, {
            onChunk: async (data) => {
                events.push(`start ${data.length} acks=${acks()}`);
                await new Promise((resolve) => setTimeout(resolve, 5));
                events.push(`done ${data.length} acks=${acks()}`);
            },
            onProgress: (bytes) => events.push(`progress ${bytes} acks=${acks()}`)
        });
        fake.send('chunk', new Uint8Array([1, 2]));
        fake.send('chunk', new Uint8Array([3]));
        fake.json('complete', { type: 'complete' });
        await download;
        expect(events).toEqual([
            'start 2 acks=0',
            'done 2 acks=0',
            'progress 2 acks=1',
            'start 1 acks=1',
            'done 1 acks=1',
            'progress 3 acks=2'
        ]);
    });

    it('keeps no Blob when onChunk takes the chunks', async () => {
        const fake = fakeNats();
        const received: number[][] = [];
        const download = downloadExportViaNats(fake.nats, 'req', payload, {
            onChunk: (data) => {
                received.push([...data]);
            }
        });
        fake.json('meta', { type: 'meta', fileName: 'x.csv', contentType: 'text/csv' });
        fake.send('chunk', new Uint8Array([1, 2, 3]));
        fake.send('chunk', new Uint8Array([4]));
        fake.json('summary', { type: 'summary', bytesSent: 4, missingChannels: [2] });
        fake.json('complete', { type: 'complete' });
        const result = await download;
        expect(received).toEqual([[1, 2, 3], [4]]);
        expect(result.streamed).toBe(true);
        expect(result.blob.size).toBe(0);
        expect(result.fileName).toBe('x.csv');
        expect(result.size).toBe(4);
        expect(result.missingChannels).toEqual([2]);
        expect(fake.cancels()).toEqual([]);
    });

    it('cancels the exporter and rejects with the onChunk error', async () => {
        const fake = fakeNats();
        const download = downloadExportViaNats(fake.nats, 'req', payload, {
            onChunk: async () => {
                throw new Error('disk full');
            }
        });
        fake.send('chunk', new Uint8Array([1]));
        await expect(download).rejects.toThrow('disk full');
        const toAck = fake.published.filter((p) => p.subject === fake.ackSubject());
        expect(toAck.map((p) => p.frame ?? 'ack')).toEqual(['cancel']);
    });

    it('collects the chunks into the Blob without onChunk', async () => {
        const fake = fakeNats();
        const download = downloadExportViaNats(fake.nats, 'req', payload);
        fake.json('meta', { type: 'meta', fileName: 'x.csv', contentType: 'text/plain' });
        fake.send('chunk', new TextEncoder().encode('a,b\n'));
        fake.send('chunk', new TextEncoder().encode('1,2\n'));
        fake.json('complete', { type: 'complete' });
        const result = await download;
        expect(result.streamed).toBe(false);
        expect(result.blob.type).toBe('text/plain');
        expect(await result.blob.text()).toBe('a,b\n1,2\n');
        expect(result.size).toBe(8);
        const toAck = fake.published.filter((p) => p.subject === fake.ackSubject());
        expect(toAck.map((p) => p.frame ?? 'ack')).toEqual(['ack', 'ack']);
    });
});
