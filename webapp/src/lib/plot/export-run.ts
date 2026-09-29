/**
 * Runs one CSV export on a NATS connection of its own.
 *
 * The plot page closes and reopens its live connection on every reload, Retry or
 * Reconnect. An export on that connection would die with it, so each export opens its
 * own connection and closes it when the export completes, fails or is cancelled.
 *
 * @module
 */
import {
    downloadExportViaNats,
    type ExportRequestPayload,
    type ExportStreamOptions,
    type ExportStreamResult
} from '../exporter';
import type { NatsService } from '../nats.svelte';

/** Options of {@link runExportOnOwnConnection}. */
export interface ExportRunOptions {
    /** Opens the export's connection; resolves to `null` when that fails. */
    openConnection: () => Promise<NatsService | null>;
    /** Export request subject (`archiveExportRequestSubject`). */
    subject: string;
    /** Request fields. */
    payload: ExportRequestPayload;
    /** Cancels the export, including while the connection is being opened. */
    signal: AbortSignal;
    /** Progress and summary callbacks, passed on to `downloadExportViaNats`. */
    onProgress?: ExportStreamOptions['onProgress'];
    onSummary?: ExportStreamOptions['onSummary'];
    /**
     * Changes the connection handed to the download, for example to see the chunks as
     * they arrive (see `tapExportChunks`). The original connection is still the one
     * closed afterwards.
     */
    wrapConnection?: (service: NatsService) => NatsService;
    /** The download function; `downloadExportViaNats` unless a test replaces it. */
    download?: typeof downloadExportViaNats;
}

/** Error thrown when the export is cancelled before the download starts. */
function cancelledError(): Error {
    const err = new Error('Export cancelled');
    err.name = 'AbortError';
    return err;
}

/**
 * Closes a connection without waiting, logging any error.
 *
 * @param service - Connection to close.
 */
function closeQuietly(service: NatsService) {
    try {
        service.connection.close().catch((err) => console.error('Error closing the export connection:', err));
    } catch (err) {
        console.error('Error closing the export connection:', err);
    }
}

/**
 * Opens a connection, downloads the export on it and closes it again.
 *
 * @param options - See {@link ExportRunOptions}.
 * @returns The export result from `downloadExportViaNats`.
 * @throws An error named `AbortError` when `signal` is aborted (at any point), an
 *   error when the connection cannot be opened, or whatever the download throws. The
 *   connection is closed in every case.
 */
export async function runExportOnOwnConnection(options: ExportRunOptions): Promise<ExportStreamResult> {
    if (options.signal.aborted) throw cancelledError();
    const service = await options.openConnection();
    if (!service) {
        if (options.signal.aborted) throw cancelledError();
        throw new Error('Failed to connect to NATS server for the export');
    }
    try {
        if (options.signal.aborted) throw cancelledError();
        const download = options.download ?? downloadExportViaNats;
        const target = options.wrapConnection ? options.wrapConnection(service) : service;
        return await download(target, options.subject, options.payload, {
            signal: options.signal,
            onProgress: options.onProgress,
            onSummary: options.onSummary
        });
    } finally {
        closeQuietly(service);
    }
}
