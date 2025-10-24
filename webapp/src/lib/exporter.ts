export interface ExportRequestPayload {
  asset: number;
  channels: number[];
  start: string;
  end: string;
  format?: "csv";
  download_name?: string;
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
  websocketUrl?: string;
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

const DEFAULT_WS_URL = "ws://127.0.0.1:9001/export";

export async function downloadExportViaWebSocket(
  payload: ExportRequestPayload,
  options: ExportStreamOptions = {}
): Promise<ExportStreamResult> {
  const url = options.websocketUrl ?? DEFAULT_WS_URL;
  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";

  const chunks: ArrayBuffer[] = [];
  let meta: MetaFrame | null = null;
  let summary: SummaryFrame | null = null;
  let totalBytes = 0;

  const result = await new Promise<ExportStreamResult>((resolve, reject) => {
    ws.onerror = (event) => {
      reject(new Error(`WebSocket error: ${JSON.stringify(event)}`));
    };

    ws.onclose = (event) => {
      if (event.code !== 1000) {
        reject(new Error(`WebSocket closed unexpectedly (${event.code})`));
      }
    };

    ws.onmessage = async (event) => {
      if (typeof event.data === "string") {
        try {
          const frame = JSON.parse(event.data) as Frame;
          if (frame.type === "meta") {
            meta = frame;
          } else if (frame.type === "summary") {
            summary = frame;
            options.onSummary?.(frame.missingChannels ?? []);
          } else if (frame.type === "complete") {
            const fileName =
              meta?.fileName ?? payload.download_name ?? `labjack_export.csv`;
            const mime = meta?.contentType ?? "text/csv";
            const blob = new Blob(chunks, { type: mime });
            resolve({
              blob,
              fileName,
              size: summary?.bytesSent ?? totalBytes ?? blob.size,
              missingChannels: summary?.missingChannels ?? [],
            });
            ws.close(1000);
          } else if (frame.type === "error") {
            reject(new Error((frame as ErrorFrame).message));
            ws.close(1011);
          }
        } catch (err) {
          reject(err as Error);
        }
      } else {
        const buffer =
          event.data instanceof ArrayBuffer
            ? event.data
            : await (event.data as Blob).arrayBuffer();
        chunks.push(buffer);
        totalBytes += buffer.byteLength;
        options.onProgress?.(totalBytes);
      }
    };

    ws.onopen = () => {
      const requestPayload = {
        ...payload,
        format: "csv" as const,
      };
      ws.send(JSON.stringify(requestPayload));
    };
  });

  return result;
}
