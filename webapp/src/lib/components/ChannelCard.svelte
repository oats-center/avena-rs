<script lang="ts">
    import RealTimePlot from "$lib/components/RealTimePlot.svelte";
    import { seedManualYLimits, type AxisLimits } from "$lib/plot/axis";
    import { channelStatusLabel, isTriggerMode, plotConfigFor, type ChannelPlotMode } from "$lib/plot/channel";
    import type { ChannelView } from "$lib/plot/channel-view.svelte";
    import type { ChannelUnitInfo } from "$lib/plot/units";

    /** Component props. See the `@component` block below. */
    interface Props {
        /** State of the channel shown. */
        view: ChannelView;
        /** The channel's `data_formats` entry, shown as a badge. */
        dataFormat: string | undefined;
        /** Unit of the plotted values and whether they are calibrated. */
        unitInfo: ChannelUnitInfo;
        /**
         * Called after a change that alters what the plot shows (mode, axis, re-arm),
         * so the page copies a fresh snapshot on the next frame.
         */
        onchange: () => void;
    }

    let { view, dataFormat, unitInfo, onchange }: Props = $props();

    /** The plot, asked for its current y range when autoscale is turned off. */
    let plot = $state<ReturnType<typeof RealTimePlot> | undefined>();

    const channel = $derived(view.channel);
    const plotConfig = $derived(plotConfigFor(view.mode, view.liveData, view.capture, view.trigger));
    const triggerMode = $derived(isTriggerMode(view.mode));

    /**
     * Applies a typed axis limit; a rejected value is put back into the input.
     *
     * @param field - Limit being edited.
     * @param input - The input element.
     */
    function commitAxisLimit(field: keyof AxisLimits, input: HTMLInputElement) {
        const restore = view.commitAxisLimit(field, input.value);
        if (restore === null) onchange();
        else input.value = restore;
    }

    /**
     * Turns autoscale on or off. Turning it off starts Y Min and Y Max from the range
     * the plot shows (see `seedManualYLimits`), so the plot does not jump.
     *
     * @param autoY - New checkbox state.
     */
    function setAutoY(autoY: boolean) {
        if (autoY) {
            view.updateAxis({ autoY: true });
        } else {
            const limits = seedManualYLimits(plot?.getDisplayedYRange() ?? null, view.axis);
            view.updateAxis({ autoY: false, ...limits });
        }
        onchange();
    }

    /**
     * Switches between filtered and raw values and redraws.
     *
     * @param show - `true` for filtered.
     */
    function setShowFiltered(show: boolean) {
        view.setShowFiltered(show);
        onchange();
    }

    /**
     * Applies a typed trigger number; a rejected value is put back into the input.
     *
     * @param field - Setting being edited.
     * @param input - The input element.
     */
    function commitTriggerNumber(field: "threshold" | "postTriggerWindowSec", input: HTMLInputElement) {
        const restore = view.commitTriggerNumber(field, input.value);
        if (restore !== null) input.value = restore;
    }
</script>

