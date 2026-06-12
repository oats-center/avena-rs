import type { NatsService } from "$lib/nats.svelte";

export interface ExportRequestPayload {
  asset: number;
  channels: number[];
  start: string;
  end: string;
  format?: "csv";
  download_name?: string;
  box_id?: string;
}

export interface ExportStreamResult {
  blob: Blob;
  fileName: string;
  size: number;
  missingChannels: number[];
}

export interface ExportStreamOptions {
  onProgress?: (received: number) => void;
  onSummary?: (missingChannels: number[]) => void;
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

const DEFAULT_EXPORT_SUBJECT_PREFIX = "avenars.export";
const DEFAULT_EXPORT_TIMEOUT_MS = 30_000;
const EXPORT_FRAME_HEADER = "X-Avena-Export-Frame";
const EXPORT_FRAME_META = "meta";
const EXPORT_FRAME_CHUNK = "chunk";
const EXPORT_FRAME_SUMMARY = "summary";
const EXPORT_FRAME_COMPLETE = "complete";
const EXPORT_FRAME_ERROR = "error";

function sanitizeToken(raw: string): string {
  const normalized = raw
    .trim()
    .toLowerCase()
    .replace(/[\s./]+/g, "-")
    .replace(/[^a-z0-9_-]/g, "")
    .replace(/^-+|-+$/g, "");

  return normalized || "unknown";
}

function exportSubjectPrefix(): string {
  return import.meta.env.VITE_EXPORT_NATS_SUBJECT_PREFIX?.trim() || DEFAULT_EXPORT_SUBJECT_PREFIX;
}

function requestSubject(boxId: string): string {
  return `${exportSubjectPrefix()}.request.${sanitizeToken(boxId)}`;
}

function replySubject(jobId: string): string {
  return `${exportSubjectPrefix()}.reply.${sanitizeToken(jobId)}`;
}

function randomJobId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }

  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function textDecoder(): TextDecoder {
  return new TextDecoder();
}

function textEncoder(): TextEncoder {
  return new TextEncoder();
}

function frameTypeFromHeaders(headers: any): string {
  if (!headers) return "";
  const value = headers.get?.(EXPORT_FRAME_HEADER);
  if (typeof value === "string") return value;
  if (Array.isArray(value) && value.length > 0) return String(value[0]);
  return "";
}

export async function downloadExportViaNats(
  natsService: NatsService,
  payload: ExportRequestPayload,
  options: ExportStreamOptions = {}
): Promise<ExportStreamResult> {
  const boxId = payload.box_id?.trim();
  if (!boxId) {
    throw new Error("Export requires box_id so the request can be routed to the correct edge box.");
  }

  const nc = natsService.connection;
  const jobId = randomJobId();
  const reply = replySubject(jobId);
  const request = requestSubject(boxId);
  const sub = nc.subscribe(reply);
  const encoder = textEncoder();
  const decoder = textDecoder();

  const chunks: Uint8Array[] = [];
  let meta: MetaFrame | null = null;
  let summary: SummaryFrame | null = null;
  let totalBytes = 0;
  let settled = false;
  let timeoutId: ReturnType<typeof setTimeout> | null = null;

  const timeoutMs = Number(import.meta.env.VITE_EXPORT_NATS_TIMEOUT_MS ?? DEFAULT_EXPORT_TIMEOUT_MS);

  const clearTimer = () => {
    if (timeoutId) {
      clearTimeout(timeoutId);
      timeoutId = null;
    }
  };

  const resetTimer = (reject: (reason?: unknown) => void) => {
    clearTimer();
    timeoutId = setTimeout(() => {
      if (settled) return;
      settled = true;
      sub.unsubscribe();
      reject(new Error(`Timed out waiting for export response after ${timeoutMs}ms.`));
    }, timeoutMs);
  };

  return new Promise<ExportStreamResult>((resolve, reject) => {
    const cleanup = () => {
      clearTimer();
      sub.unsubscribe();
    };

    const fail = (message: string) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(new Error(message));
    };

    const succeed = (result: ExportStreamResult) => {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(result);
    };

    (async () => {
      try {
        resetTimer(reject);

        for await (const message of sub) {
          resetTimer(reject);

          const frameType = frameTypeFromHeaders(message.headers);
          if (!frameType) {
            fail("Export response was missing frame headers.");
            return;
          }

          if (frameType === EXPORT_FRAME_CHUNK) {
            const data = message.data.slice();
            totalBytes += data.length;
            chunks.push(data);
            options.onProgress?.(totalBytes);
            continue;
          }

          const text = decoder.decode(message.data);
          let frame: Frame;
          try {
            frame = JSON.parse(text) as Frame;
          } catch {
            fail("Export response contained invalid JSON metadata.");
            return;
          }

          if (frameType === EXPORT_FRAME_META) {
            meta = frame as MetaFrame;
            continue;
          }

          if (frameType === EXPORT_FRAME_SUMMARY) {
            summary = frame as SummaryFrame;
            options.onSummary?.(summary.missingChannels ?? []);
            continue;
          }

          if (frameType === EXPORT_FRAME_ERROR) {
            fail((frame as ErrorFrame).message || "Export failed.");
            return;
          }

          if (frameType === EXPORT_FRAME_COMPLETE) {
            const fileName = meta?.fileName ?? payload.download_name ?? "labjack_export.csv";
            const mime = meta?.contentType ?? "text/csv";
            const blob = new Blob(chunks, { type: mime });
            succeed({
              blob,
              fileName,
              size: totalBytes,
              missingChannels: summary?.missingChannels ?? [],
            });
            return;
          }
        }

        fail("Export response stream ended unexpectedly.");
      } catch (error) {
        fail(error instanceof Error ? error.message : "Export failed.");
      }
    })().catch((error) => {
      fail(error instanceof Error ? error.message : "Export failed.");
    });

    const requestBody = {
      job_id: jobId,
      response_subject: reply,
      asset: payload.asset,
      channels: payload.channels,
      start: payload.start,
      end: payload.end,
      format: payload.format ?? "csv",
      download_name: payload.download_name,
    };

    nc.publish(request, encoder.encode(JSON.stringify(requestBody)));
    nc.flush()
      .then(() => {
        resetTimer(reject);
      })
      .catch((error) => {
        fail(error instanceof Error ? error.message : "Failed to publish export request.");
      });
  });
}
