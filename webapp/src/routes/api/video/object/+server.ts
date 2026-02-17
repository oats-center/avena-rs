import { json, type RequestHandler } from '@sveltejs/kit';

interface VideoObjectRequest {
  object_key: string;
}

export const POST: RequestHandler = async ({ request, fetch }) => {
  let payload: VideoObjectRequest;
  try {
    payload = await request.json();
  } catch {
    return json({ error: 'Invalid JSON body' }, { status: 400 });
  }

  if (!payload || typeof payload !== 'object') {
    return json({ error: 'Request body is required' }, { status: 400 });
  }

  if (!payload.object_key || typeof payload.object_key !== 'string' || payload.object_key.trim().length === 0) {
    return json({ error: 'object_key must be a non-empty string' }, { status: 400 });
  }

  const exporterBase = process.env.EXPORTER_HTTP_URL;
  if (!exporterBase) {
    return json({ error: 'EXPORTER_HTTP_URL is not set' }, { status: 500 });
  }

  let upstream: Response;
  try {
    upstream = await fetch(`${exporterBase}/video/object`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(payload),
    });
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    return json(
      { error: `Failed to reach exporter video API at ${exporterBase}/video/object: ${reason}` },
      { status: 502 }
    );
  }

  if (!upstream.ok) {
    let errorMessage = `Video object request failed (${upstream.status})`;
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
