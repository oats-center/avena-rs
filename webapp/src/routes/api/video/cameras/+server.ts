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

  const upstream = await fetch(
    `${exporterBase}/video/cameras?asset=${encodeURIComponent(String(asset))}`
  );

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

  return json({ asset, cameras });
};
