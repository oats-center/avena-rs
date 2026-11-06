/**
 * Relays the live camera stream named by the server's `CAMERA_STREAM_URL`: a
 * fragmented-MP4 URL with its codecs in `Content-Type`, such as go2rtc's
 * `http://127.0.0.1:1984/api/stream.mp4?src=camera&video`. The browser reaches the
 * camera through the webapp's own origin and never sees the relay or the camera's
 * credentials. Answers 404 when the variable is unset, so the route is off by default.
 * The upstream request ends when the browser disconnects, so the relay can drop its
 * camera connection.
 */
import { error } from "@sveltejs/kit";
import { env } from "$env/dynamic/private";
import type { RequestHandler } from "./$types";

export const GET: RequestHandler = async ({ request }) => {
    const upstream = env.CAMERA_STREAM_URL;
    if (!upstream) error(404, "No camera configured");
    const response = await fetch(upstream, { signal: request.signal });
    if (!response.ok || !response.body) error(502, "Camera stream unavailable");
    return new Response(response.body, {
        headers: {
            "content-type": response.headers.get("content-type") ?? "video/mp4",
            "cache-control": "no-store"
        }
    });
};
