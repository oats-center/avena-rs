import { json, type RequestHandler } from '@sveltejs/kit';

interface VideoClipRequest {
  asset: number;
  center_time: string;
  pre_sec?: number;
  post_sec?: number;
}

export const POST: RequestHandler = async ({ request, fetch }) => {
  let payload: VideoClipRequest;
  try {
    payload = await request.json();
  } catch {
    return json({ error: 'Invalid JSON body' }, { status: 400 });
  }

  if (!payload || typeof payload !== 'object') {
    return json({ error: 'Request body is required' }, { status: 400 });
  }

  if (!Number.isInteger(payload.asset) || payload.asset <= 0) {
    return json({ error: 'asset must be a positive integer' }, { status: 400 });
  }

  if (!payload.center_time || typeof payload.center_time !== 'string') {
    return json({ error: 'center_time must be an RFC3339 string' }, { status: 400 });
  }

  if (payload.pre_sec !== undefined && (typeof payload.pre_sec !== 'number' || !Number.isFinite(payload.pre_sec) || payload.pre_sec < 0)) {
    return json({ error: 'pre_sec must be a non-negative number' }, { status: 400 });
  }

  if (payload.post_sec !== undefined && (typeof payload.post_sec !== 'number' || !Number.isFinite(payload.post_sec) || payload.post_sec < 0)) {
    return json({ error: 'post_sec must be a non-negative number' }, { status: 400 });
  }

  const exporterBase = process.env.EXPORTER_HTTP_URL ?? 'http://127.0.0.1:9001';
  const upstream = await fetch(`${exporterBase}/video/clip`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(payload),
  });

  if (!upstream.ok) {
    let errorMessage = `Video clip request failed (${upstream.status})`;
    try {
      const body = await upstream.json();
      if (body?.error && typeof body.error === 'string') {
        errorMessage = body.error;
      }
    } catch {
      const text = await upstream.text();
      if (text) errorMessage = text;
    }

    return json({ error: errorMessage }, { status: upstream.status });
  }

  const headers = new Headers();
  const contentType = upstream.headers.get('Content-Type') ?? 'video/mp4';
  headers.set('Content-Type', contentType);

  const disposition = upstream.headers.get('Content-Disposition');
  if (disposition) {
    headers.set('Content-Disposition', disposition);
  }

  return new Response(upstream.body, {
    status: upstream.status,
    headers,
  });
};
