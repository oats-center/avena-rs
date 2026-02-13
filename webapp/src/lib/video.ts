export interface VideoClipRequestPayload {
  asset: number;
  center_time: string;
  pre_sec?: number;
  post_sec?: number;
}

export interface VideoClipResult {
  blob: Blob;
  filename: string;
}

function parseFilenameFromDisposition(disposition: string | null): string | null {
  if (!disposition) return null;

  const utf8Match = disposition.match(/filename\*=UTF-8''([^;]+)/i);
  if (utf8Match?.[1]) {
    try {
      return decodeURIComponent(utf8Match[1]);
    } catch {
      return utf8Match[1];
    }
  }

  const quotedMatch = disposition.match(/filename="([^"]+)"/i);
  if (quotedMatch?.[1]) return quotedMatch[1];

  const plainMatch = disposition.match(/filename=([^;]+)/i);
  return plainMatch?.[1]?.trim() ?? null;
}

export async function requestVideoClip(
  payload: VideoClipRequestPayload
): Promise<VideoClipResult> {
  const response = await fetch('/api/video/clip', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(payload),
  });

  if (!response.ok) {
    let message = `Video clip request failed (${response.status})`;
    try {
      const body = await response.json();
      if (body?.error && typeof body.error === 'string') {
        message = body.error;
      }
    } catch {
      const text = await response.text();
      if (text) message = text;
    }
    throw new Error(message);
  }

  const blob = await response.blob();
  const filename =
    parseFilenameFromDisposition(response.headers.get('Content-Disposition')) ??
    `clip_asset${payload.asset}.mp4`;

  return {
    blob,
    filename,
  };
}
