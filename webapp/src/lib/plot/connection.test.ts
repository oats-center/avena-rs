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

describe('connection labels', () => {
    it('names each state and colors it', async () => {
        const { connectionLabel, connectionDotClass, connectionBadgeClass } = await import('./connection');
        expect(connectionLabel('connected')).toBe('Connected');
        expect(connectionLabel('connecting')).toBe('Connecting...');
        expect(connectionLabel('reconnecting')).toBe('Reconnecting...');
        expect(connectionLabel('disconnected')).toBe('Disconnected');
        expect(connectionDotClass('connected')).toBe('bg-success');
        expect(connectionDotClass('disconnected')).toBe('bg-error');
        expect(connectionDotClass('connecting')).toBe('bg-warning');
        expect(connectionBadgeClass('connected')).toBe('badge-success');
        expect(connectionBadgeClass('disconnected')).toBe('badge-error');
        expect(connectionBadgeClass('reconnecting')).toBe('badge-warning');
    });
});
