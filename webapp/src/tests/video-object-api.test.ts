import { afterEach, describe, expect, it, vi } from 'vitest';
import { POST } from '../routes/api/video/object/+server';

const ORIGINAL_EXPORTER_HTTP_URL = process.env.EXPORTER_HTTP_URL;

function restoreExporterUrl() {
  if (ORIGINAL_EXPORTER_HTTP_URL === undefined) {
    delete process.env.EXPORTER_HTTP_URL;
    return;
  }
  process.env.EXPORTER_HTTP_URL = ORIGINAL_EXPORTER_HTTP_URL;
}

function buildEvent(request: Request, fetchImpl: typeof fetch) {
  return {
    request,
    fetch: fetchImpl
  } as any;
}

function jsonRequest(body: unknown) {
  return new Request('http://localhost/api/video/object', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  });
}

afterEach(() => {
  restoreExporterUrl();
  vi.restoreAllMocks();
});

describe('POST /api/video/object', () => {
  it('returns 400 for invalid JSON', async () => {
    const request = new Request('http://localhost/api/video/object', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{not-json'
    });
    const response = await POST(buildEvent(request, vi.fn() as any));
    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({ error: 'Invalid JSON body' });
  });

  it('validates payload shape', async () => {
    const request = jsonRequest({ object_key: '' });
    const response = await POST(buildEvent(request, vi.fn() as any));
    expect(response.status).toBe(400);
    expect(await response.json()).toEqual({
      error: 'object_key must be a non-empty string'
    });
  });

  it('returns 500 when EXPORTER_HTTP_URL is missing', async () => {
    delete process.env.EXPORTER_HTTP_URL;
    const request = jsonRequest({ object_key: 'clips/asset1001/camera_cam11/C_test.mp4' });
    const response = await POST(buildEvent(request, vi.fn() as any));
    expect(response.status).toBe(500);
    expect(await response.json()).toEqual({ error: 'EXPORTER_HTTP_URL is not set' });
  });

  it('returns 502 when upstream fetch fails', async () => {
    process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
    const request = jsonRequest({ object_key: 'clips/asset1001/camera_cam11/C_test.mp4' });
    const mockFetch = vi.fn(async () => {
      throw new Error('connect ECONNREFUSED');
    }) as unknown as typeof fetch;
    const response = await POST(buildEvent(request, mockFetch));
    expect(response.status).toBe(502);
    const body = await response.json();
    expect(body.error).toContain('Failed to reach exporter video API');
    expect(body.error).toContain('connect ECONNREFUSED');
  });

  it('returns upstream error response with payload message', async () => {
    process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
    const request = jsonRequest({ object_key: 'clips/asset1001/camera_cam11/C_missing.mp4' });
    const mockFetch = vi.fn(async () => {
      return new Response(JSON.stringify({ error: 'video object missing' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' }
      });
    }) as unknown as typeof fetch;
    const response = await POST(buildEvent(request, mockFetch));
    expect(response.status).toBe(404);
    expect(await response.json()).toEqual({ error: 'video object missing' });
  });

  it('proxies successful video object body and headers', async () => {
    process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
    const request = jsonRequest({ object_key: 'clips/asset1001/camera_cam11/C_test.mp4' });
    const clipBytes = Uint8Array.from([1, 2, 3, 4]);
    const mockFetch = vi.fn(async () => {
      return new Response(clipBytes, {
        status: 200,
        headers: {
          'Content-Type': 'video/mp4',
          'Content-Disposition': 'attachment; filename="C_test.mp4"'
        }
      });
    }) as unknown as typeof fetch;

    const response = await POST(buildEvent(request, mockFetch));
    expect(response.status).toBe(200);
    expect(response.headers.get('Content-Type')).toBe('video/mp4');
    expect(response.headers.get('Content-Disposition')).toContain('C_test.mp4');
    expect(new Uint8Array(await response.arrayBuffer())).toEqual(clipBytes);
  });
});
