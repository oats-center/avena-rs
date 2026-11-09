/**
 * Plays a live fragmented-MP4 stream (the webapp's `/camera` route) in a `<video>`
 * through Media Source Extensions.
 *
 * The stream is read with `fetch` and appended chunk by chunk. Playback stays near
 * the newest frame: when it falls more than {@link MAX_LAG_SECONDS} behind (a slow
 * link delivers in bursts) it jumps forward, and already-played video beyond
 * {@link KEEP_SECONDS} is removed so memory stays flat. When the stream ends or fails
 * it reconnects after {@link RETRY_MS}, until stopped.
 */

/** Seconds behind the newest buffered frame before playback jumps forward. */
export const MAX_LAG_SECONDS = 1.5;

/** Seconds of already-played video kept in the buffer. */
export const KEEP_SECONDS = 10;

/** Delay before reconnecting after the stream ends or fails, in ms. */
const RETRY_MS = 2000;

/**
 * Decides whether playback should jump to stay live.
 *
 * @param currentTime - The video's playback position, in seconds.
 * @param bufferedStart - Start of the buffered range, in seconds.
 * @param bufferedEnd - End of the buffered range, in seconds.
 * @returns The position to jump to (just before the newest frame), or `null` to keep
 *   playing: when playback is inside the buffer and at most {@link MAX_LAG_SECONDS}
 *   behind its end.
 *
 * @example
 * liveSeekTarget(10, 0, 11) // null
 * liveSeekTarget(10, 0, 14) // 13.8
 * liveSeekTarget(0, 2.5, 3) // 2.8 (the stream does not start at 0)
 */
export function liveSeekTarget(
    currentTime: number,
    bufferedStart: number,
    bufferedEnd: number
): number | null {
    if (currentTime >= bufferedStart && bufferedEnd - currentTime <= MAX_LAG_SECONDS) return null;
    return Math.max(bufferedStart, bufferedEnd - 0.2);
}

/**
 * Starts playing `url` into `video` and keeps it live until stopped.
 *
 * @param video - The element to play into; it should be muted so autoplay is allowed.
 * @param url - A same-origin URL serving fragmented MP4 with its codecs in the
 *   `Content-Type` header.
 * @param onLive - Called with `true` once frames are playing and `false` while
 *   (re)connecting.
 * @returns A function that stops playback, ends the request and stops reconnecting.
 */
export function playLiveMp4(
    video: HTMLVideoElement,
    url: string,
    onLive: (live: boolean) => void
): () => void {
    const abort = new AbortController();
    void (async () => {
        while (!abort.signal.aborted) {
            try {
                await streamOnce(video, url, abort.signal, onLive);
            } catch (err) {
                if (!abort.signal.aborted) console.warn("Camera stream interrupted:", err);
            }
            onLive(false);
            await new Promise((resolve) => setTimeout(resolve, RETRY_MS));
        }
    })();
    return () => abort.abort();
}

/**
 * Plays one connection of the stream until it ends, fails or is aborted.
 *
 * @throws When the response is not playable or appending fails.
 */
async function streamOnce(
    video: HTMLVideoElement,
    url: string,
    signal: AbortSignal,
    onLive: (live: boolean) => void
): Promise<void> {
    const response = await fetch(url, { signal, cache: "no-store" });
    const type = response.headers.get("content-type") ?? "";
    if (!response.ok || !response.body || !MediaSource.isTypeSupported(type)) {
        throw new Error(`unplayable camera stream (${response.status} ${type})`);
    }
    const media = new MediaSource();
    const objectUrl = URL.createObjectURL(media);
    video.src = objectUrl;
    try {
        await nextEvent(media, "sourceopen");
        const buffer = media.addSourceBuffer(type);
        const reader = response.body.getReader();
        for (;;) {
            const { done, value } = await reader.read();
            if (done || signal.aborted) return;
            buffer.appendBuffer(value);
            await nextEvent(buffer, "updateend");
            if (!buffer.buffered.length) continue;
            const start = buffer.buffered.start(0);
            const end = buffer.buffered.end(buffer.buffered.length - 1);
            const seek = liveSeekTarget(video.currentTime, start, end);
            if (seek !== null) video.currentTime = seek;
            if (video.currentTime - start > 2 * KEEP_SECONDS) {
                buffer.remove(start, video.currentTime - KEEP_SECONDS);
                await nextEvent(buffer, "updateend");
            }
            if (video.paused) await video.play().catch(() => undefined);
            onLive(!video.paused);
        }
    } finally {
        URL.revokeObjectURL(objectUrl);
        video.removeAttribute("src");
        video.load();
    }
}

/** Resolves on the next `type` event of `target`. */
function nextEvent(target: EventTarget, type: string): Promise<void> {
    return new Promise((resolve) => target.addEventListener(type, () => resolve(), { once: true }));
}
