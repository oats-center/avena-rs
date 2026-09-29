/**
 * Saving an export: streamed straight to a file where the browser has the File System
 * Access API (`showSaveFilePicker`, Chromium browsers), otherwise through a download
 * link for the finished Blob (Firefox, Safari).
 *
 * The save picker needs a user gesture, so the page calls {@link pickSaveFile}
 * synchronously in the export click handler, before its first `await`. The chunks are
 * then written to the file as they arrive: {@link tapExportChunks} wraps the export's
 * NATS connection so every `chunk` frame of the reply stream is written, and the write
 * finishes, before `downloadExportViaNats` sees the frame and acknowledges it. A slow
 * disk therefore slows the exporter down instead of piling data up in memory.
 *
 * @module
 */
import type { ExportRequestPayload } from '../exporter';
import type { NatsService } from '../nats.svelte';

/** NATS header that names the frame type of each export reply (see the export protocol). */
const EXPORT_FRAME_HEADER = 'Avena-Export-Frame';

/** The part of `FileSystemFileHandle` used here. */
export interface SaveFileHandle {
    createWritable(): Promise<SaveFileWritable>;
}

/** The part of `FileSystemWritableFileStream` used here. */
export interface SaveFileWritable {
    write(data: Uint8Array): Promise<void>;
    close(): Promise<void>;
    abort(reason?: unknown): Promise<void>;
}

/** `window.showSaveFilePicker`, which TypeScript's DOM types do not include. */
type ShowSaveFilePicker = (options: {
    suggestedName?: string;
    types?: { description: string; accept: Record<string, string[]> }[];
}) => Promise<SaveFileHandle>;

/**
 * Opens the browser's save dialog for a CSV file, if the browser has one.
 *
 * Must be called synchronously from a user gesture (a click or submit handler, before
 * any `await`), or the browser refuses it.
 *
 * @param suggestedName - File name offered in the dialog.
 * @param scope - Object holding `showSaveFilePicker`; `window` unless a test passes one.
 * @returns The dialog's promise (it rejects with an `AbortError` when the user
 *   cancels), or `null` when the browser has no save dialog.
 */
export function pickSaveFile(suggestedName: string, scope: unknown = globalThis): Promise<SaveFileHandle> | null {
    const picker = (scope as { showSaveFilePicker?: ShowSaveFilePicker } | undefined)?.showSaveFilePicker;
    if (typeof picker !== 'function') return null;
    return picker.call(scope, {
        suggestedName,
        types: [{ description: 'CSV file', accept: { 'text/csv': ['.csv'] } }]
    });
}

/**
 * Tells whether an error means the user closed the save dialog without choosing a file.
 *
 * @param err - Rejection of the {@link pickSaveFile} promise.
 */
export function isPickerCancelled(err: unknown): boolean {
    return typeof err === 'object' && err !== null && (err as { name?: unknown }).name === 'AbortError';
}

/**
 * Suggests a file name before the exporter has named the file: the request's
 * `download_name`, else the exporter's default `labjack_asset<NNN>_<start>_<end>.csv`
 * with times as `YYYYMMDDTHHMMSS` in UTC.
 *
 * @param payload - Export request.
 */
export function suggestedExportFileName(payload: ExportRequestPayload): string {
    if (payload.download_name) return payload.download_name;
    const stamp = (iso: string) => {
        const date = new Date(iso);
        if (isNaN(date.getTime())) return 'unknown';
        return date.toISOString().slice(0, 19).replace(/[-:]/g, '');
    };
    return `labjack_asset${String(payload.asset).padStart(3, '0')}_${stamp(payload.start)}_${stamp(payload.end)}.csv`;
}

/**
 * Tells whether a reply message is an export `chunk` frame: not a NATS status message,
 * and named `chunk` by the frame header or carrying no header (see
 * `downloadExportViaNats`).
 *
 * @param msg - Reply message.
 */
export function isChunkFrame(msg: { headers?: { get(name: string): string; code?: number; hasError?: boolean } }): boolean {
    const headers = msg.headers;
    if (headers && (headers.hasError || (headers.code ?? 0) >= 300)) return false;
    return (headers?.get(EXPORT_FRAME_HEADER) || 'chunk') === 'chunk';
}

/**
 * Wraps a connection so that every export `chunk` frame received on a subscription
 * made through it is passed to `onChunk` first. The frame reaches the reader (and is
 * acknowledged) only after `onChunk` resolves. If `onChunk` throws, the reader's loop
 * throws the same error. All other calls go to the connection unchanged.
 *
 * @param service - The export's connection.
 * @param onChunk - Gets a copy of each chunk's bytes, in arrival order.
 * @returns A service to hand to `downloadExportViaNats`.
 */
export function tapExportChunks(service: NatsService, onChunk: (data: Uint8Array) => Promise<void>): NatsService {
    const connection = service.connection;
    const tappedConnection = new Proxy(connection, {
        get(target, property) {
            if (property === 'subscribe') {
                return (...args: unknown[]) => {
                    const sub = (target.subscribe as (...a: unknown[]) => AsyncIterable<unknown> & object)(...args);
                    return tapSubscription(sub, onChunk);
                };
            }
            const value = Reflect.get(target, property, target);
            return typeof value === 'function' ? value.bind(target) : value;
        }
    });
    return new Proxy(service, {
        get(target, property) {
            if (property === 'connection') return tappedConnection;
            return Reflect.get(target, property, target);
        }
    });
}

/**
 * Wraps one subscription: iterating it yields the same messages, after passing each
 * chunk frame to `onChunk`. Every other property is the subscription's own.
 */
function tapSubscription<S extends AsyncIterable<unknown> & object>(
    sub: S,
    onChunk: (data: Uint8Array) => Promise<void>
): S {
    return new Proxy(sub, {
        get(target, property) {
            if (property === Symbol.asyncIterator) {
                return async function* () {
                    for await (const msg of target) {
                        const m = msg as { data: Uint8Array | ArrayBuffer; headers?: { get(name: string): string; code?: number; hasError?: boolean } };
                        if (isChunkFrame(m)) {
                            const data = m.data instanceof Uint8Array ? m.data : new Uint8Array(m.data);
                            await onChunk(data.slice());
                        }
                        yield msg;
                    }
                };
            }
            const value = Reflect.get(target, property, target);
            return typeof value === 'function' ? value.bind(target) : value;
        }
    });
}

/**
 * Saves a finished export through a temporary download link, then releases the
 * object URL. Used where the browser has no save dialog.
 *
 * @param blob - The CSV.
 * @param fileName - Name for the download.
 */
export function saveBlobViaLink(blob: Blob, fileName: string) {
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = fileName;
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);
}
