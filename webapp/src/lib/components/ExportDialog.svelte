<script lang="ts">
    import { describeLocalTimeZone, formatBytes, formatUtcPreview } from "$lib/plot/export-form";

    /** Component props. See the `@component` block below. */
    interface Props {
        /** Channels offered, in config order. */
        channels: number[];
        /** Ticked channels. */
        selected: Set<number>;
        /** Start as a `datetime-local` value. Bindable. */
        start: string;
        /** End as a `datetime-local` value. Bindable. */
        end: string;
        /** Error shown in the form, or `""`. */
        error: string;
        /** Missing-channel warning, or `""`. */
        warning: string;
        /** True while a download runs. */
        exporting: boolean;
        /** Bytes received so far. */
        progress: number;
        /** Final size once known, else `null`. */
        total: number | null;
        /** Called when a channel box is ticked or unticked. */
        ontoggle: (channel: number, checked: boolean) => void;
        /** Called on Start Download (form submit). */
        onsubmit: (event: Event) => void;
        /** Called on Cancel, Cancel Download or a backdrop click while idle. */
        onclose: () => void;
    }

    let {
        channels,
        selected,
        start = $bindable(),
        end = $bindable(),
        error,
        warning,
        exporting,
        progress,
        total,
        ontoggle,
        onsubmit,
        onclose
    }: Props = $props();
</script>

<!--
@component
Export Historical Data dialog of the plot page: start and end time (to the second, in
the browser's time zone, with the UTC range shown), the channels, errors and warnings,
and a progress bar while downloading. All state and the download itself live in the
page; this component shows them and reports input.
-->
<div class="modal modal-open">
    <div class="modal-box max-w-2xl">
        <h3 class="font-bold text-lg text-base-content mb-4">Export Historical Data</h3>
        <form class="space-y-5" {onsubmit}>
            <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div class="form-control">
                    <label class="label" for="export-start">
                        <span class="label-text">Start Time</span>
                    </label>
                    <input
                        id="export-start"
                        type="datetime-local"
                        class="input input-bordered"
                        step="1"
                        bind:value={start}
                        max={end || undefined}
                        required
                        disabled={exporting}
                    />
                </div>
                <div class="form-control">
                    <label class="label" for="export-end">
                        <span class="label-text">End Time</span>
                    </label>
                    <input
                        id="export-end"
                        type="datetime-local"
                        class="input input-bordered"
                        step="1"
                        bind:value={end}
                        min={start || undefined}
                        required
                        disabled={exporting}
                    />
                </div>
            </div>

            <p class="text-sm text-base-content/70">
                Times are in your browser's time zone, {describeLocalTimeZone()}.
                {#if formatUtcPreview(start) && formatUtcPreview(end)}
                    Requested range: {formatUtcPreview(start)} to {formatUtcPreview(end)}.
                {/if}
            </p>

            <div>
                <h4 class="font-semibold text-base-content mb-2">Channels</h4>
                <div class="grid grid-cols-2 md:grid-cols-3 gap-2">
                    {#each channels as ch}
                        <label class="flex items-center space-x-2 text-sm">
                            <input
                                type="checkbox"
                                class="checkbox checkbox-warning checkbox-sm"
                                checked={selected.has(ch)}
                                onchange={(event) => ontoggle(ch, (event.target as HTMLInputElement).checked)}
                                disabled={exporting}
                            />
                            <span>Channel {ch}</span>
                        </label>
                    {/each}
                </div>
            </div>

            {#if error}
                <div class="alert alert-error text-sm">
                    <span>{error}</span>
                </div>
            {/if}

            {#if warning}
                <div class="alert alert-warning text-sm">
                    <span>{warning}</span>
                </div>
            {/if}

            {#if exporting}
                <div class="space-y-2">
                    <progress
                        class="progress progress-warning w-full"
                        value={progress}
                        max={total ?? Math.max(progress, 1)}
                    ></progress>
                    <p class="text-sm text-base-content/70">
                        Downloaded {formatBytes(progress)}
                        {#if total}
                            / {formatBytes(total)}
                        {/if}
                    </p>
                </div>
            {/if}

            <div class="modal-action">
                <button
                    type="button"
                    class="btn btn-ghost"
                    onclick={onclose}
                >
                    {exporting ? "Cancel Download" : "Cancel"}
                </button>
                <button
                    type="submit"
                    class="btn btn-warning"
                    disabled={exporting}
                >
                    {exporting ? "Downloading..." : "Start Download"}
                </button>
            </div>
        </form>
    </div>
    <div
        class="modal-backdrop bg-black/40"
        role="button"
        tabindex="0"
        onclick={() => {
            // While downloading, only the Cancel Download button stops it.
            if (!exporting) onclose();
        }}
        onkeydown={(event) => {
            if (!exporting && (event.key === "Escape" || event.key === "Enter" || event.key === " ")) {
                event.preventDefault();
                onclose();
            }
        }}
    ></div>
</div>
