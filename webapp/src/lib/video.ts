export interface VideoClipRequestPayload {
  asset: number;
  camera_id?: string;
  center_time: string;
  pre_sec?: number;
  post_sec?: number;
}

export interface VideoClipResult {
  blob: Blob;
  filename: string;
}

export interface VideoCameraCoverage {
  camera_id: string;
  latest_start: string;
  latest_end: string;
  recommended_center_max: string;
}

export interface VideoCameraListResult {
  cameras: string[];
  coverage: VideoCameraCoverage[];
  default_clip_pre_sec: number;
  default_clip_post_sec: number;
}

export async function fetchVideoCameras(asset: number): Promise<VideoCameraListResult> {
  const response = await fetch(`/api/video/cameras?asset=${encodeURIComponent(String(asset))}`);
  if (!response.ok) {
    let message = `Video camera request failed (${response.status})`;
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

  const body = await response.json();
  const cameras = Array.isArray(body?.cameras)
    ? body.cameras.filter((value: unknown): value is string => typeof value === 'string')
    : [];

  const coverage = Array.isArray(body?.coverage)
    ? body.coverage
        .filter((entry: unknown): entry is Record<string, unknown> => !!entry && typeof entry === 'object')
        .map((entry): VideoCameraCoverage | null => {
          const camera_id = typeof entry.camera_id === 'string' ? entry.camera_id : '';
          const latest_start = typeof entry.latest_start === 'string' ? entry.latest_start : '';
          const latest_end = typeof entry.latest_end === 'string' ? entry.latest_end : '';
          const recommended_center_max =
            typeof entry.recommended_center_max === 'string' ? entry.recommended_center_max : '';
          if (!camera_id) return null;
          return { camera_id, latest_start, latest_end, recommended_center_max };
        })
        .filter((entry: VideoCameraCoverage | null): entry is VideoCameraCoverage => entry !== null)
    : [];

  return {
    cameras,
    coverage,
    default_clip_pre_sec:
      typeof body?.default_clip_pre_sec === 'number' ? body.default_clip_pre_sec : 5,
    default_clip_post_sec:
      typeof body?.default_clip_post_sec === 'number' ? body.default_clip_post_sec : 5,
  };
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
