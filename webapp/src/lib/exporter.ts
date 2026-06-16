import { createInbox } from "@nats-io/nats-core";
import type { NatsService } from "./nats.svelte";

/** Request payload sent to the archive exporter worker. */
export interface ExportRequestPayload {
  /** Asset number whose archived channel data should be exported. */
  asset: number;
  /** LabJack analog input channels to include in the CSV export. */
  channels: number[];
  /** Inclusive RFC 3339 start timestamp for exported samples. */
  start: string;
  /** Inclusive RFC 3339 end timestamp for exported samples. */
  end: string;
  /** Export format. The frontend currently requests CSV only. */
  format?: "csv";
  /** Optional filename suggested by the caller and used when metadata is absent. */
  download_name?: string;
  /** Optional box identifier retained for compatibility with older request shapes. */
  box_id?: string;
}

/** Completed export result assembled from streamed NATS response frames. */
export interface ExportStreamResult {
  /** CSV content as a browser Blob. */
  blob: Blob;
  /** Filename advertised by the exporter or derived from the request. */
  fileName: string;
  /** Number of bytes reported by the exporter, or counted locally. */
  size: number;
  /** Requested channels that had no matching archived rows. */
  missingChannels: number[];
}

/** Optional callbacks and timeout controls for a streamed export. */
export interface ExportStreamOptions {
  /** Called after each chunk with total bytes received so far. */
  onProgress?: (received: number) => void;
  /** Called when the exporter sends its missing-channel summary. */
  onSummary?: (missingChannels: number[]) => void;
  /** Idle timeout for the export response stream. */
  idleTimeoutMs?: number;
}

type SummaryFrame = {
  type: "summary";
  bytesSent?: number;
  missingChannels?: number[];
};

type MetaFrame = {
  type: "meta";
  fileName?: string;
  contentType?: string;
};

type ErrorFrame = {
  type: "error";
  message: string;
};

type CompleteFrame = {
  type: "complete";
};

type Frame = SummaryFrame | MetaFrame | ErrorFrame | CompleteFrame | Record<string, unknown>;

const EXPORT_FRAME_HEADER = "Avena-Export-Frame";

/** Returns true when a decoded JSON frame is export metadata. */
function isMetaFrame(frame: Frame): frame is MetaFrame {
  return (frame as { type?: unknown }).type === "meta";
}

/** Returns true when a decoded JSON frame is the final export summary. */
function isSummaryFrame(frame: Frame): frame is SummaryFrame {
  return (frame as { type?: unknown }).type === "summary";
}

/** Returns true when a decoded JSON frame reports an exporter error. */
function isErrorFrame(frame: Frame): frame is ErrorFrame {
  return (frame as { type?: unknown }).type === "error";
}

/**
 * Requests a CSV export from the NATS worker and assembles streamed chunks.
 *
 * The function publishes the request with an inbox reply subject, acknowledges
 * chunk frames on a separate inbox, and returns after the exporter sends a
 * complete frame.
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
