import { json, type RequestHandler } from '@sveltejs/kit';

export const GET: RequestHandler = async ({ url, fetch }) => {
  const assetRaw = url.searchParams.get('asset');
  const asset = Number(assetRaw);
  if (!assetRaw || !Number.isInteger(asset) || asset <= 0) {
    return json({ error: 'asset query param must be a positive integer' }, { status: 400 });
  }

  const exporterBase = process.env.EXPORTER_HTTP_URL;
  if (!exporterBase) {
    return json({ error: 'EXPORTER_HTTP_URL is not set' }, { status: 500 });
  }

  let upstream: Response;
  try {
    upstream = await fetch(
      `${exporterBase}/video/cameras?asset=${encodeURIComponent(String(asset))}`
    );
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    return json(
      { error: `Failed to reach exporter video API at ${exporterBase}/video/cameras: ${reason}` },
      { status: 502 }
    );
  }

  let body: any = null;
  try {
    body = await upstream.json();
  } catch {
    return json({ error: 'Invalid response from exporter' }, { status: 502 });
  }

  if (!upstream.ok) {
    return json(
      { error: body?.error ?? `Video camera request failed (${upstream.status})` },
      { status: upstream.status }
    );
  }

  const cameras = Array.isArray(body?.cameras)
    ? body.cameras.filter((value: unknown): value is string => typeof value === 'string')
    : [];

  const coverage = Array.isArray(body?.coverage)
    ? body.coverage
        .filter((entry: unknown): entry is Record<string, unknown> => !!entry && typeof entry === 'object')
        .map((entry) => ({
          camera_id: typeof entry.camera_id === 'string' ? entry.camera_id : '',
          latest_start: typeof entry.latest_start === 'string' ? entry.latest_start : '',
          latest_end: typeof entry.latest_end === 'string' ? entry.latest_end : '',
          recommended_center_min:
            typeof entry.recommended_center_min === 'string' ? entry.recommended_center_min : '',
          recommended_center_max:
            typeof entry.recommended_center_max === 'string' ? entry.recommended_center_max : '',
          contiguous_start: typeof entry.contiguous_start === 'string' ? entry.contiguous_start : '',
          contiguous_end: typeof entry.contiguous_end === 'string' ? entry.contiguous_end : '',
        }))
        .filter((entry) => entry.camera_id.length > 0)
    : [];

  return json({
    asset,
    cameras,
    default_clip_pre_sec:
      typeof body?.default_clip_pre_sec === 'number' ? body.default_clip_pre_sec : 5,
    default_clip_post_sec:
      typeof body?.default_clip_post_sec === 'number' ? body.default_clip_post_sec : 5,
    coverage,
  });
};
