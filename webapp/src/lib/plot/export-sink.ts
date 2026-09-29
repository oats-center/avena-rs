/**
 * Saving an export: streamed straight to a file where the browser has the File System
 * Access API (`showSaveFilePicker`, Chromium browsers), otherwise through a download
 * link for the finished Blob (Firefox, Safari).
 *
 * The save picker needs a user gesture, so the page calls {@link pickSaveFile}
 * synchronously in the export click handler, before its first `await`. The chunks are
 * then passed to `downloadExportViaNats` as its `onChunk` hook (see
 * {@link runAndSaveExport}): every chunk is written to the file, and the write finishes,
 * before the chunk is acknowledged. A slow disk therefore slows the exporter down
 * instead of piling data up in memory, and no other copy of the export is kept.
 *
 * @module
 */
import type { ExportRequestPayload, ExportStreamResult } from '../exporter';

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

/**
 * Runs an export and saves it: into `writable`, chunk by chunk, when the user chose a
 * file, otherwise through a download link once the export completes.
 *
 * With a file, `run` gets a chunk consumer that writes to it; pass that on as
 * `downloadExportViaNats`'s `onChunk`, so each write finishes before its chunk is
 * acknowledged. The file is closed when the export completes. If the export or a
 * write fails, or the export is cancelled, the file is aborted instead, which
 * discards what was written; a file that was being replaced stays as it was.
 *
 * @param writable - The chosen file, or `null` to save through a download link.
 * @param run - Starts the export, passing `onChunk` on to the download (it is
 *   `undefined` without a file, and the result's Blob is saved instead).
 * @param saveBlob - Saves the Blob; {@link saveBlobViaLink} unless a test passes one.
 * @returns The export result.
 * @throws Whatever the export, a write or closing the file throws.
 */
export async function runAndSaveExport(
    writable: SaveFileWritable | null,
    run: (onChunk?: (data: Uint8Array) => Promise<void>) => Promise<ExportStreamResult>,
    saveBlob: (blob: Blob, fileName: string) => void = saveBlobViaLink
): Promise<ExportStreamResult> {
    if (!writable) {
        const result = await run(undefined);
        saveBlob(result.blob, result.fileName);
        return result;
    }
    try {
        const result = await run((data) => writable.write(data));
        await writable.close();
        return result;
    } catch (err) {
        await writable.abort(err).catch((abortErr) => console.error('Error discarding the export file:', abortErr));
        throw err;
    }
}
