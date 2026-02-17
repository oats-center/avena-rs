import { afterEach, describe, expect, it, vi } from 'vitest';
import { POST } from '../routes/api/video/clip/+server';

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
	return new Request('http://localhost/api/video/clip', {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify(body)
	});
}

afterEach(() => {
	restoreExporterUrl();
	vi.restoreAllMocks();
});

describe('POST /api/video/clip', () => {
	it('returns 400 for invalid JSON', async () => {
		const request = new Request('http://localhost/api/video/clip', {
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: '{not-json'
		});
		const response = await POST(buildEvent(request, vi.fn() as any));
		expect(response.status).toBe(400);
		expect(await response.json()).toEqual({ error: 'Invalid JSON body' });
	});

	it('validates payload shape', async () => {
		const request = jsonRequest({ asset: -1, center_time: '' });
		const response = await POST(buildEvent(request, vi.fn() as any));
		expect(response.status).toBe(400);
		expect(await response.json()).toEqual({
			error: 'asset must be a positive integer'
		});
	});

	it('returns 500 when EXPORTER_HTTP_URL is missing', async () => {
		delete process.env.EXPORTER_HTTP_URL;
		const request = jsonRequest({
			asset: 1001,
			center_time: '2026-02-17T10:00:00Z'
		});
		const response = await POST(buildEvent(request, vi.fn() as any));
		expect(response.status).toBe(500);
		expect(await response.json()).toEqual({ error: 'EXPORTER_HTTP_URL is not set' });
	});

	it('returns 502 when upstream fetch fails', async () => {
		process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
		const request = jsonRequest({
			asset: 1001,
			center_time: '2026-02-17T10:00:00Z'
		});
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
		const request = jsonRequest({
			asset: 1001,
			center_time: '2026-02-17T10:00:00Z',
			camera_id: 'cam11'
		});
		const mockFetch = vi.fn(async () => {
			return new Response(JSON.stringify({ error: 'clip not available' }), {
				status: 409,
				headers: { 'Content-Type': 'application/json' }
			});
		}) as unknown as typeof fetch;
		const response = await POST(buildEvent(request, mockFetch));
		expect(response.status).toBe(409);
		expect(await response.json()).toEqual({ error: 'clip not available' });
	});

	it('proxies successful video clip body and headers', async () => {
		process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
		const request = jsonRequest({
			asset: 1001,
			center_time: '2026-02-17T10:00:00Z',
			camera_id: 'cam11',
			pre_sec: 5,
			post_sec: 5
		});
		const clipBytes = Uint8Array.from([1, 2, 3, 4]);
		const mockFetch = vi.fn(async () => {
			return new Response(clipBytes, {
				status: 200,
				headers: {
					'Content-Type': 'video/mp4',
					'Content-Disposition': 'attachment; filename="clip.mp4"'
				}
			});
		}) as unknown as typeof fetch;

		const response = await POST(buildEvent(request, mockFetch));
		expect(response.status).toBe(200);
		expect(response.headers.get('Content-Type')).toBe('video/mp4');
		expect(response.headers.get('Content-Disposition')).toContain('clip.mp4');
		expect(new Uint8Array(await response.arrayBuffer())).toEqual(clipBytes);
	});
});
