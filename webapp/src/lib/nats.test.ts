import { describe, expect, it } from 'vitest';
import { getKeyValues, type NatsService } from './nats.svelte';

/** Fake service whose bucket answers `get` after a tick, recording concurrency. */
function fakeService(values: Record<string, string | Error>) {
    let inFlight = 0;
    let maxInFlight = 0;
    let opens = 0;
    const kv = {
        async get(key: string) {
            inFlight++;
            maxInFlight = Math.max(maxInFlight, inFlight);
            await new Promise((resolve) => setTimeout(resolve, 1));
            inFlight--;
            const value = values[key];
            if (value instanceof Error) throw value;
            return value === undefined ? null : { string: () => value };
        }
    };
    const service = {
        bucket: async () => {
            opens++;
            return kv;
        }
    } as unknown as NatsService;
    return { service, maxInFlight: () => maxInFlight, opens: () => opens };
}

describe('getKeyValues', () => {
    it('returns one result per key, in key order, with failures kept apart', async () => {
        const failure = new Error('boom');
        const fake = fakeService({ a: '1', b: failure, c: '3' });
        const results = await getKeyValues(fake.service, 'avenabox', ['c', 'b', 'missing', 'a']);
        expect(results).toEqual([
            { status: 'fulfilled', value: '3' },
            { status: 'rejected', reason: failure },
            { status: 'fulfilled', value: 'Key value does not exist' },
            { status: 'fulfilled', value: '1' }
        ]);
    });

    it('keeps at most `concurrency` reads in flight', async () => {
        const keys = Array.from({ length: 20 }, (_, i) => `k${i}`);
        const fake = fakeService(Object.fromEntries(keys.map((k) => [k, k])));
        const results = await getKeyValues(fake.service, 'avenabox', keys, 4);
        expect(results.map((r) => (r.status === 'fulfilled' ? r.value : null))).toEqual(keys);
        expect(fake.maxInFlight()).toBe(4);
    });

    it('handles an empty key list', async () => {
        const fake = fakeService({});
        expect(await getKeyValues(fake.service, 'avenabox', [])).toEqual([]);
    });
});
