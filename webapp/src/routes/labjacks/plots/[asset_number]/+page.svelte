<script lang="ts">
    import { onMount, onDestroy, untrack } from "svelte";
    import { page } from "$app/stores";
    import { connect, getKeyValue, getKeys, type NatsService } from "$lib/nats.svelte";
    import { downloadExportViaNats, isExportCancelled, type ExportRequestPayload } from "$lib/exporter";
    import { normalizeCalibration } from "$lib/calibration";
    import { normalizeLabJackConfig, type LabJackConfig } from "$lib/labjack-config";
    import { archiveExportRequestSubject, liveLabJackChannelPattern, liveLabJackChannelSubject } from "$lib/subjects";
    import type { Subscription } from "@nats-io/nats-core";
    import RealTimePlot from "$lib/components/RealTimePlot.svelte";
    import { parseAssetNumberParam } from "$lib/plot/route";
    import { applyAxisLimitInput, parseFiniteInput, type AxisLimits } from "$lib/plot/axis";
    import { nextConnectionState, type LiveConnectionState } from "$lib/plot/connection";
    import { describeChannelUnit, type ChannelUnitInfo } from "$lib/plot/units";
    import {
        FlatBufferParser
    } from "$lib/flatbuffer-parser";
    import {
        ScanMessageQueue,
        createLiveChannel,
        drainChannelQueue,
        snapshotNewest,
        type DataPoint,
        type LiveChannel
    } from "$lib/plot/stream";
    import {
        advanceTrigger,
        getTriggerWindows,
        hasRequiredPreBuffer,
        type TriggerCapture,
        type TriggerSettings
    } from "$lib/plot/trigger";

    
    /**
     * Plot mode of one channel.
     *
     * `free_run` scrolls continuously. `trigger_normal` freezes a capture on each threshold
     * crossing and re-arms after the post-trigger window. `trigger_single` keeps the first
     * capture until the user presses Re-arm.
     */
    type ChannelPlotMode = 'free_run' | 'trigger_normal' | 'trigger_single';

    /** Axis settings of one channel, edited in the Mode & Axis panel. */
    interface AxisSettings {
        /** When true the plot scales Y to the data and ignores `yMin` and `yMax`. */
        autoY: boolean;
        /** Lower Y limit when `autoY` is false. */
        yMin: number;
        /**
         * Upper Y limit when `autoY` is false. Kept above `yMin` by {@link
         * commitAxisLimit}.
         */
        yMax: number;
        /** Width of the X axis in seconds. Above 0. */
        xWindowSec: number;
        /** Mirrors the X axis. */
        invertX: boolean;
        /** Mirrors the Y axis. */
        invertY: boolean;
    }

    /**
     * Samples the automatic X window aims to show; the window is this count divided by
     * the scan rate.
     */
    const TARGET_SAMPLES_PER_WINDOW = 1000;
    /**
     * Messages taken from one channel's queue at a time. Channels take turns, so one
     * busy channel cannot starve the other.
     */
    const DECODE_BATCH_MESSAGES = 50;
    /**
     * Decoding time allowed per animation frame, in ms. At least one batch per channel
     * is always decoded; what does not fit stays queued, in order, for the next frame.
     */
    const FRAME_DECODE_BUDGET_MS = 8;
    /** Decoding time allowed per run of the background timer, in ms. */
    const BACKGROUND_DECODE_BUDGET_MS = 50;
    /**
     * Shortest time between two copies of the buffers into reactive state (and so
     * between redraws), in ms: about 30 redraws per second at most.
     */
    const MIN_SNAPSHOT_INTERVAL_MS = 33;
    /**
     * Period of the fallback timer that keeps decoding while the browser runs no
     * animation frames (a background tab), in ms.
     */
    const BACKGROUND_PUMP_INTERVAL_MS = 1000;
    /**
     * Live snapshots cover this multiple of the channel's X window, so the plot edge is
     * never empty.
     */
    const SNAPSHOT_OVERSCAN_FACTOR = 1.25;
    /**
     * Most undecoded messages held per channel between frames. Normally a frame finds at
     * most a few; this is reached only if decoding stalls for a long time (for example a
     * background tab whose timers the browser throttles). Then the oldest are dropped,
     * counted, shown in Data Statistics, and appear as a gap in the plot.
     */
    const MAX_QUEUED_MESSAGES_PER_CHANNEL = 5000;
    
    /** `asset_number` route parameter; `NaN` when it is not a non-negative integer. */
    let assetNumber = $state<number>(0);
    let labjackConfig = $state<LabJackConfig | null>(null);
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    /**
     * What the error banner offers: `retry` (loading again can succeed), `login` (no
     * login data in this tab), or `none` (an invalid asset number, which cannot load).
     */
    let errorAction = $state<"retry" | "login" | "none">("retry");
    /**
     * Connection used for live subscriptions and export requests. Closed before each
     * reload and in `onDestroy`.
     */
    let natsService: any = null;
    /** Incremented by each {@link loadLabJackConfig} call; older calls see they are stale. */
    let loadGeneration = 0;
    /** Set in `onDestroy`, so a load that finishes afterwards closes its connection. */
    let destroyed = false;
    /**
     * One live-data subscription per selected channel, kept in step with the selection
     * by {@link syncChannelSubscriptions}.
     */
    let channelSubscriptions = new Map<number, Subscription>();
    /**
     * Copy of the live buffers the template reads, refreshed by {@link flushUiSnapshots}.
     */
    let channelData = $state<Map<number, DataPoint[]>>(new Map());
    /**
     * Copy of the frozen trigger buffers the template reads, refreshed by {@link
     * flushUiSnapshots}.
     */
    let frozenChannelData = $state<Map<number, DataPoint[]>>(new Map());
    let channelModes = $state<Map<number, ChannelPlotMode>>(new Map());
    let axisSettings = $state<Map<number, AxisSettings>>(new Map());
    /** Per channel, why the last typed axis limit was rejected. */
    let axisInputErrors = $state<Map<number, string>>(new Map());
    /**
     * State of this page's NATS connection, kept up to date by {@link watchConnection}
     * from the client's status events and `closed()` promise.
     */
    let connectionState = $state<LiveConnectionState>("disconnected");
    /** Why the connection closed, shown in the banner when `disconnected` after a loss. */
    let connectionLostReason = $state<string>("");
    /** True while the connection is up and the live subscriptions exist. */
    let isConnected = $derived(connectionState === "connected");
    let flatBufferParser = new FlatBufferParser();
    let triggerSettings = $state<Map<number, TriggerSettings>>(new Map());
    /** Per channel, true while a trigger capture is held. */
    let channelTriggered = $state<Map<number, boolean>>(new Map());
    /**
     * Per channel, time of the last trigger in Unix milliseconds, or 0 when not
     * triggered.
     */
    let channelTriggerTime = $state<Map<number, number>>(new Map());
    /**
     * Per channel, true when the buffer spans the pre-trigger window. Set true in free
     * run.
     */
    let channelPrebufferReady = $state<Map<number, boolean>>(new Map());
    /** Per channel, the current trigger capture, if any. */
    let channelTriggerCaptures = $state<Map<number, TriggerCapture>>(new Map());
    /** Pending `requestAnimationFrame` of the decode and redraw loop. */
    let frameHandle = 0;
    /** Fallback timer that decodes while no animation frames run. */
    let backgroundTimer: ReturnType<typeof setInterval> | null = null;
    /** `performance.now()` of the last decode pass. */
    let lastPumpAt = 0;
    /** `performance.now()` of the last {@link flushUiSnapshots}. */
    let lastFlushAt = 0;
    /**
     * Automatic X window in seconds, from {@link deriveAutoTimeWindowSec}; the default
     * for new channels.
     */
    let timeWindow = $state<number>(1); // seconds
    /** Maximum points kept in each live buffer. Set by {@link updateMaxDataPoints}. */
    let maxDataPoints = $state<number>(10000);
    let showExportModal = $state<boolean>(false);
    /** Export start as a `datetime-local` value (`YYYY-MM-DDTHH:mm:ss`, local time). */
    let exportStart = $state<string>("");
    /** Export end as a `datetime-local` value (`YYYY-MM-DDTHH:mm:ss`, local time). */
    let exportEnd = $state<string>("");
    /** Aborts the running export; `null` when none runs. */
    let exportAbort: AbortController | null = null;
    let exportChannels = $state<Set<number>>(new Set());
    let exportError = $state<string>("");
    /** Exporter's missing-channel notice. Cleared when the download finishes. */
    let exportWarning = $state<string>("");
    let exporting = $state<boolean>(false);
    /** Bytes of CSV received so far. */
    let exportProgress = $state<number>(0);
    /** Final size in bytes. Set only after the download completes; `null` while it runs. */
    let exportTotal = $state<number | null>(null);
    /**
     * Rolling live buffers and stream state, one per channel. Not reactive: they are
     * changed in place on every message and copied into {@link channelData} when they change
     * (at most about 30 times a second). Buffers are sorted by sample time with no duplicates (see `$lib/plot/stream`).
     */
    let liveChannels = new Map<number, LiveChannel>();
    /**
     * Frozen trigger captures, one per channel. Not reactive; copied into {@link
     * frozenChannelData}.
     */
    let frozenChannelBuffers = new Map<number, DataPoint[]>();
    /**
     * Every received, not yet decoded message per selected channel, in arrival order.
     * Drained in order by {@link processPendingVisualizationBatches}.
     */
    let scanQueue = new ScanMessageQueue(MAX_QUEUED_MESSAGES_PER_CHANNEL);
    /**
     * Per-channel stream counters shown in Data Statistics: messages dropped because the
     * queue overflowed, gaps (missing time between messages), timeline resets, samples
     * skipped as duplicates, and undecodable payloads.
     */
    let channelStreamStatus = $state<Map<number, { dropped: number; gaps: number; resets: number; skipped: number; decodeErrors: number }>>(new Map());
    /** Channels chosen for plotting, at most two. Only these are decoded. */
    let selectedPlotChannels = $state<Set<number>>(new Set());
    /** True when buffers changed since the last {@link flushUiSnapshots}. */
    let uiSnapshotDirty = false;
    
    /**
     * Reads `asset_number` and `key` from the URL and loads the config; an asset number
     * that is not a non-negative integer is reported as an error by
     * {@link loadLabJackConfig}. Runs again when the page store changes.
     */
    $effect(() => {
        const nextAssetNumber = parseAssetNumberParam($page.params.asset_number);
        const nextConfigKey = $page.url.searchParams.get('key')?.trim() || "";
        assetNumber = nextAssetNumber ?? Number.NaN;
        untrack(() => loadLabJackConfig());
    });

    /**
     * Sets the automatic X window from the scan rate and resizes the buffers when the config
     * loads. It also reruns on axis or trigger changes, which `updateMaxDataPoints` reads.
     */
    $effect(() => {
        if (labjackConfig) {
            timeWindow = deriveAutoTimeWindowSec(labjackConfig.sensor_settings.scan_rate_hz);
            updateMaxDataPoints();
        }
    });

    /** Resizes the buffers when any channel's axis or trigger settings change. */
    $effect(() => {
        if (!labjackConfig) return;
        // Bare reads register the two maps as dependencies of this effect.
        axisSettings;
        triggerSettings;
        updateMaxDataPoints();
    });

    /**
     * Sets {@link maxDataPoints} to the number of samples the longest needed window holds.
     *
     * The window is the largest of: twice {@link timeWindow}, twice each channel's X window
     * (at least 0.1 s), and each channel's pre plus post trigger window. It is multiplied by
     * the scan rate, plus 10 % and one read of headroom. Then trims the live buffers and marks the snapshot dirty. Does nothing
     * before the config is loaded.
     */
    function updateMaxDataPoints() {
        if (!labjackConfig) return;
        const sr = labjackConfig.sensor_settings.scan_rate_hz;
        let requiredSeconds = timeWindow * 2;

        for (const axis of axisSettings.values()) {
            requiredSeconds = Math.max(requiredSeconds, Math.max(0.1, axis.xWindowSec) * 2);
        }

        for (const trigger of triggerSettings.values()) {
            const windows = getTriggerWindows(trigger);
            requiredSeconds = Math.max(requiredSeconds, windows.preWindowSec + windows.postWindowSec);
        }

        // 10 % and one read of headroom: the actual scan rate can be slightly above the
        // configured one, and gap markers take a slot each. Without it a trigger capture
        // could find the start of its pre-trigger window already trimmed.
        const scansPerRead = Math.max(1, labjackConfig.sensor_settings.scans_per_read || 1);
        maxDataPoints = Math.ceil(sr * requiredSeconds * 1.1) + scansPerRead;
        trimAllChannelBuffers();
        markUiSnapshotDirty();
    }

    /**
     * Returns the automatic X window for a scan rate.
     *
     * @param scanRateHz - Scans per second per channel.
     * @returns {@link TARGET_SAMPLES_PER_WINDOW} divided by the rate, in seconds, at least
     *   0.05. Returns 1 when the rate is not a positive finite number.
     *
     * @example
     * ```ts
     * deriveAutoTimeWindowSec(1000);  // 1
     * deriveAutoTimeWindowSec(50000); // 0.05 (0.02 raised to the minimum)
     * ```
     */
    function deriveAutoTimeWindowSec(scanRateHz: number): number {
        if (!Number.isFinite(scanRateHz) || scanRateHz <= 0) {
            return 1;
        }
        return Math.max(0.05, TARGET_SAMPLES_PER_WINDOW / scanRateHz);
    }

    /** Marks the buffers as changed so the next frame copies them into reactive state. */
    function markUiSnapshotDirty() {
        uiSnapshotDirty = true;
    }

    /**
     * Drops the oldest points from each live buffer so none is longer than {@link
     * maxDataPoints}.
     */
    function trimAllChannelBuffers() {
        for (const live of liveChannels.values()) {
            const data = live.buffer;
            const excess = data.length - maxDataPoints;
            if (excess > 0) {
                data.splice(0, excess);
            }
        }
    }

    /**
     * Copies the newest part of a channel's live buffer for the plot.
     *
     * Keeps the X window times {@link SNAPSHOT_OVERSCAN_FACTOR}, at least 0.25 s. In the
     * trigger modes it keeps at least the pre-trigger window plus 0.25 s. The span is
     * measured back from the newest point's timestamp, not from the clock, and one point
     * before the span is kept so the line reaches the left edge.
     *
     * @param channel - LabJack channel number.
     * @returns A new array, empty when the buffer is empty.
     */
    function snapshotLiveChannelData(channel: number): DataPoint[] {
        const data = liveChannels.get(channel)?.buffer ?? [];
        if (data.length === 0) return [];

        const axisWindowSec = axisSettings.get(channel)?.xWindowSec ?? timeWindow;
        let keepSeconds = Math.max(0.25, axisWindowSec * SNAPSHOT_OVERSCAN_FACTOR);

        const triggerMode = channelModes.get(channel);
        if (triggerMode === "trigger_normal" || triggerMode === "trigger_single") {
            const trigger = triggerSettings.get(channel);
            if (trigger) {
                const windows = getTriggerWindows(trigger);
                keepSeconds = Math.max(keepSeconds, windows.preWindowSec + 0.25);
            }
        }

        return snapshotNewest(data, keepSeconds * 1000);
    }

    /**
     * Copies the live and frozen buffers of every selected channel into {@link channelData}
     * and {@link frozenChannelData}, which triggers a redraw.
     *
     * @param force - Copy even when nothing is marked dirty.
     */
    function flushUiSnapshots(force: boolean = false) {
        if (!force && !uiSnapshotDirty) return;

        const liveSnapshots = new Map<number, DataPoint[]>();
        const frozenSnapshots = new Map<number, DataPoint[]>();

        for (const channel of getRenderablePlotChannels()) {
            liveSnapshots.set(channel, snapshotLiveChannelData(channel));
            frozenSnapshots.set(channel, [...(frozenChannelBuffers.get(channel) || [])]);
        }

        channelData = liveSnapshots;
        frozenChannelData = frozenSnapshots;
        refreshStreamStatus();
        uiSnapshotDirty = false;
    }

    /** Copies the per-channel stream counters into {@link channelStreamStatus}. */
    function refreshStreamStatus() {
        const next = new Map<number, { dropped: number; gaps: number; resets: number; skipped: number; decodeErrors: number }>();
        for (const channel of getRenderablePlotChannels()) {
            const stats = liveChannels.get(channel)?.stats;
            next.set(channel, {
                dropped: scanQueue.droppedMessages.get(channel) ?? 0,
                gaps: stats?.gaps ?? 0,
                resets: stats?.resets ?? 0,
                skipped: stats?.skippedSamples ?? 0,
                decodeErrors: stats?.decodeErrors ?? 0
            });
        }
        channelStreamStatus = next;
    }

    /**
     * Chooses up to two channels to plot after a config load.
     *
     * Keeps channels from the previous selection that are still enabled, then fills up to
     * two from `enabledChannels` in config order.
     *
     * @param enabledChannels - `channels_enabled` from the config.
     * @param existingSelection - Selection before the reload, if any.
     * @returns A new set with at most two channels.
     */
    function pickInitialPlotChannels(enabledChannels: number[], existingSelection?: Set<number>): Set<number> {
        const selected = new Set<number>();

        if (existingSelection) {
            for (const channel of enabledChannels) {
                if (existingSelection.has(channel)) {
                    selected.add(channel);
                    if (selected.size >= 2) return selected;
                }
            }
        }

        for (const channel of enabledChannels) {
            selected.add(channel);
            if (selected.size >= 2) break;
        }

        return selected;
    }

    /**
     * Returns the selected channels in config order.
     *
     * @returns Channel numbers to render as plot cards. Empty before the config loads.
     */
    function getRenderablePlotChannels(): number[] {
        if (!labjackConfig) return [];
        return labjackConfig.sensor_settings.channels_enabled.filter((channel) =>
            selectedPlotChannels.has(channel)
        );
    }

    /**
     * Returns the unit of a channel's plotted values and whether they are calibrated,
     * from its calibration and its `measurement_units` entry (see
     * `describeChannelUnit`).
     *
     * @param channel - LabJack channel number.
     * @returns The unit description. Raw volts before the config loads.
     */
    function getChannelUnitInfo(channel: number): ChannelUnitInfo {
        const calibration = normalizeCalibration(
            labjackConfig?.sensor_settings.calibrations?.[String(channel)]
        );
        const index = getChannelConfigIndex(channel);
        const measurementUnit = index >= 0 ? labjackConfig?.sensor_settings.measurement_units[index] : undefined;
        return describeChannelUnit(calibration, measurementUnit);
    }

    /**
     * Returns a channel's position in `channels_enabled`, which is also its index into
     * `data_formats` and `measurement_units`.
     *
     * @param channel - LabJack channel number.
     * @returns The index, or -1 when the channel is not enabled or no config is loaded.
     */
    function getChannelConfigIndex(channel: number): number {
        return labjackConfig?.sensor_settings.channels_enabled.indexOf(channel) ?? -1;
    }

    /**
     * Adds or removes a channel from the plot selection and subscribes or unsubscribes
     * its live subject.
     *
     * Adding is ignored when two channels are already selected. The channel's stream
     * state is reset either way, so a channel selected again starts a fresh trace.
     *
     * @param channel - LabJack channel number.
     * @param checked - New checkbox state.
     */
    function togglePlotChannel(channel: number, checked: boolean) {
        const next = new Set(selectedPlotChannels);

        if (checked) {
            if (next.size >= 2) return;
            next.add(channel);
        } else {
            next.delete(channel);
        }

        selectedPlotChannels = next;
        resetChannelStream(channel);
        try {
            syncChannelSubscriptions();
        } catch (err) {
            console.error(`Error updating the subscription of channel ${channel}:`, err);
        }
        markUiSnapshotDirty();
    }

    /**
     * Decodes queued messages of each selected channel, in arrival order, and appends
     * the samples to the channel's buffer.
     *
     * Each sample gets the time `firstSampleUnixNs + i * sampleIntervalNs` from its
     * `Scan`, and the channel's calibration from `sensor_settings.calibrations`. A `NaN`
     * or LJM `-9999` sample is kept as a `NaN` point, drawn as a gap; every other value
     * is plotted whatever its size. Missing time between messages gets a gap marker,
     * repeated or late messages are skipped, and a timeline that jumps backward starts a
     * new buffer (see `drainChannelQueue` in `$lib/plot/stream`). After each message the
     * trigger logic runs on the new points.
     *
     * Work is bounded: channels take turns decoding {@link DECODE_BATCH_MESSAGES}
     * messages each until the queues are empty or `budgetMs` has passed (at least one
     * turn always runs). Messages not decoded stay queued, in order, for the next call;
     * none are dropped or reordered.
     *
     * @param budgetMs - Time allowed for this call, in ms.
     */
    function processPendingVisualizationBatches(budgetMs: number) {
        if (!labjackConfig) return;
        const deadline = performance.now() + budgetMs;
        let backlog = true;
        while (backlog) {
            backlog = decodeOneTurn();
            if (performance.now() >= deadline) break;
        }
    }

    /**
     * Decodes up to {@link DECODE_BATCH_MESSAGES} queued messages of each selected
     * channel.
     *
     * @returns `true` if some channel still has queued messages.
     */
    function decodeOneTurn(): boolean {
        if (!labjackConfig) return false;
        let backlog = false;

        for (const channel of labjackConfig.sensor_settings.channels_enabled) {
            if (!selectedPlotChannels.has(channel) || scanQueue.size(channel) === 0) continue;

            let live = liveChannels.get(channel);
            if (!live) {
                live = createLiveChannel();
                liveChannels.set(channel, live);
            }
            const calibrationSpec = normalizeCalibration(
                labjackConfig.sensor_settings.calibrations?.[String(channel)]
            );

            const liveChannel = live;
            drainChannelQueue(
                scanQueue,
                channel,
                liveChannel,
                (payload) => flatBufferParser.parse(payload),
                calibrationSpec,
                maxDataPoints,
                (chunkStartIndex, reset, chunk) => {
                    handleNewChunk(channel, liveChannel.buffer, chunkStartIndex, reset, chunk);
                },
                DECODE_BATCH_MESSAGES
            );
            markUiSnapshotDirty();
            if (scanQueue.size(channel) > 0) backlog = true;
        }
        return backlog;
    }

    
    /**
     * Loads the config for {@link assetNumber} and starts the live subscriptions.
     *
     * Steps: closes the old connection; checks the asset number; reads `serverName` and `credentialsContent` from
     * sessionStorage and opens a new connection; reads the `key` query parameter from bucket
     * `avenabox` and uses it if its `asset_number` matches; otherwise reads every
     * `*.*.*.config` key in turn and takes the first match. Then resets channel state,
     * pushes a first snapshot and calls {@link startDataSubscription}. Sets `error` on
     * missing login data, connection failure or no match; the promise does not reject.
     *
     * @remarks
     * Also used by the Retry button and when the URL changes. It lists all config keys
     * even when the `key` config matches. Each call first closes the previous connection
     * and its subscriptions. If a newer call starts, or the page is destroyed, while this
     * one is waiting, this one closes the connection it opened and changes nothing.
     */
    async function loadLabJackConfig() {
        const generation = ++loadGeneration;
        const superseded = () => generation !== loadGeneration || destroyed;
        loading = true;
        error = "";
        errorAction = "retry";

        closeLiveConnection();

        if (!Number.isSafeInteger(assetNumber) || assetNumber < 0) {
            labjackConfig = null;
            error = `"${$page.params.asset_number ?? ""}" is not a valid asset number. Open a LabJack's plots from the LabJacks page.`;
            errorAction = "none";
            loading = false;
            return;
        }
        
        try {
            const serverName = sessionStorage.getItem("serverName");
            const credentialsContent = sessionStorage.getItem("credentialsContent");
            
            if (!serverName || !credentialsContent) {
                error = "No NATS connection found. Please login first.";
                errorAction = "login";
                loading = false;
                return;
            }

            connectionState = "connecting";
            const service = await connect(serverName, credentialsContent);
            if (superseded()) {
                closeService(service);
                return;
            }
            if (!service) {
                connectionState = "disconnected";
                error = "Failed to connect to NATS server";
                loading = false;
                return;
            }
            natsService = service;
            watchConnection(service);
            
            const preferredKey = $page.url.searchParams.get('key')?.trim() || "";
            let foundConfig: LabJackConfig | null = null;

            if (preferredKey) {
                try {
                    const configStr = await getKeyValue(service, "avenabox", preferredKey);
                    const config = normalizeLabJackConfig(JSON.parse(configStr));
                    if (config && config.asset_number === assetNumber) {
                        foundConfig = config;
                    }
                } catch (err) {
                    console.error(`Failed to load preferred config key ${preferredKey}:`, err);
                }
            }

            // Search every config only when the ?key= config is missing or is for another asset.
            if (!foundConfig) {
                const keys = await getKeys(service, "avenabox", "*.*.*.config");
                if (superseded()) return;
                for (const key of keys) {
                    try {
                        const configStr = await getKeyValue(service, "avenabox", key);
                        const config = normalizeLabJackConfig(JSON.parse(configStr));
                        if (!config) continue;
                        if (config.asset_number === assetNumber) {
                            foundConfig = config;
                            break;
                        }
                    } catch (err) {
                        console.error(`Failed to parse config for key ${key}:`, err);
                    }
                }
            }
            
            // A newer load has closed this connection; leave its state alone.
            if (superseded()) return;

            if (foundConfig) {
                labjackConfig = foundConfig;
                updateMaxDataPoints();
                initializeChannelData();
                flushUiSnapshots(true);
                await startDataSubscription(generation);
            } else {
                labjackConfig = null;
                error = `LabJack with asset number ${assetNumber} not found`;
            }
        } catch (err) {
            if (superseded()) return;
            console.error("Error loading LabJack config:", err);
            error = "Failed to load LabJack configuration";
        } finally {
            if (!superseded()) {
                loading = false;
                if (connectionState === "connecting") {
                    connectionState = natsService ? "connected" : "disconnected";
                }
            }
        }
    }

    /**
     * Closes a connection without waiting, logging any error.
     *
     * @param service - Connection to close, or `null`.
     */
    function closeService(service: { connection: { close(): Promise<void> } } | null) {
        if (!service) return;
        try {
            service.connection.close().catch((err) => console.error("Error closing NATS connection:", err));
        } catch (err) {
            console.error("Error closing NATS connection:", err);
        }
    }

    /**
     * Follows a connection's status events and its `closed()` promise, updating
     * {@link connectionState}. Events from a connection the page no longer uses (after
     * a reload or when leaving the page) are ignored, so the page's own `close()` is not
     * reported as a lost connection.
     *
     * @param service - Connection just opened by {@link loadLabJackConfig}.
     */
    function watchConnection(service: NatsService) {
        const current = () => natsService === service && !destroyed;
        (async () => {
            try {
                for await (const status of service.connection.status()) {
                    if (!current()) break;
                    connectionState = nextConnectionState(connectionState, status.type);
                }
            } catch (err) {
                console.error("NATS status stream ended with an error:", err);
            }
        })();
        service.connection.closed().then((err) => {
            if (!current()) return;
            connectionState = "disconnected";
            connectionLostReason = err instanceof Error ? err.message : "The server closed the connection.";
        });
    }

    /**
     * Unsubscribes every live subscription and closes this page's connection. Their
     * reader loops end; any message they still hold is ignored (see
     * {@link startDataSubscription}).
     */
    function closeLiveConnection() {
        for (const channel of Array.from(channelSubscriptions.keys())) {
            unsubscribeChannel(channel);
        }
        connectionState = "disconnected";
        connectionLostReason = "";
        const service = natsService;
        natsService = null;
        closeService(service);
    }
    
    /**
     * Resets per-channel state for the loaded config.
     *
     * Every enabled channel gets empty buffers, free-run mode, auto Y with limits -1 to 1,
     * the automatic X window, and a rising trigger at 0 with 40 % pre-trigger and a
     * post-trigger window equal to the automatic X window. Pending messages are dropped.
     * The plot selection is kept where possible (see {@link pickInitialPlotChannels}).
     */
    function initializeChannelData() {
        if (!labjackConfig) return;
        const autoTimeWindow = deriveAutoTimeWindowSec(labjackConfig.sensor_settings.scan_rate_hz);
        
        const newChannelData = new Map<number, DataPoint[]>();
        const newFrozenChannelData = new Map<number, DataPoint[]>();
        const newChannelModes = new Map<number, ChannelPlotMode>();
        const newAxisSettings = new Map<number, AxisSettings>();
        const newTriggerSettings = new Map<number, TriggerSettings>();
        const newChannelTriggered = new Map<number, boolean>();
        const newChannelTriggerTime = new Map<number, number>();
        const newChannelPrebufferReady = new Map<number, boolean>();
        const newChannelTriggerCaptures = new Map<number, TriggerCapture>();
        const newLiveChannels = new Map<number, LiveChannel>();
        const newFrozenBuffers = new Map<number, DataPoint[]>();
        labjackConfig.sensor_settings.channels_enabled.forEach(channel => {
            newChannelData.set(channel, []);
            newFrozenChannelData.set(channel, []);
            newLiveChannels.set(channel, createLiveChannel());
            newFrozenBuffers.set(channel, []);
            newChannelModes.set(channel, 'free_run');
            newAxisSettings.set(channel, {
                autoY: true,
                yMin: -1,
                yMax: 1,
                xWindowSec: autoTimeWindow,
                invertX: false,
                invertY: false
            });
            newTriggerSettings.set(channel, {
                type: 'rising',
                threshold: 0,
                preTriggerPercent: 40,
                postTriggerWindowSec: autoTimeWindow
            });
            newChannelTriggered.set(channel, false);
            newChannelTriggerTime.set(channel, 0);
            newChannelPrebufferReady.set(channel, false);
        });
        
        channelData = newChannelData;
        frozenChannelData = newFrozenChannelData;
        channelModes = newChannelModes;
        axisSettings = newAxisSettings;
        axisInputErrors = new Map();
        triggerSettings = newTriggerSettings;
        channelTriggered = newChannelTriggered;
        channelTriggerTime = newChannelTriggerTime;
        channelPrebufferReady = newChannelPrebufferReady;
        channelTriggerCaptures = newChannelTriggerCaptures;
        liveChannels = newLiveChannels;
        frozenChannelBuffers = newFrozenBuffers;
        scanQueue.clear();
        channelStreamStatus = new Map();
        selectedPlotChannels = pickInitialPlotChannels(
            labjackConfig.sensor_settings.channels_enabled,
            selectedPlotChannels
        );
        uiSnapshotDirty = false;
    }

    /**
     * Subscribes to the live subjects of the selected channels (see
     * {@link syncChannelSubscriptions}). Sets {@link connectionState} to `connected`
     * when the subscriptions exist, or `error` if subscribing throws.
     *
     * @param generation - The {@link loadGeneration} of the load that owns the
     *   subscriptions; reader loops stop once a newer load starts.
     */
    async function startDataSubscription(generation: number) {
        if (!natsService || !labjackConfig) return;
        
        try {
            syncChannelSubscriptions(generation);
            if (connectionState === "connecting") connectionState = "connected";
        } catch (err) {
            console.error("Error starting data subscription:", err);
            error = "Failed to start data subscription";
        }
    }

    /**
     * Subscribes to every selected channel that has no subscription and unsubscribes
     * every channel that is no longer selected, so only the plotted channels are
     * received.
     *
     * The subject comes from `liveLabJackChannelSubject`:
     * `avenars.<site>.<box>.<source>.live.chNN` for structured configs, or
     * `<root>.<asset>.data.chNN` for legacy ones.
     *
     * @param generation - The {@link loadGeneration} the new subscriptions belong to.
     * @throws If subscribing throws (for example on a closed connection).
     */
    function syncChannelSubscriptions(generation: number = loadGeneration) {
        if (!natsService || !labjackConfig) return;
        for (const channel of Array.from(channelSubscriptions.keys())) {
            if (!selectedPlotChannels.has(channel)) unsubscribeChannel(channel);
        }
        for (const channel of labjackConfig.sensor_settings.channels_enabled) {
            if (selectedPlotChannels.has(channel) && !channelSubscriptions.has(channel)) {
                subscribeChannel(channel, generation);
            }
        }
    }

    /**
     * Subscribes to one channel's live subject and starts its reader loop.
     *
     * The loop, not awaited, queues every payload in arrival order; decoding waits for
     * the next frame. It stops when the subscription is unsubscribed, when a reload
     * starts, or when this subscription is no longer the channel's current one (the
     * channel was deselected and selected again), so an old loop never feeds the
     * channel's fresh stream.
     *
     * @param channel - LabJack channel number.
     * @param generation - The {@link loadGeneration} this subscription belongs to.
     */
    function subscribeChannel(channel: number, generation: number) {
        if (!natsService || !labjackConfig) return;
        const subject = liveLabJackChannelSubject(labjackConfig, channel);
        const subscription: Subscription = natsService.connection.subscribe(subject);
        channelSubscriptions.set(channel, subscription);

        (async () => {
            for await (const msg of subscription) {
                if (generation !== loadGeneration || channelSubscriptions.get(channel) !== subscription) break;
                try {
                    scanQueue.push(channel, {
                        payload: msg.data instanceof ArrayBuffer
                            ? msg.data
                            : (msg.data as Uint8Array),
                        receivedAt: Date.now()
                    });
                } catch (err) {
                    console.error(`Error processing message for channel ${channel}:`, err);
                }
            }
        })();
    }

    /**
     * Unsubscribes one channel and forgets its subscription.
     *
     * @param channel - LabJack channel number.
     */
    function unsubscribeChannel(channel: number) {
        const subscription = channelSubscriptions.get(channel);
        channelSubscriptions.delete(channel);
        if (!subscription) return;
        try {
            subscription.unsubscribe();
        } catch (err) {
            console.error("Error unsubscribing:", err);
        }
    }

    /**
     * Empties a channel's buffer, stream state, counters, queued messages and trigger
     * capture, so its next data starts a fresh trace instead of joining data from
     * before it was deselected.
     *
     * @param channel - LabJack channel number.
     */
    function resetChannelStream(channel: number) {
        liveChannels.set(channel, createLiveChannel());
        scanQueue.clearChannel(channel);
        clearTriggerState(channel);
        channelPrebufferReady.set(channel, false);
        channelPrebufferReady = new Map(channelPrebufferReady);
    }
    
    /**
     * Runs the trigger logic after a batch was appended to a channel's live buffer.
     *
     * In a trigger mode it calls {@link processTriggerMode}; in free run it marks the
     * pre-buffer as ready.
     *
     * @param channel - LabJack channel number.
     * @param buffer - The channel's live buffer, already including `chunk`.
     * @param chunkStartIndex - Index in `buffer` of the chunk's first point.
     * @param reset - The buffer was emptied before this chunk (timeline jumped back).
     * @param chunk - Points just appended, oldest first.
     */
    function handleNewChunk(
        channel: number,
        buffer: DataPoint[],
        chunkStartIndex: number,
        reset: boolean,
        chunk: DataPoint[]
    ) {
        const channelMode = channelModes.get(channel) ?? "free_run";
        if (channelMode === "trigger_normal" || channelMode === "trigger_single") {
            processTriggerMode(channel, channelMode, buffer, chunkStartIndex, reset, chunk);
        } else {
            if (!(channelPrebufferReady.get(channel) ?? false)) {
                channelPrebufferReady.set(channel, true);
                channelPrebufferReady = new Map(channelPrebufferReady);
            }
        }
    }

    /**
     * Advances the trigger state of one channel after a new batch.
     *
     * Updates the pre-buffer flag, then lets `advanceTrigger` (`$lib/plot/trigger`) extend
     * the open capture or look for the next crossing. A capture holds exactly the
     * samples in `[trigger - pre, trigger + post]`. In `trigger_normal` the last capture
     * stays on screen until the next trigger replaces it; in `trigger_single` it is held
     * until Re-arm.
     *
     * @param channel - LabJack channel number.
     * @param mode - `trigger_normal` or `trigger_single`.
     * @param buffer - The channel's live buffer, already including the batch.
     * @param chunkStartIndex - Index in `buffer` of the batch's first point.
     * @param reset - The buffer was emptied before this batch.
     * @param chunk - Points just appended.
     */
    function processTriggerMode(
        channel: number,
        mode: ChannelPlotMode,
        buffer: DataPoint[],
        chunkStartIndex: number,
        reset: boolean,
        chunk: DataPoint[]
    ) {
        const settings = triggerSettings.get(channel);
        if (!settings) return;

        const prebufferReady = hasRequiredPreBuffer(buffer, settings);
        if ((channelPrebufferReady.get(channel) ?? false) !== prebufferReady) {
            channelPrebufferReady.set(channel, prebufferReady);
            channelPrebufferReady = new Map(channelPrebufferReady);
        }

        const previous = channelTriggerCaptures.get(channel) ?? null;
        const capture = advanceTrigger(
            previous,
            mode === "trigger_single",
            settings,
            buffer,
            chunkStartIndex,
            reset,
            chunk
        );

        if (capture && capture !== previous) {
            setTriggerCapture(channel, capture);
        }
    }

    /**
     * Stores a channel's capture and the trigger flags derived from it.
     *
     * @param channel - LabJack channel number.
     * @param capture - New or updated capture.
     */
    function setTriggerCapture(channel: number, capture: TriggerCapture) {
        channelTriggerCaptures.set(channel, capture);
        channelTriggerCaptures = new Map(channelTriggerCaptures);
        if (!(channelTriggered.get(channel) ?? false)) {
            channelTriggered.set(channel, true);
            channelTriggered = new Map(channelTriggered);
        }
        if (channelTriggerTime.get(channel) !== capture.triggerTime) {
            channelTriggerTime.set(channel, capture.triggerTime);
            channelTriggerTime = new Map(channelTriggerTime);
        }
        frozenChannelBuffers.set(channel, capture.data);
        markUiSnapshotDirty();
    }

    /**
     * Arms the channel again by clearing its trigger flag, time, capture and frozen plot.
     *
     * @param channel - LabJack channel number.
     */
    function clearTriggerState(channel: number) {
        channelTriggered.set(channel, false);
        channelTriggerTime.set(channel, 0);
        channelTriggerCaptures.delete(channel);
        frozenChannelBuffers.set(channel, []);
        markUiSnapshotDirty();

        channelTriggered = new Map(channelTriggered);
        channelTriggerTime = new Map(channelTriggerTime);
        channelTriggerCaptures = new Map(channelTriggerCaptures);
    }

    /**
     * Tells whether a mode uses the trigger.
     *
     * @param mode - Plot mode.
     * @returns `true` for `trigger_normal` and `trigger_single`.
     */
    function isTriggerMode(mode: ChannelPlotMode): boolean {
        return mode === "trigger_normal" || mode === "trigger_single";
    }

    /**
     * Changes a channel's plot mode.
     *
     * Switching between the two trigger modes keeps the current capture; any other change
     * clears it. The pre-buffer flag is recomputed from the live buffer.
     *
     * @param channel - LabJack channel number.
     * @param nextMode - Mode chosen in the Plot Mode select.
     */
    function setChannelMode(channel: number, nextMode: ChannelPlotMode) {
        const currentMode = channelModes.get(channel) ?? "free_run";
        if (currentMode === nextMode) return;

        if (!isTriggerMode(nextMode) || !isTriggerMode(currentMode)) {
            clearTriggerState(channel);
        }

        if (isTriggerMode(nextMode)) {
            const settings = triggerSettings.get(channel);
            const data = liveChannels.get(channel)?.buffer ?? [];
            const ready = settings ? hasRequiredPreBuffer(data, settings) : false;
            channelPrebufferReady.set(channel, ready);
        } else {
            channelPrebufferReady.set(channel, true);
        }
        channelPrebufferReady = new Map(channelPrebufferReady);

        channelModes.set(channel, nextMode);
        channelModes = new Map(channelModes);
    }

    /**
     * Merges changes into a channel's axis settings and clears the channel's axis input
     * message, which described an earlier rejected value. Numeric limits are checked by
     * {@link commitAxisLimit} before they get here.
     *
     * @param channel - LabJack channel number.
     * @param updates - Fields to change.
     */
    function updateAxisSettings(channel: number, updates: Partial<AxisSettings>) {
        const current = axisSettings.get(channel);
        if (!current) return;
        if (axisInputErrors.has(channel)) {
            const nextErrors = new Map(axisInputErrors);
            nextErrors.delete(channel);
            axisInputErrors = nextErrors;
        }

        axisSettings.set(channel, { ...current, ...updates });
        axisSettings = new Map(axisSettings);
        markUiSnapshotDirty();
    }

    /**
     * Applies a typed Y Min, Y Max or X Window.
     *
     * Any finite number is accepted, including 0, as long as Y Min stays below Y Max and
     * the X window is above 0 (see `applyAxisLimitInput`). Otherwise the input is reset
     * to the value in use and the reason is shown under the inputs.
     *
     * @param channel - LabJack channel number.
     * @param field - Limit being edited.
     * @param input - The input element.
     */
    function commitAxisLimit(channel: number, field: keyof AxisLimits, input: HTMLInputElement) {
        const current = axisSettings.get(channel);
        if (!current) return;
        const result = applyAxisLimitInput(current, field, input.value);
        const nextErrors = new Map(axisInputErrors);
        if (result.ok) {
            nextErrors.delete(channel);
            updateAxisSettings(channel, result.limits);
        } else {
            nextErrors.set(channel, result.error);
            input.value = String(current[field]);
        }
        axisInputErrors = nextErrors;
    }

    /**
     * Applies a typed trigger threshold or post-trigger window. Any finite threshold is
     * accepted, including 0; the post window must be at least 0.01 s. Otherwise the
     * input is reset to the value in use.
     *
     * @param channel - LabJack channel number.
     * @param field - Setting being edited.
     * @param input - The input element.
     */
    function commitTriggerNumber(
        channel: number,
        field: "threshold" | "postTriggerWindowSec",
        input: HTMLInputElement
    ) {
        const setting = triggerSettings.get(channel);
        if (!setting) return;
        const value = parseFiniteInput(input.value);
        if (value === null || (field === "postTriggerWindowSec" && value < 0.01)) {
            input.value = String(setting[field]);
            return;
        }
        triggerSettings.set(channel, { ...setting, [field]: value });
        triggerSettings = new Map(triggerSettings);
    }

    /**
     * Builds the data and trigger props for one channel's `RealTimePlot`.
     *
     * In a trigger mode with a capture it returns mode `"frozen"` with the capture as
     * `frozenData` and its own pre and post windows; `frozenCollecting` is true while the
     * capture is still filling. Otherwise mode `"continuous"`.
     *
     * @param channel - LabJack channel number.
     * @returns `mode`, `data` (live snapshot), `frozenData`, `isTriggered`, `triggerTime`
     *   (Unix ms), `frozenPreWindowSec`, `frozenPostWindowSec` and `frozenCollecting`.
     */
    function getPlotConfig(channel: number) {
        const mode = channelModes.get(channel) ?? "free_run";
        const liveData = channelData.get(channel) || [];
        const capture = channelTriggerCaptures.get(channel);

        if (isTriggerMode(mode) && capture) {
            return {
                mode: "frozen" as const,
                data: liveData,
                frozenData: capture.data,
                isTriggered: true,
                triggerTime: capture.triggerTime,
                frozenPreWindowSec: capture.preWindowSec,
                frozenPostWindowSec: capture.postWindowSec,
                frozenCollecting: !capture.complete
            };
        }

        const { preWindowSec, postWindowSec } = getTriggerWindows(triggerSettings.get(channel));
        return {
            mode: "continuous" as const,
            data: liveData,
            frozenData: undefined,
            isTriggered: false,
            triggerTime: 0,
            frozenPreWindowSec: preWindowSec,
            frozenPostWindowSec: postWindowSec,
            frozenCollecting: false
        };
    }

    /**
     * Text of the connection badges.
     *
     * @param state - Connection state.
     */
    function connectionLabel(state: LiveConnectionState): string {
        switch (state) {
            case "connected": return "Connected";
            case "connecting": return "Connecting...";
            case "reconnecting": return "Reconnecting...";
            default: return "Disconnected";
        }
    }

    /**
     * Color class of the connection dot in the header.
     *
     * @param state - Connection state.
     */
    function connectionDotClass(state: LiveConnectionState): string {
        if (state === "connected") return "bg-success";
        if (state === "disconnected") return "bg-error";
        return "bg-warning";
    }

    /** Default length of the export range, ending now, in ms. */
    const DEFAULT_EXPORT_RANGE_MS = 2 * 60 * 1000;

    /**
     * Formats a date for a `datetime-local` input, in local time, to the second.
     *
     * @param date - Date to format.
     * @returns `YYYY-MM-DDTHH:mm:ss`.
     */
    function toLocalInputValue(date: Date): string {
        const pad = (value: number) => value.toString().padStart(2, "0");
        return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
    }

    /**
     * Describes the browser's time zone, in which the export times are entered.
     *
     * @returns E.g. `Europe/Berlin (UTC+02:00)`.
     */
    function describeLocalTimeZone(): string {
        const name = Intl.DateTimeFormat().resolvedOptions().timeZone || "local time";
        const offsetMin = -new Date().getTimezoneOffset();
        const sign = offsetMin >= 0 ? "+" : "-";
        const abs = Math.abs(offsetMin);
        const pad = (value: number) => value.toString().padStart(2, "0");
        return `${name} (UTC${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)})`;
    }

    /**
     * Shows an export input value as UTC, the time the request is sent in.
     *
     * @param value - `datetime-local` value.
     * @returns E.g. `2026-09-28 12:00:05 UTC`, or `""` when the value is not a date.
     */
    function formatUtcPreview(value: string): string {
        const date = new Date(value);
        if (!value || isNaN(date.getTime())) return "";
        return `${date.toISOString().slice(0, 19).replace("T", " ")} UTC`;
    }

    /**
     * Converts a `datetime-local` value, read as local time, to an RFC 3339 UTC string.
     *
     * @param value - Value such as `2025-01-31T14:05` or `2025-01-31T14:05:30`.
     * @returns The time from `Date.toISOString()`, e.g. `2025-01-31T19:05:00.000Z` in UTC-5.
     * @throws Error if the value is not a valid date.
     */
    function toRfc3339(value: string): string {
        const date = new Date(value);
        if (isNaN(date.getTime())) {
            throw new Error("Invalid date/time value");
        }
        return date.toISOString();
    }

    /**
     * Opens the export form with the plotted channels (all enabled channels if none is
     * plotted) and the last {@link DEFAULT_EXPORT_RANGE_MS} selected.
     */
    function openExportModal() {
        if (!labjackConfig) return;
        const plotted = getRenderablePlotChannels();
        exportChannels = new Set(plotted.length > 0 ? plotted : labjackConfig.sensor_settings.channels_enabled);
        const now = new Date();
        exportEnd = toLocalInputValue(now);
        const start = new Date(now.getTime() - DEFAULT_EXPORT_RANGE_MS);
        exportStart = toLocalInputValue(start);
        exportError = "";
        exportWarning = "";
        exporting = false;
        exportProgress = 0;
        exportTotal = null;
        showExportModal = true;
    }

    /**
     * Closes the export form, cancelling a running download first: the download stops
     * reading frames, releases its subscription, and nothing is saved.
     */
    function closeExportModal() {
        exportAbort?.abort();
        exportAbort = null;
        showExportModal = false;
        exporting = false;
        exportWarning = "";
    }

    /**
     * Adds or removes a channel from the export selection.
     *
     * @param channel - LabJack channel number.
     * @param checked - New checkbox state.
     */
    function toggleExportChannel(channel: number, checked: boolean) {
        const updated = new Set(exportChannels);
        if (checked) {
            updated.add(channel);
        } else {
            updated.delete(channel);
        }
        exportChannels = updated;
    }

    /**
     * Formats a byte count with 1024-based units.
     *
     * @param value - Size in bytes.
     * @returns E.g. `512 B`, `1.5 KB`, `12.0 MB`. Stops at GB.
     */
    function formatBytes(value: number): string {
        const units = ["B", "KB", "MB", "GB"];
        let size = value;
        let unitIndex = 0;
        while (size >= 1024 && unitIndex < units.length - 1) {
            size /= 1024;
            unitIndex += 1;
        }
        return `${size.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
    }

    /**
     * Validates the export form, downloads the CSV over NATS and saves it.
     *
     * Checks that times and at least one channel are set and that start is not after end.
     * Sends an `ExportRequestPayload` (asset, sorted channels, RFC 3339 start and end,
     * `box_id`, and a file name made from `labjack_name`) through `downloadExportViaNats`
     * to `<root>.<site>.<box>.<source>.export.request`. That helper handles the streamed
     * chunks and the acknowledgement of each chunk; this function updates the progress bar
     * and the missing-channel warning from its callbacks. Errors are shown in the form.
     *
     * @param event - Form `submit` event. Its default action is prevented.
     */
    async function handleExportSubmit(event: Event) {
        event.preventDefault();
        if (!labjackConfig) {
            exportError = "Configuration not loaded";
            return;
        }

        if (!exportStart || !exportEnd) {
            exportError = "Please select a start and end time";
            return;
        }

        if (exportChannels.size === 0) {
            exportError = "Select at least one channel";
            return;
        }

        let startIso: string;
        let endIso: string;
        try {
            startIso = toRfc3339(exportStart);
            endIso = toRfc3339(exportEnd);
        } catch (err) {
            exportError = "Invalid date/time selection";
            return;
        }

        if (new Date(startIso) > new Date(endIso)) {
            exportError = "Start time must be before end time";
            return;
        }

        exporting = true;
        exportError = "";
        exportWarning = "";
        exportProgress = 0;
        exportTotal = null;
        const abort = new AbortController();
        exportAbort = abort;

        try {
            const payload: ExportRequestPayload = {
                asset: labjackConfig.asset_number,
                channels: Array.from(exportChannels).sort((a, b) => a - b),
                start: startIso,
                end: endIso,
                box_id: labjackConfig.box_id || undefined,
                download_name: labjackConfig.labjack_name
                    ? `${labjackConfig.labjack_name.replace(/\s+/g, "_")}.csv`
                    : undefined,
            };

            const result = await downloadExportViaNats(natsService, archiveExportRequestSubject(labjackConfig), payload, {
                signal: abort.signal,
                onProgress: (received) => {
                    exportProgress = received;
                },
                onSummary: (missing) => {
                    if (missing.length > 0) {
                        const formatted = missing
                            .map((ch) => ch.toString().padStart(2, "0"))
                            .join(", ");
                        exportWarning = `No samples found for channels: ${formatted}. Continuing with remaining channels.`;
                    } else {
                        exportWarning = "";
                    }
                },
            });

            exportTotal = result.size;
            exportProgress = result.size;

            // Save the Blob through a temporary download link, then release the object URL.
            const url = URL.createObjectURL(result.blob);
            const link = document.createElement("a");
            link.href = url;
            link.download = result.fileName;
            document.body.appendChild(link);
            link.click();
            link.remove();
            URL.revokeObjectURL(url);

            showExportModal = false;
            exportWarning = "";
        } catch (err) {
            // A cancelled export was closed by the user; there is nothing to report.
            if (isExportCancelled(err) || abort.signal.aborted) return;
            console.error("Export failed", err);
            exportError = err instanceof Error ? err.message : "Export failed";
        } finally {
            if (exportAbort === abort) {
                exportAbort = null;
                exporting = false;
            }
        }
    }

    /**
     * One pass of the decode and redraw loop: decodes queued messages within
     * `budgetMs`, then copies the buffers into reactive state if they changed and
     * {@link MIN_SNAPSHOT_INTERVAL_MS} has passed since the last copy.
     *
     * @param budgetMs - Decoding time allowed, in ms.
     */
    function pumpLiveData(budgetMs: number) {
        const now = performance.now();
        lastPumpAt = now;
        processPendingVisualizationBatches(budgetMs);
        if (uiSnapshotDirty && performance.now() - lastFlushAt >= MIN_SNAPSHOT_INTERVAL_MS) {
            flushUiSnapshots();
            lastFlushAt = performance.now();
        }
    }

    /** Runs {@link pumpLiveData} on every animation frame. */
    function onAnimationFrame() {
        pumpLiveData(FRAME_DECODE_BUDGET_MS);
        frameHandle = requestAnimationFrame(onAnimationFrame);
    }

    // Decoding and redraws run on animation frames. Browsers pause animation frames in
    // background tabs, so a slow timer keeps decoding there and the queue does not fill.
    onMount(() => {
        frameHandle = requestAnimationFrame(onAnimationFrame);
        backgroundTimer = setInterval(() => {
            if (performance.now() - lastPumpAt > BACKGROUND_PUMP_INTERVAL_MS / 2) {
                pumpLiveData(BACKGROUND_DECODE_BUDGET_MS);
            }
        }, BACKGROUND_PUMP_INTERVAL_MS);
    });
    
    // Stop the loop, unsubscribe and close this page's connection when leaving the page.
    // A load still in progress sees `destroyed` and closes its own connection.
    onDestroy(() => {
        destroyed = true;
        if (frameHandle) {
            cancelAnimationFrame(frameHandle);
            frameHandle = 0;
        }
        if (backgroundTimer) {
            clearInterval(backgroundTimer);
            backgroundTimer = null;
        }
        exportAbort?.abort();
        closeLiveConnection();
        scanQueue.clear();
    });
</script>

<!--
@component
Live plot page for one LabJack, with a form to download archived data as CSV.

URL: `/labjacks/plots/[asset_number]?key=<kv key>`
- `asset_number`: the config's `asset_number`. If it is not a non-negative integer,
  `+page.ts` answers 404 and SvelteKit shows the shared error page.
- `key` (optional): KV key of the config in bucket `avenabox`, such as
  `<site>.<box>.<source>.config`. It is used only when that config's `asset_number`
  matches. Otherwise the page reads every `*.*.*.config` key and takes the first config
  with a matching `asset_number`. The page writes nothing to KV.

Reads `serverName` and `credentialsContent` from sessionStorage (written by the login
page) and opens its own connection to central NATS. Shows an error if either is missing.

Live data: subscribes only to the channels selected for plotting (at most two), one
subject each, built by `liveLabJackChannelSubject`:
`avenars.<site>.<box>.<source>.live.chNN` for structured configs or
`<root>.<asset>.data.chNN` for legacy ones. Selecting a channel subscribes it and
starts a fresh trace; deselecting unsubscribes it and drops its buffer. Each message
is a FlatBuffer `Scan`. Every message is queued
(bounded; overflow drops the oldest and is shown in Data Statistics). On each
animation frame the page decodes queued messages in order, within a time budget
(what does not fit waits, in order, for the next frame; a 1 s timer takes over in
background tabs), applies the channel's calibration, appends
to a rolling buffer sorted by sample time (NaN and missing time become gaps; duplicates
are skipped), runs the trigger logic and, at most about 30 times a second, copies the buffers into
reactive state. Each channel can run in Free Run, Trigger Normal or Trigger Single.

Hands off to one `RealTimePlot` per selected channel, passing the live snapshot, the
frozen trigger capture, axis settings and trigger state.

Export: builds an `ExportRequestPayload` and calls `downloadExportViaNats` with subject
`<root>.<site>.<box>.<source>.export.request` (from `archiveExportRequestSubject`).
That helper streams the CSV in chunks and acknowledges each one; this page shows the
progress and saves the result through a temporary download link. The form defaults to
the plotted channels and the last two minutes, takes times to the second in the
browser's time zone (shown in the form, with the UTC range sent), and Cancel Download
aborts a running export: no more chunks are read or acknowledged, the reply
subscription is released and nothing is saved.
-->
<svelte:head>
    <title>Real-time Plots - LabJack {assetNumber} - Avena-RS</title>
</svelte:head>

<div class="min-h-screen bg-base-300">
    <!-- Header -->
    <div class="navbar bg-base-100 shadow-xl border-b border-base-200">
        <div class="flex-1">
            <div class="flex items-center">
                <a
                    href="/labjacks"
                    class="btn btn-ghost btn-circle mr-4"
                    title="Back to LabJacks"
                    aria-label="Back to LabJacks"
                >
                    <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
                    </svg>
                </a>
                <div class="avatar placeholder mr-4">
                    <div class="flex items-center justify-center">
                        <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-12 h-12">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>
                        </svg>
                    </div>
                </div>
                <div>
                    <h1 class="text-2xl font-bold text-base-content">Real-time Plots</h1>
                    <p class="text-base-content/70 text-sm">
                        {#if labjackConfig}
                            {labjackConfig.labjack_name} (Asset #{labjackConfig.asset_number})
                        {:else if loading}
                            Loading...
                        {:else}
                            No LabJack loaded
                        {/if}
                    </p>
                </div>
            </div>
        </div>
        <div class="flex-none">
            <!-- Connection status, following the NATS client's status events. -->
            <div class="flex items-center mr-4">
                <div class="w-2 h-2 rounded-full mr-2 {connectionDotClass(connectionState)}"></div>
                <span class="text-base-content text-sm">
                    {connectionLabel(connectionState)}
                </span>
            </div>
        </div>
    </div>

    <!-- Main Content -->
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <!-- Error Message -->
        {#if error}
            <div class="alert alert-error mb-6">
                <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                    <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd"/>
                </svg>
                <span>{error}</span>
                {#if errorAction === "retry"}
                    <div>
                        <button
                            onclick={loadLabJackConfig}
                            class="btn btn-sm btn-error"
                        >
                            Retry
                        </button>
                    </div>
                {:else if errorAction === "login"}
                    <div>
                        <a href="/" class="btn btn-sm btn-error">Log in</a>
                    </div>
                {:else}
                    <div>
                        <a href="/labjacks" class="btn btn-sm btn-error">LabJacks</a>
                    </div>
                {/if}
            </div>
        {/if}

        <!-- Lost connection banner -->
        {#if labjackConfig && !loading && connectionState === "reconnecting"}
            <div class="alert alert-warning mb-6" role="status">
                <span class="loading loading-spinner loading-sm"></span>
                <span>Connection to NATS lost. Reconnecting... The plots resume by themselves; the missing time shows as a gap.</span>
            </div>
        {:else if labjackConfig && !loading && connectionState === "disconnected" && !error}
            <div class="alert alert-error mb-6" role="alert">
                <span>Disconnected from NATS{connectionLostReason ? `: ${connectionLostReason}` : ""}. Live data has stopped.</span>
                <div>
                    <button onclick={loadLabJackConfig} class="btn btn-sm">Reconnect</button>
                </div>
            </div>
        {/if}

        <!-- Loading State -->
        {#if loading}
            <div class="flex justify-center items-center py-12">
                <span class="loading loading-spinner loading-lg text-warning"></span>
                <span class="ml-4 text-lg text-base-content">Loading LabJack configuration...</span>
            </div>
        {:else if labjackConfig}
            <!-- Data Statistics -->
            <div class="card bg-base-100 shadow-xl mb-6">
                <div class="card-body">
                    <h4 class="card-title text-base-content">Data Statistics</h4>
                    <div class="text-sm text-base-content/70 space-y-1">
                        <div class="flex justify-between">
                            <span>Asset Number:</span>
                            <span class="badge badge-primary badge-sm">{assetNumber}</span>
                        </div>
                        <div class="flex justify-between">
                            <span>Scans / Read:</span>
                            <span class="badge badge-info badge-sm">{labjackConfig.sensor_settings.scans_per_read}</span>
                        </div>
                        <div class="flex justify-between">
                            <span>Scan Rate:</span>
                            <span class="badge badge-info badge-sm">{labjackConfig.sensor_settings.scan_rate_hz} Hz</span>
                        </div>
                        <div class="flex justify-between">
                            <span>Auto X Window:</span>
                            <span class="badge badge-info badge-sm">{deriveAutoTimeWindowSec(labjackConfig.sensor_settings.scan_rate_hz).toFixed(3)} s</span>
                        </div>
                        <div class="flex justify-between">
                            <span>Enabled Channels:</span>
                            <span class="badge badge-secondary badge-sm">{labjackConfig.sensor_settings.channels_enabled.join(', ')}</span>
                        </div>
                        <div class="flex justify-between">
                            <span>NATS Subject Pattern:</span>
                            <span class="badge badge-accent badge-sm font-mono h-auto break-all">{liveLabJackChannelPattern(labjackConfig)}</span>
                        </div>
                        <div class="flex justify-between">
                            <span>Channel Data Status:</span>
                            <div class="flex flex-wrap gap-1">
                                <!--
                                    Rate is the average sample rate of the current
                                    snapshot: points divided by its time span.
                                -->
                                {#each Array.from(channelData.entries()) as [ch, data]}
                                    {@const latest = data[data.length - 1]}
                                    {@const rate = data.length > 1 ? Math.round(1000 * (data.length - 1) / (latest?.timestamp - data[0]?.timestamp)) : 0}
                                    {@const status = channelStreamStatus.get(ch)}
                                    <span class="badge badge-outline badge-xs">Ch{ch}: {data.length} pts ({rate} Hz)</span>
                                    <!--
                                        Stream problems since the page loaded: messages
                                        dropped because decoding fell behind, gaps in the
                                        received data, timeline resets, samples skipped as
                                        duplicates, undecodable messages.
                                    -->
                                    {#if status && status.dropped > 0}
                                        <span class="badge badge-error badge-xs">Ch{ch}: {status.dropped} msgs dropped (overload)</span>
                                    {/if}
                                    {#if status && (status.gaps > 0 || status.resets > 0 || status.skipped > 0 || status.decodeErrors > 0)}
                                        <span class="badge badge-warning badge-xs">
                                            Ch{ch}: {status.gaps} gaps{status.resets > 0 ? `, ${status.resets} resets` : ''}{status.skipped > 0 ? `, ${status.skipped} dup samples` : ''}{status.decodeErrors > 0 ? `, ${status.decodeErrors} bad msgs` : ''}
                                        </span>
                                    {/if}
                                {/each}
                            </div>
                        </div>
                        <div class="flex justify-between">
                            <span>Connection Status:</span>
                            <span class="badge {connectionState === 'connected' ? 'badge-success' : connectionState === 'disconnected' ? 'badge-error' : 'badge-warning'} badge-sm">
                                {connectionLabel(connectionState)}
                            </span>
                        </div>
                    </div>
                </div>
            </div>

            <div class="flex justify-end mb-6">
                <button
                    class="btn btn-warning"
                    onclick={openExportModal}
                    disabled={!isConnected || exporting}
                >
                    Download Historical Data
                </button>
            </div>

            <div class="card bg-base-100 shadow-xl mb-6">
                <div class="card-body">
                    <div class="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                        <div>
                            <h4 class="card-title text-base-content">Visible Plot Channels</h4>
                            <p class="text-sm text-base-content/70">
                                Select up to 2 channels. Only selected channels are received, parsed and rendered in the browser; a newly selected channel starts with an empty plot.
                            </p>
                        </div>
                        <span class="badge badge-info badge-sm">
                            {selectedPlotChannels.size} / 2 selected
                        </span>
                    </div>
                    <div class="flex flex-wrap gap-3 mt-2">
                        {#each labjackConfig.sensor_settings.channels_enabled as channel}
                            {@const isSelected = selectedPlotChannels.has(channel)}
                            {@const disableUnchecked = !isSelected && selectedPlotChannels.size >= 2}
                            <label class="label cursor-pointer gap-2 border border-base-300 rounded-lg px-3 py-2">
                                <input
                                    type="checkbox"
                                    class="checkbox checkbox-primary checkbox-sm"
                                    checked={isSelected}
                                    disabled={disableUnchecked}
                                    onchange={(e) => {
                                        if (e.target instanceof HTMLInputElement) {
                                            togglePlotChannel(channel, e.target.checked);
                                        }
                                    }}
                                />
                                <span class="label-text">Channel {channel}</span>
                            </label>
                        {/each}
                    </div>
                </div>
            </div>


            <!-- Channel Sections: Combined Trigger Settings + Plots -->
            <div class="space-y-6">
                {#each getRenderablePlotChannels() as channel}
                    {@const index = getChannelConfigIndex(channel)}
                    {@const channelTriggerSetting = triggerSettings.get(channel)}
                    {@const channelMode = channelModes.get(channel) ?? 'free_run'}
                    {@const channelAxis = axisSettings.get(channel)}
                    {@const plotConfig = getPlotConfig(channel)}
                    {@const isChannelTriggered = channelTriggered.get(channel) || false}
                    {@const channelTriggerTimeValue = channelTriggerTime.get(channel) || 0}
                    {@const isPrebufferReady = channelPrebufferReady.get(channel) ?? false}
                    {@const unitInfo = getChannelUnitInfo(channel)}
                    
                    <!-- Combined Channel Section -->
                    <div class="card bg-base-100 shadow-xl border border-base-200">
                        <div class="card-body">
                            <!-- Channel Header -->
                            <div class="flex flex-col gap-4 md:flex-row md:items-center md:justify-between mb-6">
                                <h3 class="card-title text-base-content">Channel {channel}</h3>
                                <div class="flex flex-wrap items-center gap-3">
                                    <div class="badge badge-outline badge-sm">
                                        {labjackConfig.sensor_settings.data_formats[index]}
                                    </div>
                                    <!-- Whether the plotted values are raw volts or calibrated, and into which unit. -->
                                    <div
                                        class="badge badge-sm {unitInfo.warning ? 'badge-warning' : unitInfo.calibrated ? 'badge-secondary' : 'badge-ghost'}"
                                        title={unitInfo.warning ?? ""}
                                    >
                                        {unitInfo.tag}
                                    </div>
                                    <span class="badge badge-info badge-sm">
                                        {#if channelMode === 'free_run'}
                                            Running
                                        {:else if !isPrebufferReady}
                                            Pre-buffering
                                        {:else if isChannelTriggered}
                                            Triggered
                                        {:else}
                                            Armed
                                        {/if}
                                    </span>
                                    {#if isChannelTriggered}
                                        <span class="text-xs text-success">
                                            Triggered at {new Date(channelTriggerTimeValue).toLocaleTimeString()}
                                        </span>
                                    {/if}
                                </div>
                            </div>

                            {#if unitInfo.warning}
                                <p class="text-sm text-warning -mt-4 mb-4">{unitInfo.warning}</p>
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
                                            value={channelMode}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLSelectElement) {
                                                    setChannelMode(channel, e.target.value as ChannelPlotMode);
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
                                            value={channelAxis?.xWindowSec ?? timeWindow}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    commitAxisLimit(channel, "xWindowSec", e.target);
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
                                                checked={channelAxis?.autoY ?? true}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLInputElement) {
                                                        updateAxisSettings(channel, { autoY: e.target.checked });
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
                                            value={(channelAxis?.autoY ?? true) ? "" : (channelAxis?.yMin ?? -1)}
                                            placeholder={(channelAxis?.autoY ?? true) ? "auto" : undefined}
                                            disabled={channelAxis?.autoY ?? true}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    commitAxisLimit(channel, "yMin", e.target);
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
                                            value={(channelAxis?.autoY ?? true) ? "" : (channelAxis?.yMax ?? 1)}
                                            placeholder={(channelAxis?.autoY ?? true) ? "auto" : undefined}
                                            disabled={channelAxis?.autoY ?? true}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    commitAxisLimit(channel, "yMax", e.target);
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
                                                checked={channelAxis?.invertX ?? false}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLInputElement) {
                                                        updateAxisSettings(channel, { invertX: e.target.checked });
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
                                                checked={channelAxis?.invertY ?? false}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLInputElement) {
                                                        updateAxisSettings(channel, { invertY: e.target.checked });
                                                    }
                                                }}
                                            />
                                        </label>
                                    </div>
                                </div>
                                {#if axisInputErrors.get(channel)}
                                    <p class="text-sm text-error mt-2">{axisInputErrors.get(channel)}</p>
                                {/if}
                            </div>

                            {#if isTriggerMode(channelMode)}
                                <div class="mb-6 p-4 bg-base-200 rounded-lg">
                                    <!--
                                        The threshold is compared with the plotted
                                        (calibrated) values, so it is in the channel's
                                        unit, not always volts.
                                    -->
                                    <h4 class="text-md font-medium text-base-content mb-4">Trigger Settings</h4>
                                    <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-5 gap-4">
                                        <div class="form-control">
                                            <label class="label" for="trigger-type-{channel}">
                                                <span class="label-text">Trigger Edge</span>
                                            </label>
                                            <select
                                                id="trigger-type-{channel}"
                                                value={channelTriggerSetting?.type || 'rising'}
                                                onchange={(e) => {
                                                    const setting = triggerSettings.get(channel);
                                                    if (setting && e.target instanceof HTMLSelectElement) {
                                                        triggerSettings.set(channel, {
                                                            ...setting,
                                                            type: e.target.value as 'rising' | 'falling'
                                                        });
                                                        triggerSettings = new Map(triggerSettings);
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
                                                value={channelTriggerSetting?.threshold ?? 0}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLInputElement) {
                                                        commitTriggerNumber(channel, "threshold", e.target);
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
                                                value={channelTriggerSetting?.preTriggerPercent || 0}
                                                onchange={(e) => {
                                                    const setting = triggerSettings.get(channel);
                                                    if (setting && e.target instanceof HTMLInputElement) {
                                                        const raw = parseInt(e.target.value, 10) || 0;
                                                        triggerSettings.set(channel, {
                                                            ...setting,
                                                            preTriggerPercent: Math.min(95, Math.max(0, raw))
                                                        });
                                                        triggerSettings = new Map(triggerSettings);
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
                                                value={channelTriggerSetting?.postTriggerWindowSec ?? timeWindow}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLInputElement) {
                                                        commitTriggerNumber(channel, "postTriggerWindowSec", e.target);
                                                    }
                                                }}
                                                class="input input-bordered input-warning"
                                            />
                                        </div>
                                        <div class="form-control justify-end">
                                            <button
                                                class="btn btn-outline btn-warning mt-8"
                                                onclick={() => clearTriggerState(channel)}
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
                                    data={plotConfig.data}
                                    unit={unitInfo.unit}
                                    calibrated={unitInfo.calibrated}
                                    timeWindow={channelAxis?.xWindowSec ?? timeWindow}
                                    isTriggered={plotConfig.isTriggered}
                                    triggerTime={plotConfig.triggerTime}
                                    mode={plotConfig.mode}
                                    frozenData={plotConfig.frozenData}
                                    frozenPreWindowSec={plotConfig.frozenPreWindowSec}
                                    frozenPostWindowSec={plotConfig.frozenPostWindowSec}
                                    frozenCollecting={plotConfig.frozenCollecting}
                                    showTriggerThreshold={isTriggerMode(channelMode)}
                                    triggerThreshold={channelTriggerSetting?.threshold}
                                    prebuffering={isTriggerMode(channelMode) && !isChannelTriggered && !isPrebufferReady}
                                    yAutoScale={channelAxis?.autoY ?? true}
                                    yMin={channelAxis?.yMin ?? -1}
                                    yMax={channelAxis?.yMax ?? 1}
                                    invertX={channelAxis?.invertX ?? false}
                                    invertY={channelAxis?.invertY ?? false}
                                />
                            </div>
                        </div>
                    </div>
                {/each}
            </div>

            {#if showExportModal}
                <div class="modal modal-open">
                    <div class="modal-box max-w-2xl">
                        <h3 class="font-bold text-lg text-base-content mb-4">Export Historical Data</h3>
                        <form class="space-y-5" onsubmit={handleExportSubmit}>
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
                                        bind:value={exportStart}
                                        max={exportEnd || undefined}
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
                                        bind:value={exportEnd}
                                        min={exportStart || undefined}
                                        required
                                        disabled={exporting}
                                    />
                                </div>
                            </div>

                            <p class="text-sm text-base-content/70">
                                Times are in your browser's time zone, {describeLocalTimeZone()}.
                                {#if formatUtcPreview(exportStart) && formatUtcPreview(exportEnd)}
                                    Requested range: {formatUtcPreview(exportStart)} to {formatUtcPreview(exportEnd)}.
                                {/if}
                            </p>

                            <div>
                                <h4 class="font-semibold text-base-content mb-2">Channels</h4>
                                <div class="grid grid-cols-2 md:grid-cols-3 gap-2">
                                    {#each labjackConfig.sensor_settings.channels_enabled as ch}
                                        <label class="flex items-center space-x-2 text-sm">
                                            <input
                                                type="checkbox"
                                                class="checkbox checkbox-warning checkbox-sm"
                                                checked={exportChannels.has(ch)}
                                                onchange={(event) => toggleExportChannel(ch, (event.target as HTMLInputElement).checked)}
                                                disabled={exporting}
                                            />
                                            <span>Channel {ch}</span>
                                        </label>
                                    {/each}
                                </div>
                            </div>

                            {#if exportError}
                                <div class="alert alert-error text-sm">
                                    <span>{exportError}</span>
                                </div>
                            {/if}

                            {#if exportWarning}
                                <div class="alert alert-warning text-sm">
                                    <span>{exportWarning}</span>
                                </div>
                            {/if}

                            {#if exporting}
                                <div class="space-y-2">
                                    <progress
                                        class="progress progress-warning w-full"
                                        value={exportProgress}
                                        max={exportTotal ?? Math.max(exportProgress, 1)}
                                    ></progress>
                                    <p class="text-sm text-base-content/70">
                                        Downloaded {formatBytes(exportProgress)}
                                        {#if exportTotal}
                                            / {formatBytes(exportTotal)}
                                        {/if}
                                    </p>
                                </div>
                            {/if}

                            <div class="modal-action">
                                <button
                                    type="button"
                                    class="btn btn-ghost"
                                    onclick={closeExportModal}
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
                            if (!exporting) closeExportModal();
                        }}
                        onkeydown={(event) => {
                            if (!exporting && (event.key === "Escape" || event.key === "Enter" || event.key === " ")) {
                                event.preventDefault();
                                closeExportModal();
                            }
                        }}
                    ></div>
                </div>
            {/if}
        {/if}
    </div>
</div>