<!--
@component
One plotted channel on the plot page: a header with the data format, unit tag and
trigger state, the Mode & Axis panel, the Trigger Settings panel (trigger modes
only), and the `RealTimePlot`. All state lives in the `ChannelView` passed in; this
component only edits it.
-->
<div class="card bg-base-100 shadow-xl border border-base-200">
    <div class="card-body">
        <!-- Channel Header -->
        <div class="flex flex-col gap-4 md:flex-row md:items-center md:justify-between mb-6">
            <h3 class="card-title text-base-content">Channel {channel}</h3>
            <div class="flex flex-wrap items-center gap-3">
                <div class="badge badge-outline badge-sm">
                    {dataFormat}
                </div>
                <!-- Whether the plotted values are raw volts or calibrated, and into which unit. -->
                <div
                    class="badge badge-sm {unitInfo.warning ? 'badge-warning' : unitInfo.calibrated ? 'badge-secondary' : 'badge-ghost'}"
                    title={unitInfo.warning ?? ""}
                >
                    {unitInfo.tag}
                </div>
                <span class="badge badge-info badge-sm">
                    {channelStatusLabel(view.mode, view.prebufferReady, view.triggered)}
                </span>
                {#if view.filter}
                    <!-- Filtered / Raw view switch; not saved to the config. -->
                    <div class="join" role="group" aria-label="Channel {channel} plotted values">
                        <button
                            type="button"
                            class="btn btn-xs join-item {view.showFiltered ? 'btn-success' : ''}"
                            aria-pressed={view.showFiltered}
                            onclick={() => setShowFiltered(true)}
                        >Filtered</button>
                        <button
                            type="button"
                            class="btn btn-xs join-item {!view.showFiltered ? 'btn-neutral' : ''}"
                            aria-pressed={!view.showFiltered}
                            onclick={() => setShowFiltered(false)}
                        >Raw</button>
                    </div>
                {/if}
                {#if view.triggered}
                    <span class="text-xs text-success">
                        Triggered at {new Date(view.triggerTime).toLocaleTimeString()}
                    </span>
                {/if}
            </div>
        </div>

        {#if unitInfo.warning}
            <p class="text-sm text-warning -mt-4 mb-4">{unitInfo.warning}</p>
        {/if}

        {#if view.filterPlan}
            <p class="text-xs text-base-content/70 -mt-4 mb-4" data-testid="filter-note-{channel}">
                Filters: {view.filterDescription}.
                {#if view.filter}
                    {view.showFiltered ? "Showing filtered values" : "Showing raw values; the filters keep running"}{#if view.filterPlan.delaySamples > 0}; the trace ends {view.filterPlan.delaySamples} samples ({(view.filterPlan.delaySamples * 1000 / view.filterPlan.fs).toFixed(1)} ms) before the newest sample (despike window){/if}.
                {/if}
                {#if view.filterPlan.skipped.length > 0}
                    <span class="text-warning">Not applied at {view.filterPlan.fs} Hz: {view.filterPlan.skipped.join("; ")}.</span>
                {/if}
            </p>
        {/if}

        <div class="mb-6 p-4 bg-base-200 rounded-lg">
            <h4 class="text-md font-medium text-base-content mb-4">Mode & Axis</h4>
            <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-7 gap-4">
                <div class="form-control">
                    <label class="label" for="plot-mode-{channel}">
                        <span class="label-text">Plot Mode</span>
                    </label>
                    <select
                        id="plot-mode-{channel}"
                        value={view.mode}
                        onchange={(e) => {
                            if (e.target instanceof HTMLSelectElement && view.setMode(e.target.value as ChannelPlotMode)) {
                                onchange();
                            }
                        }}
                        class="select select-bordered"
                    >
                        <option value="free_run">Free Run</option>
                        <option value="trigger_normal">Trigger Normal</option>
                        <option value="trigger_single">Trigger Single</option>
                    </select>
                </div>

                <div class="form-control">
                    <label class="label" for="x-window-{channel}">
                        <span class="label-text">X Window (s)</span>
                    </label>
                    <input
                        id="x-window-{channel}"
                        type="number"
                        step="0.1"
                        class="input input-bordered"
                        value={view.axis.xWindowSec}
                        onchange={(e) => {
                            if (e.target instanceof HTMLInputElement) {
                                commitAxisLimit("xWindowSec", e.target);
                            }
                        }}
                    />
                </div>

                <div class="form-control">
                    <label class="label cursor-pointer">
                        <span class="label-text">Auto Y-Scale</span>
                        <input
                            type="checkbox"
                            class="checkbox checkbox-primary"
                            checked={view.axis.autoY}
                            onchange={(e) => {
                                if (e.target instanceof HTMLInputElement) {
                                    setAutoY(e.target.checked);
                                }
                            }}
                        />
                    </label>
                </div>

                <div class="form-control">
                    <label class="label" for="y-min-{channel}">
                        <span class="label-text">Y Min ({unitInfo.unit})</span>
                    </label>
                    <input
                        id="y-min-{channel}"
                        type="number"
                        step="0.01"
                        class="input input-bordered"
                        value={view.axis.autoY ? "" : view.axis.yMin}
                        placeholder={view.axis.autoY ? "auto" : undefined}
                        disabled={view.axis.autoY}
                        onchange={(e) => {
                            if (e.target instanceof HTMLInputElement) {
                                commitAxisLimit("yMin", e.target);
                            }
                        }}
                    />
                </div>

                <div class="form-control">
                    <label class="label" for="y-max-{channel}">
                        <span class="label-text">Y Max ({unitInfo.unit})</span>
                    </label>
                    <input
                        id="y-max-{channel}"
                        type="number"
                        step="0.01"
                        class="input input-bordered"
                        value={view.axis.autoY ? "" : view.axis.yMax}
                        placeholder={view.axis.autoY ? "auto" : undefined}
                        disabled={view.axis.autoY}
                        onchange={(e) => {
                            if (e.target instanceof HTMLInputElement) {
                                commitAxisLimit("yMax", e.target);
                            }
                        }}
                    />
                </div>

                <div class="form-control">
                    <label class="label cursor-pointer">
                        <span class="label-text">Invert X</span>
                        <input
                            type="checkbox"
                            class="checkbox checkbox-primary"
                            checked={view.axis.invertX}
                            onchange={(e) => {
                                if (e.target instanceof HTMLInputElement) {
                                    view.updateAxis({ invertX: e.target.checked });
                                    onchange();
                                }
                            }}
                        />
                    </label>
                </div>

                <div class="form-control">
                    <label class="label cursor-pointer">
                        <span class="label-text">Invert Y</span>
                        <input
                            type="checkbox"
                            class="checkbox checkbox-primary"
                            checked={view.axis.invertY}
                            onchange={(e) => {
                                if (e.target instanceof HTMLInputElement) {
                                    view.updateAxis({ invertY: e.target.checked });
                                    onchange();
                                }
                            }}
                        />
                    </label>
                </div>
            </div>
            {#if view.axisError}
                <p class="text-sm text-error mt-2">{view.axisError}</p>
            {/if}
        </div>

        {#if triggerMode}
            <div class="mb-6 p-4 bg-base-200 rounded-lg">
                <!--
                    The threshold is compared with the plotted (calibrated) values, so it
                    is in the channel's unit, not always volts.
                -->
                <h4 class="text-md font-medium text-base-content mb-4">Trigger Settings</h4>
                <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-5 gap-4">
                    <div class="form-control">
                        <label class="label" for="trigger-type-{channel}">
                            <span class="label-text">Trigger Edge</span>
                        </label>
                        <select
                            id="trigger-type-{channel}"
                            value={view.trigger.type || 'rising'}
                            onchange={(e) => {
                                if (e.target instanceof HTMLSelectElement) {
                                    view.setTriggerEdge(e.target.value as 'rising' | 'falling');
                                }
                            }}
                            class="select select-bordered select-warning"
                        >
                            <option value="rising">Rising Edge</option>
                            <option value="falling">Falling Edge</option>
                        </select>
                    </div>
                    <div class="form-control">
                        <label class="label" for="trigger-threshold-{channel}">
                            <span class="label-text">Threshold ({unitInfo.unit})</span>
                        </label>
                        <input
                            id="trigger-threshold-{channel}"
                            type="number"
                            step="0.01"
                            value={view.trigger.threshold}
                            onchange={(e) => {
                                if (e.target instanceof HTMLInputElement) {
                                    commitTriggerNumber("threshold", e.target);
                                }
                            }}
                            class="input input-bordered input-warning"
                        />
                    </div>
                    <div class="form-control">
                        <label class="label" for="trigger-pre-{channel}">
                            <span class="label-text">Pre Trigger (%)</span>
                        </label>
                        <input
                            id="trigger-pre-{channel}"
                            type="number"
                            min="0"
                            max="95"
                            step="1"
                            value={view.trigger.preTriggerPercent || 0}
                            onchange={(e) => {
                                if (e.target instanceof HTMLInputElement) {
                                    view.setPreTriggerPercent(e.target.value);
                                }
                            }}
                            class="input input-bordered input-warning"
                        />
                    </div>
                    <div class="form-control">
                        <label class="label" for="trigger-post-{channel}">
                            <span class="label-text">Post Window (s)</span>
                        </label>
                        <input
                            id="trigger-post-{channel}"
                            type="number"
                            min="0.01"
                            step="0.1"
                            value={view.trigger.postTriggerWindowSec}
                            onchange={(e) => {
                                if (e.target instanceof HTMLInputElement) {
                                    commitTriggerNumber("postTriggerWindowSec", e.target);
                                }
                            }}
                            class="input input-bordered input-warning"
                        />
                    </div>
                    <div class="form-control justify-end">
                        <button
                            class="btn btn-outline btn-warning mt-8"
                            onclick={() => {
                                view.clearTrigger();
                                onchange();
                            }}
                        >
                            Re-arm Trigger
                        </button>
                    </div>
                </div>
            </div>
        {/if}

        <div>
            <h4 class="text-md font-medium text-base-content mb-4">Data Plot</h4>
            <RealTimePlot
                bind:this={plot}
                data={plotConfig.data}
                unit={unitInfo.unit}
                calibrated={unitInfo.calibrated}
                timeWindow={view.axis.xWindowSec}
                isTriggered={plotConfig.isTriggered}
                triggerTime={plotConfig.triggerTime}
                mode={plotConfig.mode}
                frozenData={plotConfig.frozenData}
                frozenPreWindowSec={plotConfig.frozenPreWindowSec}
                frozenPostWindowSec={plotConfig.frozenPostWindowSec}
                frozenCollecting={plotConfig.frozenCollecting}
                showTriggerThreshold={triggerMode}
                triggerThreshold={view.trigger.threshold}
                prebuffering={triggerMode && !view.triggered && !view.prebufferReady}
                yAutoScale={view.axis.autoY}
                yMin={view.axis.yMin}
                yMax={view.axis.yMax}
                invertX={view.axis.invertX}
                invertY={view.axis.invertY}
                tag={view.filter && view.showFiltered ? "FILTERED" : ""}
            />
        </div>
    </div>
</div>
