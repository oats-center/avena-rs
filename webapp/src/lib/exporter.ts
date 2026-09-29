/**
 * Browser client for the archive exporter's NATS worker protocol.
 *
 * The exporter runs on each edge box and listens on
 * `avenars.<site>.<box>.<source>.export.request`. The dashboard publishes one JSON
 * request there over its existing WebSocket connection to central NATS, with a reply
 * inbox. The exporter answers on that inbox with a sequence of frames, each named by
 * the `Avena-Export-Frame` header: `meta`, one or more `chunk` frames of raw CSV
 * bytes, `summary`, and `complete`, or an `error` frame instead. Because core NATS
 * has no flow control, the client acknowledges each chunk on a separate ack subject,
 * and the exporter pauses every eight chunks until the acks arrive. To stop an export
 * early the client publishes a `cancel` message on the same ack subject.
 *
 * See `docs/src/reference/export-protocol.md` for the full protocol.
 *
 * @module
 */
import { createInbox, headers } from "@nats-io/nats-core";
import type { NatsService } from "./nats.svelte";

/**
 * Export request fields supplied by the caller.
 *
 * {@link downloadExportViaNats} sends these as JSON with `format` forced to `csv` and
 * an `ack_subject` added.
 */
export interface ExportRequestPayload {
  /** Asset number; selects the `asset<NNN>` directory in the edge box's Parquet archive. */
  asset: number;
  /** LabJack channel numbers to export. The exporter sorts them and removes duplicates. */
  channels: number[];
  /** Start of the range, RFC 3339. Inclusive. */
  start: string;
  /** End of the range, RFC 3339. Inclusive; must not be before `start`. */
  end: string;
  /** Export format. Ignored here: {@link downloadExportViaNats} always sends `csv`. */
  format?: "csv";
  /**
   * File name the exporter reports in its `meta` frame. Also used locally when no
   * `meta` frame arrives. The exporter's default is
   * `labjack_asset<NNN>_<start>_<end>.csv`.
   */
  download_name?: string;
  /**
   * Box identifier kept for compatibility with older request shapes. The exporter does
   * not read it; the box is chosen by the request subject.
   */
  box_id?: string;
}

/** Finished export assembled from the exporter's reply frames. */
export interface ExportStreamResult {
  /**
   * All CSV chunks joined in arrival order, typed with the `meta` frame's content type
   * or `text/csv`.
   */
  blob: Blob;
  /**
   * File name from the `meta` frame, else the request's `download_name`, else
   * `labjack_export.csv`.
   */
  fileName: string;
  /** Size in bytes: `bytesSent` from the `summary` frame, else the bytes received. */
  size: number;
  /** Requested channels with no rows in the range, from the `summary` frame, else empty. */
  missingChannels: number[];
}

/** Optional callbacks and timeout for {@link downloadExportViaNats}. */
export interface ExportStreamOptions {
  /** Called after each chunk is stored and acked, with the total bytes received so far. */
  onProgress?: (received: number) => void;
  /** Called on the `summary` frame with its missing channels (empty if absent). */
  onSummary?: (missingChannels: number[]) => void;
  /**
   * Longest wait between two reply messages, in milliseconds. The timer restarts on
   * every message. Default: 600000 (10 minutes).
   */
  idleTimeoutMs?: number;
  /**
   * Cancels the download when aborted: a `cancel` message is published on the ack
   * subject so the exporter stops before its next chunk, no further frames are read
   * or acknowledged, the reply subscription is released, and the call rejects with
   * an error named `AbortError` (see {@link isExportCancelled}). An exporter that
   * predates the cancel message counts it as one ack and stops when its acks time out.
   */
  signal?: AbortSignal;
}

/**
 * Tells whether an error from {@link downloadExportViaNats} means the caller
 * cancelled the download through `signal`.
 *
 * @param err - Rejection value.
 * @returns `true` for the cancellation error.
 */
