import { describe, expect, it } from 'vitest';
import { downloadExportViaNats, isExportCancelled } from '../exporter';
import type { NatsService } from '../nats.svelte';

/** Minimal stand-in for a NATS connection: one reply subscription fed by the test. */
function fakeNats() {
    const queued: unknown[] = [];
    let wake: (() => void) | null = null;
    let closed = false;
    const published: string[] = [];
    const sub = {
        unsubscribed: false,
        unsubscribe() {
            this.unsubscribed = true;
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
        publish: (subject: string) => published.push(subject),
        flush: async () => {}
    };
    const chunk = (bytes: number[]) => {
        queued.push({ data: new Uint8Array(bytes), headers: { get: () => 'chunk' } });
        wake?.();
    };
    return { nats: { connection } as unknown as NatsService, sub, published, chunk };
}

const payload = { asset: 1, channels: [0], start: '2026-01-01T00:00:00Z', end: '2026-01-01T00:01:00Z' };

describe('export cancel', () => {
    it('stops reading, stops acking and releases the subscription when aborted', async () => {
        const { nats, sub, published, chunk } = fakeNats();
        const controller = new AbortController();
        let received = 0;
        const download = downloadExportViaNats(nats, 'req', payload, {
            signal: controller.signal,
            onProgress: (bytes) => (received = bytes)
        });
        chunk([1, 2, 3]);
        await new Promise((resolve) => setTimeout(resolve, 0));
        expect(received).toBe(3);
        const acksBefore = published.length;

        controller.abort();
        chunk([4, 5]);
        const err = await download.catch((e) => e);
        expect(isExportCancelled(err)).toBe(true);
        expect(sub.unsubscribed).toBe(true);
        expect(received).toBe(3);
        // No more acks; only the cancel message on the ack subject.
        expect(published.length).toBe(acksBefore + 1);
        expect(published[published.length - 1]).toBe(published[1]);
    });

    it('does not send the request when already aborted', async () => {
        const { nats, published } = fakeNats();
        const controller = new AbortController();
        controller.abort();
        const err = await downloadExportViaNats(nats, 'req', payload, { signal: controller.signal }).catch((e) => e);
        expect(isExportCancelled(err)).toBe(true);
        expect(published).toEqual([]);
    });
});
