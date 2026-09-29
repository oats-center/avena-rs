import { describe, expect, it } from 'vitest';
import { nextConnectionState } from './connection';

describe('nextConnectionState', () => {
    it('follows a lost and restored connection', () => {
        let state = nextConnectionState('connected', 'disconnect');
        expect(state).toBe('reconnecting');
        state = nextConnectionState(state, 'reconnecting');
        expect(state).toBe('reconnecting');
        state = nextConnectionState(state, 'reconnect');
        expect(state).toBe('connected');
    });

    it('ends in disconnected when the client closes', () => {
        expect(nextConnectionState('reconnecting', 'close')).toBe('disconnected');
        expect(nextConnectionState('connected', 'close')).toBe('disconnected');
        expect(nextConnectionState('disconnected', 'reconnecting')).toBe('disconnected');
    });

    it('ignores unrelated events', () => {
        expect(nextConnectionState('connected', 'ping')).toBe('connected');
        expect(nextConnectionState('connected', 'update')).toBe('connected');
        expect(nextConnectionState('connecting', 'reconnect')).toBe('connecting');
    });
});