export function isExportCancelled(err: unknown): boolean {
  return err instanceof Error && err.name === "AbortError";
}

/** Builds the error {@link downloadExportViaNats} rejects with when cancelled. */
function cancelledError(): Error {
  const err = new Error("Export cancelled");
  err.name = "AbortError";
  return err;
}

/** Body of a `summary` frame: bytes sent and channels with no rows. */
type SummaryFrame = {
  type: "summary";
  bytesSent?: number;
  missingChannels?: number[];
};

/** Body of a `meta` frame: suggested file name and MIME type. */
type MetaFrame = {
  type: "meta";
  fileName?: string;
  contentType?: string;
};

/** Body of an `error` frame: the exporter's error message. */
type ErrorFrame = {
  type: "error";
  message: string;
};

/** Body of a `complete` frame, which marks the end of a successful export. */
type CompleteFrame = {
  type: "complete";
};

/** Any JSON frame body; unknown shapes are allowed and ignored. */
type Frame = SummaryFrame | MetaFrame | ErrorFrame | CompleteFrame | Record<string, unknown>;

/** NATS header whose value names the frame type of each reply message. */
const EXPORT_FRAME_HEADER = "Avena-Export-Frame";

/**
 * Frame name of the cancel message a client publishes on its ack subject. The body
 * repeats it as JSON for tools that cannot set headers.
 */
const EXPORT_FRAME_CANCEL = "cancel";

/**
 * Bytes of received chunks kept as separate arrays before they are folded into one
 * Blob part. Once in a Blob the bytes are held by the browser's Blob storage (which
 * can page large Blobs to disk) instead of the JavaScript heap.
 */
const BLOB_PART_BYTES = 8 * 1024 * 1024;

/**
 * Reports whether a decoded JSON frame is a `meta` frame.
 *
 * @param frame - Parsed frame body.
 * @returns `true` if its `type` field is `meta`.
 */
function isMetaFrame(frame: Frame): frame is MetaFrame {
  return (frame as { type?: unknown }).type === "meta";
}

/**
 * Reports whether a decoded JSON frame is a `summary` frame.
 *
 * @param frame - Parsed frame body.
 * @returns `true` if its `type` field is `summary`.
 */
function isSummaryFrame(frame: Frame): frame is SummaryFrame {
  return (frame as { type?: unknown }).type === "summary";
}

/**
 * Reports whether a decoded JSON frame is an `error` frame.
 *
 * @param frame - Parsed frame body.
 * @returns `true` if its `type` field is `error`.
 */
function isErrorFrame(frame: Frame): frame is ErrorFrame {
  return (frame as { type?: unknown }).type === "error";
}

