/**
 * Connection state shown by the live plot page, driven by the NATS client's status
 * events and its `closed()` promise.
 *
 * @module
 */

/**
 * - `connecting`: the page is opening the connection or subscribing.
 * - `connected`: the live subscriptions exist and the client is connected.
 * - `reconnecting`: the client lost the server and is trying to reconnect; it
 *   resubscribes by itself when it succeeds.
 * - `disconnected`: no connection (never opened, closed, or reconnecting gave up).
 */
export type LiveConnectionState = 'connecting' | 'connected' | 'reconnecting' | 'disconnected';

/**
 * Returns the state after a NATS client status event.
 *
 * `disconnect` and `reconnecting` mean the client is trying to get the server back;
 * `reconnect` means it did. `close` means it gave up or was closed. Other events
 * (`ping`, `update`, `ldm`, `error`, ...) leave the state as it is. Before the page has
 * subscribed (`connecting`) a `reconnect` does not claim `connected`.
 *
 * @param current - State before the event.
 * @param statusType - `type` of the client's `Status` event.
 * @returns The new state.
 */
export function nextConnectionState(current: LiveConnectionState, statusType: string): LiveConnectionState {
    switch (statusType) {
        case 'disconnect':
        case 'reconnecting':
        case 'staleConnection':
        case 'forceReconnect':
            return current === 'disconnected' ? current : 'reconnecting';
        case 'reconnect':
            return current === 'reconnecting' ? 'connected' : current;
        case 'close':
            return 'disconnected';
        default:
            return current;
    }
}

/**
 * Text of the connection badges.
 *
 * @param state - Connection state.
 */
export function connectionLabel(state: LiveConnectionState): string {
    switch (state) {
        case 'connected': return 'Connected';
        case 'connecting': return 'Connecting...';
        case 'reconnecting': return 'Reconnecting...';
        default: return 'Disconnected';
    }
}

/**
 * Color class of the connection dot in the header.
 *
 * @param state - Connection state.
 */
export function connectionDotClass(state: LiveConnectionState): string {
    if (state === 'connected') return 'bg-success';
    if (state === 'disconnected') return 'bg-error';
    return 'bg-warning';
}

/**
 * DaisyUI badge class of the connection state in Data Statistics.
 *
 * @param state - Connection state.
 */
export function connectionBadgeClass(state: LiveConnectionState): string {
    if (state === 'connected') return 'badge-success';
    if (state === 'disconnected') return 'badge-error';
    return 'badge-warning';
}
