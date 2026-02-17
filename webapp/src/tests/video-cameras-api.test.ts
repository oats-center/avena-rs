import { afterEach, describe, expect, it, vi } from 'vitest';
import { GET } from '../routes/api/video/cameras/+server';

const ORIGINAL_EXPORTER_HTTP_URL = process.env.EXPORTER_HTTP_URL;

function restoreExporterUrl() {
	if (ORIGINAL_EXPORTER_HTTP_URL === undefined) {
		delete process.env.EXPORTER_HTTP_URL;
		return;
	}
	process.env.EXPORTER_HTTP_URL = ORIGINAL_EXPORTER_HTTP_URL;
}

function buildEvent(url: string, fetchImpl: typeof fetch) {
	return {
		url: new URL(url),
		fetch: fetchImpl
	} as any;
}

afterEach(() => {
	restoreExporterUrl();
	vi.restoreAllMocks();
});

describe('GET /api/video/cameras', () => {
	it('returns 400 for invalid asset', async () => {
		const response = await GET(
			buildEvent('http://localhost/api/video/cameras?asset=abc', vi.fn() as any)
		);
		expect(response.status).toBe(400);
		expect(await response.json()).toEqual({
			error: 'asset query param must be a positive integer'
		});
	});

	it('returns 500 when EXPORTER_HTTP_URL is missing', async () => {
		delete process.env.EXPORTER_HTTP_URL;
		const response = await GET(
			buildEvent('http://localhost/api/video/cameras?asset=1001', vi.fn() as any)
		);
		expect(response.status).toBe(500);
		expect(await response.json()).toEqual({ error: 'EXPORTER_HTTP_URL is not set' });
	});

	it('returns 502 when upstream fetch throws', async () => {
		process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
		const mockFetch = vi.fn(async () => {
			throw new Error('connect ECONNREFUSED');
		}) as unknown as typeof fetch;
		const response = await GET(
			buildEvent('http://localhost/api/video/cameras?asset=1001', mockFetch)
		);
		expect(response.status).toBe(502);
		const body = await response.json();
		expect(body.error).toContain('Failed to reach exporter video API');
		expect(body.error).toContain('connect ECONNREFUSED');
	});

	it('returns upstream status when exporter returns an error payload', async () => {
		process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
		const mockFetch = vi.fn(async () => {
			return new Response(JSON.stringify({ error: 'asset not found' }), {
				status: 404,
				headers: { 'Content-Type': 'application/json' }
			});
		}) as unknown as typeof fetch;
		const response = await GET(
			buildEvent('http://localhost/api/video/cameras?asset=9999', mockFetch)
		);
		expect(response.status).toBe(404);
		expect(await response.json()).toEqual({ error: 'asset not found' });
	});

	it('normalizes exporter response', async () => {
		process.env.EXPORTER_HTTP_URL = 'http://127.0.0.1:9001';
		const mockFetch = vi.fn(async () => {
			return new Response(
				JSON.stringify({
					cameras: ['cam11', 123, 'cam10'],
					coverage: [
						{
							camera_id: 'cam11',
							latest_start: '2026-02-17T10:00:00Z',
							latest_end: '2026-02-17T10:00:05Z',
							recommended_center_min: '2026-02-17T10:00:01Z',
							recommended_center_max: '2026-02-17T10:00:04Z',
							contiguous_start: '2026-02-17T09:59:00Z',
							contiguous_end: '2026-02-17T10:00:05Z'
						},
						{
							camera_id: '',
							latest_start: 'bad'
						}
					],
					default_clip_pre_sec: 7,
					default_clip_post_sec: 8
				}),
				{
					status: 200,
					headers: { 'Content-Type': 'application/json' }
				}
			);
		}) as unknown as typeof fetch;

		const response = await GET(
			buildEvent('http://localhost/api/video/cameras?asset=1001', mockFetch)
		);
		expect(response.status).toBe(200);
		expect(await response.json()).toEqual({
			asset: 1001,
			cameras: ['cam11', 'cam10'],
			default_clip_pre_sec: 7,
			default_clip_post_sec: 8,
			coverage: [
				{
					camera_id: 'cam11',
					latest_start: '2026-02-17T10:00:00Z',
					latest_end: '2026-02-17T10:00:05Z',
					recommended_center_min: '2026-02-17T10:00:01Z',
					recommended_center_max: '2026-02-17T10:00:04Z',
					contiguous_start: '2026-02-17T09:59:00Z',
					contiguous_end: '2026-02-17T10:00:05Z'
				}
			]
		});
	});
});
