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

function stripTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}

function defaultWebsocketScheme(): "ws" | "wss" {
  if (typeof window !== "undefined" && window.location.protocol === "https:") {
    return "wss";
  }
  return "ws";
}

function normalizeWebsocketUrl(url: string): string {
  const trimmed = url.trim();
  if (!trimmed) return "";

  const withScheme = /^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(trimmed)
    ? trimmed
    : `${defaultWebsocketScheme()}://${trimmed}`;

  try {
    const parsed = new URL(withScheme);
    if (parsed.protocol === "http:") parsed.protocol = "ws:";
    if (parsed.protocol === "https:") parsed.protocol = "wss:";
    if (!parsed.pathname || parsed.pathname === "/") {
      parsed.pathname = "/export";
    }
    return stripTrailingSlash(parsed.toString());
  } catch {
    return withScheme;
  }
}

function buildDefaultExportCandidates(): string[] {
  if (typeof window === "undefined") {
    return ["ws://127.0.0.1:9001/export"];
  }

  const candidates = new Set<string>();
  const scheme = defaultWebsocketScheme();
  const { hostname, host, origin } = window.location;

  candidates.add(`${scheme}://${hostname}:9001/export`);
  candidates.add(`${scheme}://${host}/export`);

  try {
    const originUrl = new URL(origin);
    originUrl.protocol = scheme === "wss" ? "wss:" : "ws:";
    originUrl.pathname = "/export";
    originUrl.search = "";
    originUrl.hash = "";
    candidates.add(stripTrailingSlash(originUrl.toString()));
  } catch {
    // Keep the simpler candidates if URL construction fails.
  }

  if (hostname === "localhost" || hostname === "127.0.0.1") {
    candidates.add("ws://127.0.0.1:9001/export");
  }

  return Array.from(candidates);
}

function buildExportCandidates(override?: string): string[] {
  const configured = override?.trim() || import.meta.env.VITE_EXPORT_WS_URL?.trim() || "";
  if (configured) {
    return [normalizeWebsocketUrl(configured)];
  }
  return buildDefaultExportCandidates();
}

function buildWebSocketHint(url: string): string {
  if (typeof window === "undefined") {
    return "";
  }

  if (window.location.protocol === "https:" && url.startsWith("ws://")) {
    return " The app is loaded over HTTPS, so the browser will block insecure ws:// connections. Use a wss:// endpoint or proxy /export through HTTPS.";
  }

  try {
    const parsed = new URL(url);
    const isLoopback = parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost";
    const currentHost = window.location.hostname;
    if (isLoopback && currentHost !== "127.0.0.1" && currentHost !== "localhost") {
      return " The exporter URL points at localhost, which means the browser is trying to connect to its own machine rather than the remote host running Avena.";
    }
  } catch {
    // Ignore malformed URLs and fall back to the base error message.
  }

  return "";
}

async function streamExportFromUrl(
  url: string,
  payload: ExportRequestPayload,
  options: ExportStreamOptions
): Promise<ExportStreamResult> {
  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";

  const chunks: ArrayBuffer[] = [];
  let meta: MetaFrame | null = null;
  let summary: SummaryFrame | null = null;
  let totalBytes = 0;

  return new Promise<ExportStreamResult>((resolve, reject) => {
    let settled = false;

    const cleanup = () => {
      ws.onerror = null;
      ws.onclose = null;
      ws.onmessage = null;
      ws.onopen = null;
    };

    const fail = (message: string) => {
      if (settled) return;
      settled = true;
      cleanup();
      try {
        if (ws.readyState === WebSocket.CONNECTING || ws.readyState === WebSocket.OPEN) {
          ws.close();
        }
      } catch {
        // Best-effort cleanup only.
      }
      reject(new Error(message));
    };

    const succeed = (result: ExportStreamResult) => {
      if (settled) return;
      settled = true;
      cleanup();
      try {
        if (ws.readyState === WebSocket.OPEN) {
          ws.close(1000, "complete");
        }
      } catch {
        // Best-effort cleanup only.
      }
      resolve(result);
    };

    ws.onerror = () => {
      fail(`WebSocket connection failed for ${url}.${buildWebSocketHint(url)}`);
    };

    ws.onclose = (event) => {
      if (settled || event.code === 1000) {
        return;
      }
      const suffix = event.reason ? `: ${event.reason}` : "";
      fail(`WebSocket closed unexpectedly for ${url} (${event.code}${suffix}).${buildWebSocketHint(url)}`);
    };

    ws.onmessage = async (event) => {
      if (typeof event.data === "string") {
        try {
          const frame = JSON.parse(event.data) as Frame;
          if (frame.type === "meta") {
            meta = frame;
            return;
          }

          if (frame.type === "summary") {
            summary = frame;
            options.onSummary?.(frame.missingChannels ?? []);
            return;
          }

          if (frame.type === "complete") {
            const fileName = meta?.fileName ?? payload.download_name ?? "labjack_export.csv";
            const mime = meta?.contentType ?? "text/csv";
            const blob = new Blob(chunks, { type: mime });
            succeed({
              blob,
              fileName,
              size: summary?.bytesSent ?? totalBytes ?? blob.size,
              missingChannels: summary?.missingChannels ?? [],
            });
            return;
          }

          if (frame.type === "error") {
            fail(`Export server error from ${url}: ${frame.message}`);
          }
        } catch (err) {
          fail(
            err instanceof Error
              ? `Failed to parse export frame from ${url}: ${err.message}`
              : `Failed to parse export frame from ${url}`
          );
        }
        return;
      }

      try {
        const buffer =
          event.data instanceof ArrayBuffer
            ? event.data
            : await (event.data as Blob).arrayBuffer();
        chunks.push(buffer);
        totalBytes += buffer.byteLength;
        options.onProgress?.(totalBytes);
      } catch (err) {
        fail(
          err instanceof Error
            ? `Failed to read export data from ${url}: ${err.message}`
            : `Failed to read export data from ${url}`
        );
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
}

export async function downloadExportViaWebSocket(
  payload: ExportRequestPayload,
  options: ExportStreamOptions = {}
): Promise<ExportStreamResult> {
  const candidates = buildExportCandidates(options.websocketUrl);
  let lastError: Error | null = null;

  for (const url of candidates) {
    try {
      return await streamExportFromUrl(url, payload, options);
    } catch (err) {
      lastError = err instanceof Error ? err : new Error(String(err));
      console.error(`Export attempt failed for ${url}`, lastError);
    }
  }

  throw lastError ?? new Error("No export WebSocket endpoint is configured.");
}
