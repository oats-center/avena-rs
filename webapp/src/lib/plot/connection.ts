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