/**
 * Requests a CSV export from an edge box's exporter and collects the streamed reply.
 *
 * Steps, in order:
 *
 * 1. Creates a reply inbox and an ack inbox, and subscribes to the reply inbox.
 * 2. Publishes `payload` as JSON to `requestSubject`, with `format: "csv"` and
 *    `ack_subject` set to the ack inbox, and flushes.
 * 3. Reads reply messages. A NATS status message with code 503 means no exporter is
 *    subscribed to `requestSubject` (the box is offline or its exporter is not
 *    running), and the call rejects at once instead of waiting for the idle
 *    timeout. Otherwise the `Avena-Export-Frame` header names each frame; a message
 *    without the header counts as a `chunk`. Each chunk is copied, stored, and
 *    acknowledged by publishing an empty message to the ack inbox. `meta` and
 *    `summary` frames are parsed as JSON and kept. Frames with another name, or
 *    whose `type` field does not match the header, are ignored.
 * 4. On the `complete` frame, joins the stored parts into one Blob and resolves.
 *
 * If the call ends any other way after the request was sent (cancelled through
 * `options.signal`, idle timeout, a bad frame, the subscription ending), it publishes
 * a `cancel` message on the ack subject, with the header `Avena-Export-Frame: cancel`
 * and the body `{"type":"cancel"}`, so the exporter stops instead of waiting for acks.
 * No cancel is sent after an `error` frame or a NATS status reply, since the exporter
 * has already stopped or never started.
 *
 * Chunks are folded into a Blob part every 8 MiB, so at most that much CSV sits in
 * JavaScript memory at a time; the rest is held in the browser's Blob storage until
 * the caller saves the result.
 *
 * @param nats - Connected service from {@link "nats.svelte"!connect}.
 * @param requestSubject - Export request subject, e.g. from
 *   {@link subjects!archiveExportRequestSubject}:
 *   `avenars.<site>.<box>.<source>.export.request`.
 * @param payload - Request fields. `format` is overwritten with `csv`.
 * @param options - Progress callbacks and idle timeout.
 * @returns Resolves to the assembled CSV, its file name, size and missing channels.
 * @throws If publishing fails, no exporter is listening on `requestSubject`, the
 *   server returns another error status, a non-chunk frame is not valid JSON, the
 *   exporter sends an `error` frame, no message arrives within the idle timeout, the reply
 *   subscription ends before a `complete` frame, or `options.signal` is aborted (an
 *   error named `AbortError`).
 *
 * @example
 * ```ts
 * const subject = archiveExportRequestSubject(config);
 * const result = await downloadExportViaNats(nats, subject, {
 *   asset: 1001,
 *   channels: [8, 9],
 *   start: "2026-09-22T12:00:00Z",
 *   end: "2026-09-22T12:30:00Z",
 * }, { onProgress: (bytes) => console.log(bytes) });
 * ```
 */
