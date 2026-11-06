<!--
@component
Live camera view: plays the fragmented-MP4 stream at `src` (the webapp's `/camera`
route) with `playLiveMp4`. The stream runs only while the view is mounted and the tab
is visible, so the box connects to the camera only while someone is watching. While
(re)connecting, the last frame is dimmed under a spinner. Double-click for fullscreen.
-->
<script lang="ts">
    import { onMount } from "svelte";
    import { playLiveMp4 } from "$lib/camera";

    let { src }: { src: string } = $props();

    let video: HTMLVideoElement;
    let live = $state(false);

    onMount(() => {
        let stop: (() => void) | null = null;
        const sync = () => {
            stop?.();
            stop = document.hidden ? null : playLiveMp4(video, src, (value) => (live = value));
        };
        sync();
        document.addEventListener("visibilitychange", sync);
        return () => {
            document.removeEventListener("visibilitychange", sync);
            stop?.();
        };
    });
</script>

<div class="relative aspect-[7/4] overflow-hidden rounded-box bg-black shadow-xl">
    <video
        bind:this={video}
        class="h-full w-full transition-opacity"
        class:opacity-40={!live}
        muted
        playsinline
        aria-label="Camera"
        ondblclick={() => video.requestFullscreen()}
    ></video>
    {#if !live}
        <span class="loading loading-spinner loading-lg absolute inset-0 m-auto text-white"></span>
    {/if}
</div>
