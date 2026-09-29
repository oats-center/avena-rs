<script lang="ts">
    import { onMount, onDestroy, untrack } from "svelte";
    import { page } from "$app/stores";
    import { connect, getKeyValue, getKeys, type NatsService } from "$lib/nats.svelte";
    import { isExportCancelled } from "$lib/exporter";
    import { runExportOnOwnConnection } from "$lib/plot/export-run";
    import {
        isPickerCancelled,
        pickSaveFile,
        runAndSaveExport,
        suggestedExportFileName,
        type SaveFileWritable
    } from "$lib/plot/export-sink";
    import { normalizeCalibration } from "$lib/calibration";
    import { normalizeLabJackConfig, type LabJackConfig } from "$lib/labjack-config";
    import { archiveExportRequestSubject, liveLabJackChannelSubject } from "$lib/subjects";
    import type { Subscription } from "@nats-io/nats-core";
    import ChannelCard from "$lib/components/ChannelCard.svelte";
    import ConnectionBanner from "$lib/components/ConnectionBanner.svelte";
    import ExportDialog from "$lib/components/ExportDialog.svelte";
    import StatsPanel from "$lib/components/StatsPanel.svelte";
    import { parseAssetNumberParam } from "$lib/plot/route";
    import {
        connectionDotClass,
        connectionLabel,
        nextConnectionState,
        type LiveConnectionState
    } from "$lib/plot/connection";
    import {
        deriveAutoTimeWindowSec,
        isTriggerMode,
        pickInitialPlotChannels,
        requiredBufferPoints,
        snapshotKeepMs,
        togglePlotSelection
    } from "$lib/plot/channel";
    import { ChannelView } from "$lib/plot/channel-view.svelte";
    import { buildExportRequest, defaultExportRange, missingChannelsWarning } from "$lib/plot/export-form";
    import { channelUnitInfo } from "$lib/plot/units";
    import { FlatBufferParser } from "$lib/flatbuffer-parser";
    import {
        ScanMessageQueue,
        drainChannelQueue,
        snapshotNewest,
        type DataPoint
    } from "$lib/plot/stream";
    import { advanceTrigger, hasRequiredPreBuffer } from "$lib/plot/trigger";

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
     * Most undecoded messages held per channel between frames. Normally a frame finds at
     * most a few; this is reached only if decoding stalls for a long time (for example a
     * background tab whose timers the browser throttles). Then the oldest are dropped,
     * counted, shown in Data Statistics, and appear as a gap in the plot.
     */
    const MAX_QUEUED_MESSAGES_PER_CHANNEL = 5000;

    /**
     * `asset_number` route parameter. Always a non-negative integer: `+page.ts` answers
     * any other value with a 404 before the page loads.
     */
    let assetNumber = $state<number>(0);
    let labjackConfig = $state<LabJackConfig | null>(null);
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    /**
     * What the error banner offers: `retry` (loading again can succeed) or `login` (no
     * login data in this tab). An invalid asset number never gets here: `+page.ts`
     * answers it with a 404.
     */
    let errorAction = $state<"retry" | "login">("retry");
    /**
     * Connection used for the config and the live subscriptions. Closed before each
     * reload and in `onDestroy`. Exports open a connection of their own (see
     * {@link handleExportSubmit}), so a reload does not cut a running export.
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
     * One state object per enabled channel: settings, trigger state, live buffer and
     * the snapshot the plot draws (see `ChannelView`). Replaced on each config load.
     */
    let channelViews = $state.raw<Map<number, ChannelView>>(new Map());
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
    /** Pending `requestAnimationFrame` of the decode and redraw loop. */
    let frameHandle = 0;
    /** Fallback timer that decodes while no animation frames run. */
    let backgroundTimer: ReturnType<typeof setInterval> | null = null;
    /** `performance.now()` of the last decode pass. */
    let lastPumpAt = 0;
    /** `performance.now()` of the last {@link flushUiSnapshots}. */
    let lastFlushAt = 0;
    /**
     * Automatic X window in seconds, from `deriveAutoTimeWindowSec`; the default for
     * new channels.
     */
    let timeWindow = $state<number>(1); // seconds
    /** Maximum points kept in each live buffer. Set by {@link updateMaxDataPoints}. */
    let maxDataPoints = $state<number>(10000);
    let showExportModal = $state<boolean>(false);
    /**
     * Config the export dialog was opened for. Kept apart from {@link labjackConfig}, so
     * the dialog and a running export survive a reload of the page's config.
     */
    let exportConfig = $state.raw<LabJackConfig | null>(null);
    /** Export start as a `datetime-local` value (`YYYY-MM-DDTHH:mm:ss`, local time). */
    let exportStart = $state<string>("");
    /** Export end as a `datetime-local` value (`YYYY-MM-DDTHH:mm:ss`, local time). */
    let exportEnd = $state<string>("");
    /** Aborts the running export; `null` when none runs. */
    let exportAbort: AbortController | null = null;
    let exportChannels = $state<Set<number>>(new Set());
    /** "Include filtered values" box of the export form. On by default when a channel has filters. */
    let exportIncludeFiltered = $state<boolean>(false);
    let exportError = $state<string>("");
    /** Exporter's missing-channel notice. Cleared when the download finishes. */
    let exportWarning = $state<string>("");
    let exporting = $state<boolean>(false);
    /** Bytes of CSV received so far. */
    let exportProgress = $state<number>(0);
    /** Final size in bytes. Set only after the download completes; `null` while it runs. */
    let exportTotal = $state<number | null>(null);
    /**
     * Every received, not yet decoded message per selected channel, in arrival order.
     * Drained in order by {@link processPendingVisualizationBatches}.
     */
    let scanQueue = new ScanMessageQueue(MAX_QUEUED_MESSAGES_PER_CHANNEL);
    /** Channels chosen for plotting, at most two. Only these are decoded. */
    let selectedPlotChannels = $state<Set<number>>(new Set());
    /** True when buffers changed since the last {@link flushUiSnapshots}. */
    let uiSnapshotDirty = false;

    /** Selected channels in config order: the plot cards. */
    let renderableChannels = $derived(getRenderablePlotChannels());

    /**
     * Reads `asset_number` and `key` from the URL and loads the config. Runs again when
     * the page store changes.
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
        // Bare reads register every channel's settings as dependencies of this effect.
        for (const view of channelViews.values()) {
            view.axis;
            view.trigger;
        }
        updateMaxDataPoints();
    });

    /**
     * Sets {@link maxDataPoints} to the number of samples the longest needed window
     * holds (see `requiredBufferPoints`), then trims the live buffers and marks the
     * snapshot dirty. Does nothing before the config is loaded.
     */
    function updateMaxDataPoints() {
        if (!labjackConfig) return;
        const settings = labjackConfig.sensor_settings;
        maxDataPoints = requiredBufferPoints(
            settings.scan_rate_hz,
            settings.scans_per_read,
            timeWindow,
            channelViews.values()
        );
        trimAllChannelBuffers();
        markUiSnapshotDirty();
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
        for (const view of channelViews.values()) {
            const data = view.live.buffer;
            const excess = data.length - maxDataPoints;
            if (excess > 0) {
                data.splice(0, excess);
            }
        }
    }

    /**
     * Copies the newest part of a channel's live buffer for the plot (see
     * `snapshotKeepMs`). The span is measured back from the newest point's timestamp,
     * not from the clock, and one point before the span is kept so the line reaches the
     * left edge.
     *
     * @param view - The channel.
     * @returns A new array, empty when the buffer is empty.
     */
    function snapshotLiveChannelData(view: ChannelView): DataPoint[] {
        const data = view.live.buffer;
        if (data.length === 0) return [];
        return snapshotNewest(data, snapshotKeepMs(view.axis.xWindowSec, view.mode, view.trigger));
    }

    /**
     * Copies the live buffer and stream counters of every selected channel into its
     * reactive snapshot, which triggers a redraw.
     *
     * @param force - Copy even when nothing is marked dirty.
     */
    function flushUiSnapshots(force: boolean = false) {
        if (!force && !uiSnapshotDirty) return;

        for (const channel of getRenderablePlotChannels()) {
            const view = channelViews.get(channel);
            if (!view) continue;
            view.liveData = snapshotLiveChannelData(view);
            const stats = view.live.stats;
            view.streamStatus = {
                dropped: scanQueue.droppedMessages.get(channel) ?? 0,
                gaps: stats.gaps,
                resets: stats.resets,
                skipped: stats.skippedSamples,
                decodeErrors: stats.decodeErrors
            };
        }
        uiSnapshotDirty = false;
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
        const next = togglePlotSelection(selectedPlotChannels, channel, checked);
        if (!next) return;
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
            const view = channelViews.get(channel);
            if (!view) continue;

            const calibrationSpec = normalizeCalibration(
                labjackConfig.sensor_settings.calibrations?.[String(channel)]
            );

            const liveChannel = view.live;
            drainChannelQueue(
                scanQueue,
                channel,
                liveChannel,
                (payload) => flatBufferParser.parse(payload),
                calibrationSpec,
                maxDataPoints,
                (chunkStartIndex, reset, chunk) => {
                    handleNewChunk(view, liveChannel.buffer, chunkStartIndex, reset, chunk);
                },
                DECODE_BATCH_MESSAGES,
                view.filter
            );
            markUiSnapshotDirty();
            if (scanQueue.size(channel) > 0) backlog = true;
        }
        return backlog;
    }

    /**
     * Loads the config for {@link assetNumber} and starts the live subscriptions.
     *
     * Steps: closes the old connection; reads `serverName` and `credentialsContent` from
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
     * Every enabled channel gets a new `ChannelView`: empty buffers, free-run mode, auto
     * Y with limits -1 to 1, the automatic X window, and a rising trigger at 0 with 40 %
     * pre-trigger and a post-trigger window equal to the automatic X window. Pending
     * messages are dropped. The plot selection is kept where possible (see
     * `pickInitialPlotChannels`).
     */
    function initializeChannelData() {
        if (!labjackConfig) return;
        const autoTimeWindow = deriveAutoTimeWindowSec(labjackConfig.sensor_settings.scan_rate_hz);
        const views = new Map<number, ChannelView>();
        const settings = labjackConfig.sensor_settings;
        for (const channel of settings.channels_enabled) {
            const view = new ChannelView(channel, autoTimeWindow);
            view.configureFilter(
                settings.filters?.[String(channel)],
                normalizeCalibration(settings.calibrations?.[String(channel)]),
                settings.scan_rate_hz
            );
            views.set(channel, view);
        }
        channelViews = views;
        scanQueue.clear();
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
        channelViews.get(channel)?.resetStream();
        scanQueue.clearChannel(channel);
        markUiSnapshotDirty();
    }

    /**
     * Runs the trigger logic after a batch was appended to a channel's live buffer.
     *
     * In a trigger mode it calls {@link processTriggerMode}; in free run it marks the
     * pre-buffer as ready.
     *
     * @param view - The channel.
     * @param buffer - The channel's live buffer, already including `chunk`.
     * @param chunkStartIndex - Index in `buffer` of the chunk's first point.
     * @param reset - The buffer was emptied before this chunk (timeline jumped back).
     * @param chunk - Points just appended, oldest first.
     */
    function handleNewChunk(
        view: ChannelView,
        buffer: DataPoint[],
        chunkStartIndex: number,
        reset: boolean,
        chunk: DataPoint[]
    ) {
        if (isTriggerMode(view.mode)) {
            processTriggerMode(view, buffer, chunkStartIndex, reset, chunk);
        } else if (!view.prebufferReady) {
            view.prebufferReady = true;
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
     * @param view - The channel, in `trigger_normal` or `trigger_single`.
     * @param buffer - The channel's live buffer, already including the batch.
     * @param chunkStartIndex - Index in `buffer` of the batch's first point.
     * @param reset - The buffer was emptied before this batch.
     * @param chunk - Points just appended.
     */
    function processTriggerMode(
        view: ChannelView,
        buffer: DataPoint[],
        chunkStartIndex: number,
        reset: boolean,
        chunk: DataPoint[]
    ) {
        const settings = view.trigger;
        const prebufferReady = hasRequiredPreBuffer(buffer, settings);
        if (view.prebufferReady !== prebufferReady) {
            view.prebufferReady = prebufferReady;
        }

        const previous = view.capture;
        const capture = advanceTrigger(
            previous,
            view.mode === "trigger_single",
            settings,
            buffer,
            chunkStartIndex,
            reset,
            chunk
        );

        if (capture && capture !== previous) {
            view.setCapture(capture);
            markUiSnapshotDirty();
        }
    }

    /**
     * Opens the export form with the plotted channels (all enabled channels if none is
     * plotted) and the last two minutes selected.
     */
    function openExportModal() {
        if (!labjackConfig) return;
        const plotted = getRenderablePlotChannels();
        exportChannels = new Set(plotted.length > 0 ? plotted : labjackConfig.sensor_settings.channels_enabled);
        exportConfig = $state.snapshot(labjackConfig) as LabJackConfig;
        exportIncludeFiltered = Object.keys(exportConfig.sensor_settings.filters ?? {}).length > 0;
        const range = defaultExportRange(new Date());
        exportEnd = range.end;
        exportStart = range.start;
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
     * Validates the export form, downloads the CSV over NATS and saves it.
     *
     * The request comes from `buildExportRequest`. The export runs on a NATS connection
     * of its own, opened with the login data in sessionStorage and closed when the
     * export completes, fails or is cancelled (see `runExportOnOwnConnection`), so a
     * reload, Retry or Reconnect of the page's live connection does not cut it. The
     * request goes to `<root>.<site>.<box>.<source>.export.request`;
     * `downloadExportViaNats` handles the streamed chunks and the acknowledgement of
     * each chunk, and this function updates the progress bar and the missing-channel
     * warning from its callbacks. Errors are shown in the form.
     *
     * Where the browser has a save dialog (`showSaveFilePicker`), it is opened here,
     * synchronously, before the first `await` (browsers allow it only during the
     * click), and every chunk is written to the chosen file as it arrives, before it is
     * acknowledged, through the download's `onChunk` hook, so no other copy is kept (see
     * `runAndSaveExport`). Closing the dialog without a file starts no export. Elsewhere
     * (Firefox, Safari) the finished CSV is saved through a download link. A cancelled
     * or failed export discards what was written.
     *
     * @param event - Form `submit` event. Its default action is prevented.
     */
    async function handleExportSubmit(event: Event) {
        event.preventDefault();
        const config = exportConfig;
        const request = buildExportRequest(
            config,
            exportStart,
            exportEnd,
            exportChannels,
            exportIncludeFiltered ? config?.sensor_settings.filters : undefined
        );
        if (!request.ok) {
            exportError = request.error;
            return;
        }
        if (!config) return;
        const serverName = sessionStorage.getItem("serverName");
        const credentialsContent = sessionStorage.getItem("credentialsContent");
        if (!serverName || !credentialsContent) {
            exportError = "No NATS connection found. Please login first.";
            return;
        }

        // Before any await: the save dialog needs the user's click.
        let picking: ReturnType<typeof pickSaveFile> = null;
        try {
            picking = pickSaveFile(suggestedExportFileName(request.payload));
        } catch (err) {
            console.error("Save dialog unavailable, falling back to a download link:", err);
        }

        exporting = true;
        exportError = "";
        exportWarning = "";
        exportProgress = 0;
        exportTotal = null;
        const abort = new AbortController();
        exportAbort = abort;
        let writable: SaveFileWritable | null = null;

        try {
            if (picking) {
                let handle;
                try {
                    handle = await picking;
                } catch (err) {
                    // The user closed the save dialog: no export.
                    if (isPickerCancelled(err)) return;
                    throw err;
                }
                if (abort.signal.aborted) return;
                writable = await handle.createWritable();
            }

            // Writes each chunk to the file before acknowledging it, or saves the
            // finished Blob; a failed or cancelled export discards a partly written file.

            const result = await runAndSaveExport(writable, (onChunk) =>
                runExportOnOwnConnection({
                    openConnection: () => connect(serverName, credentialsContent),
                    subject: archiveExportRequestSubject(config),
                    payload: request.payload,
                    signal: abort.signal,
                    onProgress: (received) => {
                        exportProgress = received;
                    },
                    onSummary: (missing) => {
                        exportWarning = missingChannelsWarning(missing);
                    },
                    onChunk,
                })
            );

            exportTotal = result.size;
            exportProgress = result.size;
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
background tabs), applies the channel's calibration and, when the config has noise
filters for the channel, runs the causal filters (`$lib/plot/live-filter`; the
channel card switches between filtered and raw values), appends
to a rolling buffer sorted by sample time (NaN and missing time become gaps; duplicates
are skipped), runs the trigger logic and, at most about 30 times a second, copies the buffers into
reactive state. Each channel can run in Free Run, Trigger Normal or Trigger Single.

Hands off to one `RealTimePlot` per selected channel, passing the live snapshot, the
frozen trigger capture, axis settings and trigger state.

Export: builds an `ExportRequestPayload` and calls `downloadExportViaNats` with subject
`<root>.<site>.<box>.<source>.export.request` (from `archiveExportRequestSubject`).
That helper streams the CSV in chunks and acknowledges each one; this page shows the
progress and writes each chunk to the file chosen in the save dialog before it is
acknowledged (its `onChunk` hook), or, without a save dialog, saves the result through
a temporary download link. The form defaults to
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
                {:else}
                    <div>
                        <a href="/" class="btn btn-sm btn-error">Log in</a>
                    </div>
                {/if}
            </div>
        {/if}

        <!-- Lost connection banner -->
        {#if labjackConfig && !loading && (connectionState === "reconnecting" || (connectionState === "disconnected" && !error))}
            <ConnectionBanner state={connectionState} reason={connectionLostReason} onreconnect={loadLabJackConfig} />
        {/if}

        <!-- Loading State -->
        {#if loading}
            <div class="flex justify-center items-center py-12">
                <span class="loading loading-spinner loading-lg text-warning"></span>
                <span class="ml-4 text-lg text-base-content">Loading LabJack configuration...</span>
            </div>
        {:else if labjackConfig}
            <StatsPanel
                config={labjackConfig}
                {assetNumber}
                channels={renderableChannels}
                views={channelViews}
                {connectionState}
            />

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
                {#each renderableChannels as channel (channel)}
                    {@const view = channelViews.get(channel)}
                    {#if view}
                        <ChannelCard
                            {view}
                            dataFormat={labjackConfig.sensor_settings.data_formats[labjackConfig.sensor_settings.channels_enabled.indexOf(channel)]}
                            unitInfo={channelUnitInfo(labjackConfig.sensor_settings, channel)}
                            onchange={markUiSnapshotDirty}
                        />
                    {/if}
                {/each}
            </div>
        {/if}
    </div>

    <!--
        Outside the loading and config blocks, so a reload of the config does not close
        the dialog or hide a running export.
    -->
    {#if showExportModal && exportConfig}
        <ExportDialog
            channels={exportConfig.sensor_settings.channels_enabled}
            selected={exportChannels}
            bind:start={exportStart}
            bind:end={exportEnd}
            error={exportError}
            warning={exportWarning}
            {exporting}
            progress={exportProgress}
            total={exportTotal}
            filteredChannels={exportConfig.sensor_settings.channels_enabled.filter((ch) => exportConfig?.sensor_settings.filters?.[String(ch)])}
            bind:includeFiltered={exportIncludeFiltered}
            ontoggle={toggleExportChannel}
            onsubmit={handleExportSubmit}
            onclose={closeExportModal}
        />
    {/if}
</div>
