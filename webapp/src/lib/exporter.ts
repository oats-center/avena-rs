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
 * and the exporter pauses every eight chunks until the acks arrive.
 *
 * See `docs/src/reference/export-protocol.md` for the full protocol.
 *
 * @module
 */
import { createInbox } from "@nats-io/nats-core";
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
 * 3. Reads reply messages. The `Avena-Export-Frame` header names each frame; a
 *    message without the header counts as a `chunk`. Each chunk is copied, kept in
 *    memory, and acknowledged by publishing an empty message to the ack inbox.
 *    `meta` and `summary` frames are parsed as JSON and kept. Frames with another
 *    name, or whose `type` field does not match the header, are ignored.
 * 4. On the `complete` frame, joins the chunks into one Blob and resolves.
 *
 * The whole file is held in memory until it completes.
 *
 * @remarks
 * The request is a plain publish, not a NATS request, so there is no "no
 * responders" error. If no exporter is subscribed to `requestSubject` (the exporter
 * is not running or the box is offline), the call waits for the full idle timeout
 * before rejecting.
 *
 * @param nats - Connected service from {@link "nats.svelte"!connect}.
 * @param requestSubject - Export request subject, e.g. from
 *   {@link subjects!archiveExportRequestSubject}:
 *   `avenars.<site>.<box>.<source>.export.request`.
 * @param payload - Request fields. `format` is overwritten with `csv`.
 * @param options - Progress callbacks and idle timeout.
 * @returns Resolves to the assembled CSV, its file name, size and missing channels.
 * @throws If publishing fails, a non-chunk frame is not valid JSON, the exporter
 *   sends an `error` frame, no message arrives within the idle timeout, or the reply
 *   subscription ends before a `complete` frame.
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
  const inbox = createInbox();
  const ackSubject = createInbox();
  const sub = nats.connection.subscribe(inbox);
  const chunks: ArrayBuffer[] = [];
  let meta: MetaFrame | null = null;
  let summary: SummaryFrame | null = null;
  let totalBytes = 0;
  let timedOut = false;
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

  try {
    nats.connection.publish(
      requestSubject,
      new TextEncoder().encode(
        JSON.stringify({ ...payload, format: "csv" as const, ack_subject: ackSubject })
      ),
      { reply: inbox }
    );
    if (typeof nats.connection.flush === "function") {
      await nats.connection.flush();
    }

    for await (const msg of sub) {
      resetIdleTimeout();
      const frame = msg.headers?.get(EXPORT_FRAME_HEADER) ?? "chunk";

      if (frame === "chunk") {
        const data = msg.data instanceof Uint8Array ? msg.data : new Uint8Array(msg.data);
        const copy = new Uint8Array(data.byteLength);
        copy.set(data);
        chunks.push(copy.buffer as ArrayBuffer);
        totalBytes += data.byteLength;
        // One ack per stored chunk; the exporter waits for these every eight chunks.
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
        throw new Error(`Export server error: ${parsed.message}`);
      }

      if (frame === "complete") {
        const fileName = meta?.fileName ?? payload.download_name ?? "labjack_export.csv";
        const mime = meta?.contentType ?? "text/csv";
        const blob = new Blob(chunks, { type: mime });
        return {
          blob,
          fileName,
          size: summary?.bytesSent ?? totalBytes ?? blob.size,
          missingChannels: summary?.missingChannels ?? [],
        };
      }
    }

    if (timedOut) {
      throw new Error(`Timed out after ${Math.round(idleTimeoutMs / 1000)} seconds without NATS export data`);
    }
    throw new Error("NATS export response ended before completion");
  } finally {
    clearTimeout(timeout);
    try {
      sub.unsubscribe();
    } catch {
      // Subscription may already be closed.
    }
  }
}