export async function downloadExportViaNats(
  nats: NatsService,
  requestSubject: string,
  payload: ExportRequestPayload,
  options: ExportStreamOptions = {}
): Promise<ExportStreamResult> {
  const signal = options.signal;
  if (signal?.aborted) throw cancelledError();
  const inbox = createInbox();
  const ackSubject = createInbox();
  const sub = nats.connection.subscribe(inbox);
  // Finished Blob parts, plus the chunks received since the last part was made.
  const parts: Blob[] = [];
  let pending: Uint8Array[] = [];
  let pendingBytes = 0;
  let meta: MetaFrame | null = null;
  let summary: SummaryFrame | null = null;
  let totalBytes = 0;
  let timedOut = false;
  // Set once the request is published; no cancel is needed before that.
  let requestSent = false;
  // Set when the exporter has finished on its own (complete or error frame, status).
  let exporterDone = false;
  let cancelSent = false;
  const idleTimeoutMs = options.idleTimeoutMs ?? 10 * 60_000;
  let timeout: ReturnType<typeof setTimeout>;

  // Restarts the idle timer. When it fires, unsubscribing ends the `for await`
  // loop below, which then throws the timeout error.
  const resetIdleTimeout = () => {
    clearTimeout(timeout);
    timeout = setTimeout(() => {
      timedOut = true;
      try {
        sub.unsubscribe();
      } catch {
        // Subscription may already be closed.
      }
    }, idleTimeoutMs);
  };

  timeout = setTimeout(() => {
    timedOut = true;
    try {
      sub.unsubscribe();
    } catch {
      // Subscription may already be closed.
    }
  }, idleTimeoutMs);

  // Tells the exporter to stop. Best effort: the connection may already be closed.
  const sendCancel = () => {
    if (!requestSent || exporterDone || cancelSent) return;
    cancelSent = true;
    try {
      const h = headers();
      h.set(EXPORT_FRAME_HEADER, EXPORT_FRAME_CANCEL);
      nats.connection.publish(
        ackSubject,
        new TextEncoder().encode(JSON.stringify({ type: EXPORT_FRAME_CANCEL })),
        { headers: h }
      );
      if (typeof nats.connection.flush === "function") {
        nats.connection.flush().catch(() => {});
      }
    } catch {
      // The exporter falls back to its ack timeout.
    }
  };

  // Cancelling tells the exporter to stop and unsubscribes, which ends the
  // `for await` loop below.
  const onAbort = () => {
    sendCancel();
    try {
      sub.unsubscribe();
    } catch {
      // Subscription may already be closed.
    }
  };
  signal?.addEventListener("abort", onAbort, { once: true });

  try {
    nats.connection.publish(
      requestSubject,
      new TextEncoder().encode(
        JSON.stringify({ ...payload, format: "csv" as const, ack_subject: ackSubject })
      ),
      { reply: inbox }
    );
    requestSent = true;
    if (typeof nats.connection.flush === "function") {
      await nats.connection.flush();
    }

    for await (const msg of sub) {
      if (signal?.aborted) throw cancelledError();
      resetIdleTimeout();

      // The server answers a publish with a reply subject by a status-only message
      // (503) when nothing is subscribed to the subject; other status codes are
      // errors too. Exporter frames never carry a status.
      if (msg.headers?.code === 503) {
        exporterDone = true;
        throw new Error(
          `No exporter is listening on ${requestSubject}. The edge box is offline or its exporter is not running.`
        );
      }
      if (msg.headers?.hasError) {
        exporterDone = true;
        const { code, description } = msg.headers;
        throw new Error(`NATS export request failed: ${code}${description ? ` ${description}` : ""}`);
      }

      const frame = msg.headers?.get(EXPORT_FRAME_HEADER) || "chunk";

      if (frame === "chunk") {
        const data = msg.data instanceof Uint8Array ? msg.data : new Uint8Array(msg.data);
        // Copy, since the message may be a view into a buffer the client reuses.
        pending.push(data.slice());
        pendingBytes += data.byteLength;
        totalBytes += data.byteLength;
        if (pendingBytes >= BLOB_PART_BYTES) {
          parts.push(new Blob(pending as BlobPart[]));
          pending = [];
          pendingBytes = 0;
        }
        // One ack per stored chunk; the exporter waits for these every eight chunks.
        // A cancelled download sends no more acks.
        if (signal?.aborted) throw cancelledError();
        nats.connection.publish(ackSubject);
        if (typeof nats.connection.flush === "function") {
          await nats.connection.flush();
        }
        options.onProgress?.(totalBytes);
        continue;
      }

      let parsed: Frame;
      try {
        parsed = JSON.parse(new TextDecoder().decode(msg.data)) as Frame;
      } catch (err) {
        throw new Error(
          err instanceof Error
            ? `Failed to parse export ${frame} frame: ${err.message}`
            : `Failed to parse export ${frame} frame`
        );
      }

      if (frame === "meta" && isMetaFrame(parsed)) {
        meta = parsed;
        continue;
      }

      if (frame === "summary" && isSummaryFrame(parsed)) {
        summary = parsed;
        options.onSummary?.(parsed.missingChannels ?? []);
        continue;
      }

      if (frame === "error" && isErrorFrame(parsed)) {
        exporterDone = true;
        throw new Error(`Export server error: ${parsed.message}`);
      }

      if (frame === "complete") {
        exporterDone = true;
        const fileName = meta?.fileName ?? payload.download_name ?? "labjack_export.csv";
        const mime = meta?.contentType ?? "text/csv";
        const blob = new Blob([...parts, ...(pending as BlobPart[])], { type: mime });
        return {
          blob,
          fileName,
          size: summary?.bytesSent ?? totalBytes ?? blob.size,
          missingChannels: summary?.missingChannels ?? [],
        };
      }
    }

    if (signal?.aborted) throw cancelledError();
    if (timedOut) {
      throw new Error(`Timed out after ${Math.round(idleTimeoutMs / 1000)} seconds without NATS export data`);
    }
    throw new Error("NATS export response ended before completion");
  } finally {
    signal?.removeEventListener("abort", onAbort);
    sendCancel();
    clearTimeout(timeout);
    try {
      sub.unsubscribe();
    } catch {
      // Subscription may already be closed.
    }
  }
}
