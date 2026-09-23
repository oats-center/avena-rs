<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { page } from "$app/stores";
    import { connect, getKeyValue, getKeys } from "$lib/nats.svelte";
    import { downloadExportViaNats, type ExportRequestPayload } from "$lib/exporter";
    import { applyCalibration, normalizeCalibration, type CalibrationSpec } from "$lib/calibration";
    import { archiveExportRequestSubject, liveLabJackChannelPattern, liveLabJackChannelSubject } from "$lib/subjects";
    import RealTimePlot from "$lib/components/RealTimePlot.svelte";
    import {
        FlatBufferParser
    } from "$lib/flatbuffer-parser";

    
    /** `sensor_settings` object of a LabJack config in KV. See the KV config reference. */
    interface SensorSettings {
        scans_per_read: number;
        scan_rate_hz: number;
        channels_enabled: number[];
        gains: number;
        data_formats: string[];
        measurement_units: string[];
        labjack_on_off: boolean;
        calibrations?: Record<string, CalibrationSpec>;
    }
    
    /**
     * LabJack config document stored in KV bucket `avenabox` under
     * `<site>.<box>.<source>.config`.
     */
    interface LabJackConfig {
        labjack_name: string;
        asset_number: number;
        max_channels: number;
        site_id?: string;
        box_id?: string;
        source_type?: string;
        source_id?: string;
        nats_subject: string;
        nats_stream: string;
        rotate_secs: number;
        sensor_settings: SensorSettings;
    }

    /**
     * Fallback values used by {@link normalizeSensorSettings} for missing or invalid
     * fields.
     */
    const DEFAULT_SENSOR_SETTINGS: SensorSettings = {
        scans_per_read: 200,
        scan_rate_hz: 1000,
        channels_enabled: [],
        gains: 1,
        data_formats: [],
        measurement_units: [],
        labjack_on_off: false,
        calibrations: {}
    };

    /**
     * Builds a complete sensor settings object from a raw `sensor_settings` value.
     *
     * Reads the older field names `scan_rate` (for `scans_per_read`) and `sampling_rate`
     * (for `scan_rate_hz`) when the new ones are absent. Missing or non-finite numbers take
     * the values in {@link DEFAULT_SENSOR_SETTINGS}. `data_formats` and `measurement_units`
     * are padded with `"voltage"` and `"V"` to one entry per enabled channel. Arrays and
     * `calibrations` are shallow copies.
     *
     * @param rawSensor - Parsed `sensor_settings` from KV. May be `undefined` or partial.
     * @returns A new settings object with every field set.
     */
    function normalizeSensorSettings(rawSensor: any): SensorSettings {
        const sensor: SensorSettings = {
            scans_per_read: Number(
                rawSensor?.scans_per_read ?? rawSensor?.scan_rate ?? DEFAULT_SENSOR_SETTINGS.scans_per_read
            ),
            scan_rate_hz: Number(
                rawSensor?.scan_rate_hz ?? rawSensor?.sampling_rate ?? DEFAULT_SENSOR_SETTINGS.scan_rate_hz
            ),
            channels_enabled: Array.isArray(rawSensor?.channels_enabled) ? [...rawSensor.channels_enabled] : [],
            gains: Number(rawSensor?.gains ?? DEFAULT_SENSOR_SETTINGS.gains),
            data_formats: Array.isArray(rawSensor?.data_formats) ? [...rawSensor.data_formats] : [],
            measurement_units: Array.isArray(rawSensor?.measurement_units) ? [...rawSensor.measurement_units] : [],
            labjack_on_off: Boolean(rawSensor?.labjack_on_off),
            calibrations:
                rawSensor?.calibrations && typeof rawSensor.calibrations === "object"
                    ? { ...rawSensor.calibrations }
                    : {}
        };

        if (!Number.isFinite(sensor.scans_per_read)) sensor.scans_per_read = DEFAULT_SENSOR_SETTINGS.scans_per_read;
        if (!Number.isFinite(sensor.scan_rate_hz)) sensor.scan_rate_hz = DEFAULT_SENSOR_SETTINGS.scan_rate_hz;
        if (!Number.isFinite(sensor.gains)) sensor.gains = DEFAULT_SENSOR_SETTINGS.gains;
        while (sensor.data_formats.length < sensor.channels_enabled.length) sensor.data_formats.push("voltage");
        while (sensor.measurement_units.length < sensor.channels_enabled.length) sensor.measurement_units.push("V");

        return sensor;
    }

    /**
     * Fills in defaults for a config read from KV.
     *
     * Defaults: `labjack_name` `"unknown"`, `asset_number` 0, `max_channels` 8, empty
     * `site_id` and `box_id`, `source_type` `"labjack"`, `source_id` falls back to
     * `labjack_name`, `nats_subject` `"avenars"`, `nats_stream` `"labjacks"`, `rotate_secs`
     * 60.
     *
     * @param raw - Parsed JSON value of a `*.*.*.config` key.
     * @returns The normalized config, or `null` when `raw` is not an object.
     */
    function normalizeLabJackConfig(raw: any): LabJackConfig | null {
        if (!raw || typeof raw !== "object") return null;
        const sensor = normalizeSensorSettings(raw.sensor_settings ?? {});

        return {
            labjack_name: raw.labjack_name ?? "unknown",
            asset_number: Number(raw.asset_number ?? 0),
            max_channels: Number(raw.max_channels ?? 8),
            site_id: raw.site_id ?? "",
            box_id: raw.box_id ?? "",
            source_type: raw.source_type ?? "labjack",
            source_id: raw.source_id ?? raw.labjack_name ?? "",
            nats_subject: raw.nats_subject ?? "avenars",
            nats_stream: raw.nats_stream ?? "labjacks",
            rotate_secs: Number(raw.rotate_secs ?? 60),
            sensor_settings: sensor
        };
    }
    
    /**
     * One decoded, calibrated sample as stored in the rolling buffers and passed to
     * `RealTimePlot`.
     */
    interface DataPoint {
        /**
         * Sample time in Unix milliseconds, from the `Scan` start time plus the sample
         * interval.
         */
        timestamp: number;
        /** Sample value after the channel's calibration. */
        value: number;
        /**
         * Source sample time in Unix milliseconds. This page sets it equal to
         * `timestamp`.
         */
        sourceTimestamp?: number | null;
        /**
         * Browser time in Unix milliseconds when the NATS message arrived. Used for the
         * lag readout.
         */
        receivedAt?: number;
    }

    /**
     * Plot mode of one channel.
     *
     * `free_run` scrolls continuously. `trigger_normal` freezes a capture on each threshold
     * crossing and re-arms after the post-trigger window. `trigger_single` keeps the first
     * capture until the user presses Re-arm.
     */
    type ChannelPlotMode = 'free_run' | 'trigger_normal' | 'trigger_single';

    /** Trigger settings of one channel, edited in the Trigger Settings panel. */
    interface TriggerSettings {
        /** Edge that fires the trigger. */
        type: 'rising' | 'falling';
        /**
         * Level compared with calibrated sample values, in the channel's calibrated unit.
         */
        threshold: number;
        /** Share of the capture window before the trigger, in percent, 0 to 95. */
        preTriggerPercent: number;
        /** Length of the capture after the trigger, in seconds. */
        postTriggerWindowSec: number;
    }

    /** Axis settings of one channel, edited in the Mode & Axis panel. */
    interface AxisSettings {
        /** When true the plot scales Y to the data and ignores `yMin` and `yMax`. */
        autoY: boolean;
        /** Lower Y limit when `autoY` is false. */
        yMin: number;
        /**
         * Upper Y limit when `autoY` is false. Kept above `yMin` by {@link
         * updateAxisSettings}.
         */
        yMax: number;
        /** Width of the X axis in seconds. At least 0.1. */
        xWindowSec: number;
        /** Mirrors the X axis. */
        invertX: boolean;
        /** Mirrors the Y axis. */
        invertY: boolean;
    }

    /**
     * Frozen capture around one trigger, grown as new samples arrive until it is
     * complete.
     */
    interface TriggerCaptureState {
        /** Samples from `triggerTime - preWindowSec` to `captureEndTime`. */
        data: DataPoint[];
        /** Time of the first sample past the threshold, in Unix milliseconds. */
        triggerTime: number;
        /**
         * End of the capture window, `triggerTime + postWindowSec`, in Unix milliseconds.
         */
        captureEndTime: number;
        /** Timestamp of the newest sample in `data`, in Unix milliseconds. */
        lastCapturedTimestamp: number;
        /** Pre-trigger window in seconds, fixed when the trigger fired. */
        preWindowSec: number;
        /** Post-trigger window in seconds, fixed when the trigger fired. */
        postWindowSec: number;
        /** True once samples up to `captureEndTime` have been captured. */
        complete: boolean;
    }

    /** Newest undecoded live message for one channel, waiting for the next UI tick. */
    interface PendingScanBatch {
        /** Raw FlatBuffer `Scan` bytes from the NATS message. */
        payload: ArrayBuffer | Uint8Array;
        /** Browser time in Unix milliseconds when the message arrived. */
        receivedAt: number;
    }

    /**
     * Samples the automatic X window aims to show; the window is this count divided by
     * the scan rate.
     */
    const TARGET_SAMPLES_PER_WINDOW = 1000;
    /**
     * Period of the timer that decodes pending messages and copies buffers into reactive
     * state, in ms.
     */
    const UI_SNAPSHOT_INTERVAL_MS = 100;
    /**
     * Live snapshots cover this multiple of the channel's X window, so the plot edge is
     * never empty.
     */
    const SNAPSHOT_OVERSCAN_FACTOR = 1.25;
    
    /** `asset_number` route parameter. 0 when missing; a non-number gives `NaN`. */
    let assetNumber = $state<number>(0);
    let labjackConfig = $state<LabJackConfig | null>(null);
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    /**
     * Connection used for live subscriptions and export requests. Closed in `onDestroy`.
     */
    let natsService: any = null;
    /** One live-data subscription per enabled channel. */
    let subscriptions: any[] = [];
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
    /**
     * True after the live subscriptions were created. Does not track later connection
     * loss.
     */
    let isConnected = $state<boolean>(false);
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
    let channelTriggerCaptures = $state<Map<number, TriggerCaptureState>>(new Map());
    /**
     * Browser time updated every tick; used in {@link getPlotConfig} when no capture
     * state exists.
     */
    let uiNow = $state<number>(Date.now());
    let uiNowTimer: ReturnType<typeof setInterval> | null = null;
    /**
     * Automatic X window in seconds, from {@link deriveAutoTimeWindowSec}; the default
     * for new channels.
     */
    let timeWindow = $state<number>(1); // seconds
    /** Maximum points kept in each live buffer. Set by {@link updateMaxDataPoints}. */
    let maxDataPoints = $state<number>(10000);
    let showExportModal = $state<boolean>(false);
    /** Export start as a `datetime-local` value (`YYYY-MM-DDTHH:mm`, local time). */
    let exportStart = $state<string>("");
    /** Export end as a `datetime-local` value (`YYYY-MM-DDTHH:mm`, local time). */
    let exportEnd = $state<string>("");
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
     * Rolling live buffers, one per channel. Not reactive: they are changed in place on
     * every message and copied into {@link channelData} once per tick.
     */
    let channelBuffers = new Map<number, DataPoint[]>();
    /**
     * Frozen trigger captures, one per channel. Not reactive; copied into {@link
     * frozenChannelData}.
     */
    let frozenChannelBuffers = new Map<number, DataPoint[]>();
    /**
     * Newest undecoded message per channel. A message that arrives before the previous one
     * was decoded replaces it, so at most one message per channel is plotted per tick.
     */
    let pendingScanBatches = new Map<number, PendingScanBatch>();
    /** Channels chosen for plotting, at most two. Only these are decoded. */
    let selectedPlotChannels = $state<Set<number>>(new Set());
    /**
     * Channels whose card is on or near the screen. Kept by {@link
     * observeChannelVisibility}; not read elsewhere.
     */
    let visibleChannels = $state<Set<number>>(new Set());
    /** True when buffers changed since the last {@link flushUiSnapshots}. */
    let uiSnapshotDirty = false;
    
    /**
     * Reads `asset_number` and `key` from the URL and loads the config when the asset number
     * is positive. Runs again when the page store changes.
     */
    $effect(() => {
        const nextAssetNumber = parseInt($page.params.asset_number || '0');
        const nextConfigKey = $page.url.searchParams.get('key')?.trim() || "";
        assetNumber = nextAssetNumber;
        if (nextAssetNumber > 0) {
            console.log("Loading plot config", { assetNumber: nextAssetNumber, key: nextConfigKey });
            loadLabJackConfig();
        }
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
     * the scan rate. Then trims the live buffers and marks the snapshot dirty. Does nothing
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

        maxDataPoints = Math.ceil(sr * requiredSeconds);
        trimAllChannelBuffers();
        markUiSnapshotDirty();
        console.log(`Max data points in rolling buffer: ${maxDataPoints}`);
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

    /**
     * Returns the points to plot. Currently returns `data` unchanged.
     *
     * @param data - Snapshot of one channel.
     * @returns The same array.
     */
    function downsampleForDisplay(data: DataPoint[]): DataPoint[] {
        // Preserve all points so the plotted shape matches what was received.
        return data;
    }

    /** Marks the buffers as changed so the next tick copies them into reactive state. */
    function markUiSnapshotDirty() {
        uiSnapshotDirty = true;
    }

    /**
     * Drops the oldest points from each live buffer so none is longer than {@link
     * maxDataPoints}.
     */
    function trimAllChannelBuffers() {
        for (const data of channelBuffers.values()) {
            const excess = data.length - maxDataPoints;
            if (excess > 0) {
                data.splice(0, excess);
            }
        }
    }

    /**
     * Finds the first point at or after a time by binary search.
     *
     * @param data - Points sorted by `timestamp`, oldest first.
     * @param startTime - Time in Unix milliseconds.
     * @returns Index of the first point with `timestamp >= startTime`, or `data.length`.
     */
    function findFirstTimestampIndex(data: DataPoint[], startTime: number): number {
        let low = 0;
        let high = data.length;

        while (low < high) {
            const mid = Math.floor((low + high) / 2);
            if (data[mid].timestamp < startTime) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        return low;
    }

    /**
     * Copies the newest part of a channel's live buffer for the plot.
     *
     * Keeps the X window times {@link SNAPSHOT_OVERSCAN_FACTOR}, at least 0.25 s. In the
     * trigger modes it keeps at least the pre-trigger window plus 0.25 s. The span is
     * measured back from the newest point's timestamp, not from the clock.
     *
     * @param channel - LabJack channel number.
     * @returns A new array, empty when the buffer is empty.
     */
    function snapshotLiveChannelData(channel: number): DataPoint[] {
        const data = channelBuffers.get(channel) || [];
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

        const endTime = data[data.length - 1]?.timestamp ?? 0;
        const startTime = endTime - (keepSeconds * 1000);
        const startIndex = findFirstTimestampIndex(data, startTime);
        return data.slice(startIndex);
    }

    /**
     * Copies the live and frozen buffers of every enabled channel into {@link channelData}
     * and {@link frozenChannelData}, which triggers a redraw.
     *
     * @param force - Copy even when nothing is marked dirty.
     */
    function flushUiSnapshots(force: boolean = false) {
        if (!force && !uiSnapshotDirty) return;

        const liveSnapshots = new Map<number, DataPoint[]>();
        const frozenSnapshots = new Map<number, DataPoint[]>();

        for (const channel of labjackConfig?.sensor_settings.channels_enabled ?? []) {
            liveSnapshots.set(channel, snapshotLiveChannelData(channel));
            frozenSnapshots.set(channel, [...(frozenChannelBuffers.get(channel) || [])]);
        }

        channelData = liveSnapshots;
        frozenChannelData = frozenSnapshots;
        uiSnapshotDirty = false;
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
     * Adds or removes a channel from the plot selection.
     *
     * Adding is ignored when two channels are already selected.
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
        markUiSnapshotDirty();
    }

    /**
     * Decodes the pending message of each selected channel and appends it to the buffer.
     *
     * For each selected channel with a pending message: parses the FlatBuffer `Scan`, gives
     * sample `i` the time `firstSampleUnixNs + i * sampleIntervalNs` (converted to ms),
     * applies the channel's calibration from `sensor_settings.calibrations`, and passes the
     * points to {@link addDataChunk}. Pending messages of unselected channels stay in
     * {@link pendingScanBatches} and are not decoded. Called once per tick.
     */
    function processPendingVisualizationBatches() {
        if (!labjackConfig) return;

        for (const channel of labjackConfig.sensor_settings.channels_enabled) {
            const pending = pendingScanBatches.get(channel);
            if (!pending) continue;
            const shouldProcess = selectedPlotChannels.has(channel);
            if (!shouldProcess) continue;

            pendingScanBatches.delete(channel);

            try {
                const scanData = flatBufferParser.parse(pending.payload);
                if (!scanData) continue;

                const firstSampleMs = Number(scanData.firstSampleUnixNs) / 1_000_000;
                const sampleIntervalMs = Number(scanData.sampleIntervalNs) / 1_000_000;
                const calibrationSpec = normalizeCalibration(
                    labjackConfig.sensor_settings.calibrations?.[String(channel)]
                );

                const newPoints: DataPoint[] = [];
                let timestamp = firstSampleMs;
                for (let i = 0; i < scanData.values.length; i++) {
                    // Drop non-finite values and raw values with magnitude 100 or more,
                    // checked before calibration. The timestamp still advances so later
                    // samples keep their times.
                    const rawValue = scanData.values[i];

                    if (
                        typeof rawValue === "number" &&
                        Number.isFinite(rawValue) &&
                        Number.isFinite(timestamp) &&
                        Math.abs(rawValue) < 100
                    ) {
                        const calibrated = applyCalibration(calibrationSpec, rawValue);
                        newPoints.push({
                            timestamp,
                            value: calibrated,
                            sourceTimestamp: timestamp,
                            receivedAt: pending.receivedAt
                        });
                    }

                    timestamp += sampleIntervalMs;
                }

                if (newPoints.length > 0) {
                    addDataChunk(channel, newPoints);
                }
            } catch (err) {
                console.error(`Error processing pending batch for channel ${channel}:`, err);
            }
        }
    }

    /** {@link channelData} passed through {@link downsampleForDisplay}. */
    let channelDisplayData = $derived(
        new Map(
            Array.from(channelData.entries()).map(([channel, data]) => {
                return [channel, downsampleForDisplay(data)];
            })
        )
    );

    /** {@link frozenChannelData} passed through {@link downsampleForDisplay}. */
    let frozenDisplayData = $derived(
        new Map(
            Array.from(frozenChannelData.entries()).map(([channel, data]) => {
                return [channel, downsampleForDisplay(data)];
            })
        )
    );

    
    /**
     * Loads the config for {@link assetNumber} and starts the live subscriptions.
     *
     * Steps: unsubscribes old subscriptions; reads `serverName` and `credentialsContent` from
     * sessionStorage and opens a new connection; reads the `key` query parameter from bucket
     * `avenabox` and uses it if its `asset_number` matches; otherwise reads every
     * `*.*.*.config` key in turn and takes the first match. Then resets channel state,
     * pushes a first snapshot and calls {@link startDataSubscription}. Sets `error` on
     * missing login data, connection failure or no match; the promise does not reject.
     *
     * @remarks
     * Also used by the Retry button. It lists all config keys even when the `key` config
     * matches. Each call opens a new connection and does not close the previous one.
     */
    async function loadLabJackConfig() {
        loading = true;
        error = "";
        isConnected = false;

        if (subscriptions.length > 0) {
            subscriptions.forEach((sub) => {
                try {
                    sub.unsubscribe();
                } catch (err) {
                    console.error("Error unsubscribing old subscription:", err);
                }
            });
            subscriptions = [];
        }
        
        try {
            const serverName = sessionStorage.getItem("serverName");
            const credentialsContent = sessionStorage.getItem("credentialsContent");
            
            if (!serverName || !credentialsContent) {
                error = "No NATS connection found. Please login first.";
                loading = false;
                return;
            }
            
            natsService = await connect(serverName, credentialsContent);
            if (!natsService) {
                error = "Failed to connect to NATS server";
                loading = false;
                return;
            }
            
            const preferredKey = $page.url.searchParams.get('key')?.trim() || "";
            const keys = await getKeys(natsService, "avenabox", "*.*.*.config");
            let foundConfig: LabJackConfig | null = null;

            if (preferredKey) {
                try {
                    const configStr = await getKeyValue(natsService, "avenabox", preferredKey);
                    const config = normalizeLabJackConfig(JSON.parse(configStr));
                    if (config && config.asset_number === assetNumber) {
                        foundConfig = config;
                    }
                } catch (err) {
                    console.error(`Failed to load preferred config key ${preferredKey}:`, err);
                }
            }

            if (!foundConfig) {
                for (const key of keys) {
                    try {
                        const configStr = await getKeyValue(natsService, "avenabox", key);
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
            
            if (foundConfig) {
                labjackConfig = foundConfig;
                updateMaxDataPoints();
                initializeChannelData();
                flushUiSnapshots(true);
                await startDataSubscription();
            } else {
                error = `LabJack with asset number ${assetNumber} not found`;
            }
        } catch (err) {
            console.error("Error loading LabJack config:", err);
            error = "Failed to load LabJack configuration";
        } finally {
            loading = false;
        }
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
        const newChannelTriggerCaptures = new Map<number, TriggerCaptureState>();
        const newChannelBuffers = new Map<number, DataPoint[]>();
        const newFrozenBuffers = new Map<number, DataPoint[]>();
        labjackConfig.sensor_settings.channels_enabled.forEach(channel => {
            newChannelData.set(channel, []);
            newFrozenChannelData.set(channel, []);
            newChannelBuffers.set(channel, []);
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
        triggerSettings = newTriggerSettings;
        channelTriggered = newChannelTriggered;
        channelTriggerTime = newChannelTriggerTime;
        channelPrebufferReady = newChannelPrebufferReady;
        channelTriggerCaptures = newChannelTriggerCaptures;
        channelBuffers = newChannelBuffers;
        frozenChannelBuffers = newFrozenBuffers;
        pendingScanBatches = new Map<number, PendingScanBatch>();
        selectedPlotChannels = pickInitialPlotChannels(
            labjackConfig.sensor_settings.channels_enabled,
            selectedPlotChannels
        );
        visibleChannels = new Set<number>(labjackConfig.sensor_settings.channels_enabled);
        uiSnapshotDirty = false;
    }

    /**
     * Svelte action that tracks whether a channel card is on or near the screen.
     *
     * Adds the channel to {@link visibleChannels} at once, then updates it from an
     * `IntersectionObserver` with a 300 px margin above and below the viewport. On destroy
     * it disconnects the observer and removes the channel.
     *
     * @param node - The channel card element.
     * @param channel - LabJack channel number.
     * @returns The action object with `destroy`.
     */
    function observeChannelVisibility(node: HTMLElement, channel: number) {
        const initiallyVisible = new Set(visibleChannels);
        initiallyVisible.add(channel);
        visibleChannels = initiallyVisible;

        const observer = new IntersectionObserver(
            (entries) => {
                const visible = entries.some((entry) => entry.isIntersecting);
                const next = new Set(visibleChannels);
                if (visible) {
                    next.add(channel);
                } else {
                    next.delete(channel);
                }
                visibleChannels = next;
            },
            {
                root: null,
                threshold: 0,
                rootMargin: "300px 0px 300px 0px"
            }
        );

        observer.observe(node);

        return {
            destroy() {
                observer.disconnect();
                const next = new Set(visibleChannels);
                next.delete(channel);
                visibleChannels = next;
            }
        };
    }
    
    /**
     * Subscribes to the live subject of every enabled channel.
     *
     * The subject comes from `liveLabJackChannelSubject`:
     * `avenars.<site>.<box>.<source>.live.chNN` for structured configs, or
     * `<root>.<asset>.data.chNN` for legacy ones. All enabled channels are subscribed, not
     * only the selected ones. Sets {@link isConnected} when all subscriptions exist, or
     * `error` if subscribing throws.
     */
    async function startDataSubscription() {
        if (!natsService || !labjackConfig) return;
        
        try {
            for (const channel of labjackConfig.sensor_settings.channels_enabled) {
                const subject = liveLabJackChannelSubject(labjackConfig, channel);
                const subscription = natsService.connection.subscribe(subject);
                subscriptions.push(subscription);
                
                // One reader loop per subscription, not awaited. It ends when the
                // subscription is unsubscribed. It only stores the newest payload;
                // decoding waits for the UI tick.
                (async () => {
                    for await (const msg of subscription) {
                        try {
                            pendingScanBatches.set(channel, {
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
            isConnected = true;
        } catch (err) {
            console.error("Error starting data subscription:", err);
            error = "Failed to start data subscription";
        }
    }
    
    /**
     * Appends decoded points to a channel's live buffer and runs the trigger logic.
     *
     * Trims the buffer to {@link maxDataPoints}. In a trigger mode it calls
     * {@link processTriggerMode}; in free run it marks the pre-buffer as ready.
     *
     * @param channel - LabJack channel number.
     * @param chunk - Points from one `Scan`, oldest first.
     */
    function addDataChunk(channel: number, chunk: DataPoint[]) {
        const currentData = channelBuffers.get(channel) || [];
        for (const point of chunk) {
            currentData.push(point);
        }
        const excess = currentData.length - maxDataPoints;
        if (excess > 0) {
            currentData.splice(0, excess);
        }
        channelBuffers.set(channel, currentData);

        const channelMode = channelModes.get(channel) ?? "free_run";
        if (channelMode === "trigger_normal" || channelMode === "trigger_single") {
            processTriggerMode(channel, channelMode, currentData, chunk);
        } else {
            if (!(channelPrebufferReady.get(channel) ?? false)) {
                channelPrebufferReady.set(channel, true);
                channelPrebufferReady = new Map(channelPrebufferReady);
            }
        }

        markUiSnapshotDirty();
    }

    /**
     * Advances the trigger state of one channel after a new chunk.
     *
     * While not triggered, waits until the buffer spans the pre-trigger window, then looks
     * for a crossing with {@link checkTriggerCondition}. While triggered, extends the
     * capture. In `trigger_single` the capture is held until Re-arm. In `trigger_normal`,
     * once the chunk's last sample passes the post-trigger window, the trigger is cleared
     * (the frozen plot stays) and the same chunk is checked for the next crossing.
     *
     * @param channel - LabJack channel number.
     * @param mode - `trigger_normal` or `trigger_single`.
     * @param fullData - The channel's live buffer, already including `newChunk`.
     * @param newChunk - Points just appended.
     */
    function processTriggerMode(
        channel: number,
        mode: ChannelPlotMode,
        fullData: DataPoint[],
        newChunk: DataPoint[]
    ) {
        const channelTriggerSetting = triggerSettings.get(channel);
        if (!channelTriggerSetting) return;

        const isSingleShot = mode === "trigger_single";
        const lastTimestamp = newChunk[newChunk.length - 1]?.timestamp ?? Date.now();
        const isChannelTriggered = channelTriggered.get(channel) || false;
        const prebufferReady = hasRequiredPreBuffer(fullData, channelTriggerSetting);
        const triggerTime = channelTriggerTime.get(channel) || 0;
        const postWindowMs = Math.max(0, channelTriggerSetting.postTriggerWindowSec || 0) * 1000;
        const postWindowEnd = triggerTime + postWindowMs;

        const previousReady = channelPrebufferReady.get(channel) ?? false;
        if (previousReady !== prebufferReady) {
            channelPrebufferReady.set(channel, prebufferReady);
            channelPrebufferReady = new Map(channelPrebufferReady);
        }

        if (!isChannelTriggered && !prebufferReady) {
            return;
        }

        if (isChannelTriggered) {
            const capture = channelTriggerCaptures.get(channel);
            if (capture && !capture.complete) {
                appendToTriggerCapture(channel, newChunk);
            }

            if (isSingleShot) {
                return;
            }

            if (lastTimestamp < postWindowEnd) {
                return;
            }

            clearTriggerState(channel, false);
        }

        checkTriggerCondition(channel, fullData, newChunk);
    }

    /**
     * Arms the channel again by clearing its trigger flag, time and capture.
     *
     * @param channel - LabJack channel number.
     * @param clearFrozen - Also empty the frozen plot. `trigger_normal` passes `false` so
     *   the last capture stays on screen until the next trigger.
     */
    function clearTriggerState(channel: number, clearFrozen: boolean = true) {
        channelTriggered.set(channel, false);
        channelTriggerTime.set(channel, 0);
        channelTriggerCaptures.delete(channel);

        if (clearFrozen) {
            frozenChannelBuffers.set(channel, []);
            markUiSnapshotDirty();
        }

        channelTriggered = new Map(channelTriggered);
        channelTriggerTime = new Map(channelTriggerTime);
        channelTriggerCaptures = new Map(channelTriggerCaptures);
    }
    
    /**
     * Looks for the first threshold crossing in a new chunk and starts a capture there.
     *
     * Compares each point with the one before it, starting from the last point before the
     * chunk. Rising fires when the previous value is at or below the threshold and the
     * current one is above it; falling is the mirror. Does nothing if there is no point
     * before the chunk.
     *
     * @param channel - LabJack channel number.
     * @param fullData - The channel's live buffer, already including `newChunk`.
     * @param newChunk - Points just appended.
     */
    function checkTriggerCondition(channel: number, fullData: DataPoint[], newChunk: DataPoint[]) {
        const channelTriggerSetting = triggerSettings.get(channel);
        if (!channelTriggerSetting) return;

        // Get the last point before the new chunk was added
        const lastPointBeforeChunk = fullData[fullData.length - newChunk.length - 1];
        if (!lastPointBeforeChunk) return; // Not enough data to compare

        let previousPoint = lastPointBeforeChunk;
        const threshold = channelTriggerSetting.threshold;

        for (const currentPoint of newChunk) {
            let triggered = false;
            if (channelTriggerSetting.type === 'rising' && previousPoint.value <= threshold && currentPoint.value > threshold) {
                triggered = true;
            } else if (channelTriggerSetting.type === 'falling' && previousPoint.value >= threshold && currentPoint.value < threshold) {
                triggered = true;
            }

            if (triggered) {
                channelTriggered.set(channel, true);
                channelTriggerTime.set(channel, currentPoint.timestamp);
                channelTriggered = new Map(channelTriggered);
                channelTriggerTime = new Map(channelTriggerTime);
                
                initializeTriggerCapture(
                    channel,
                    fullData,
                    currentPoint.timestamp,
                    channelTriggerSetting
                );

                return; 
            }

            previousPoint = currentPoint;
        }
    }

    /**
     * Starts a capture with the buffered points around a trigger.
     *
     * Copies points from `triggerTime - preWindowSec` to `triggerTime + postWindowSec`
     * into the capture and the frozen buffer. The capture is complete at once if the buffer
     * already reaches the end of the window.
     *
     * @param channel - LabJack channel number.
     * @param data - The channel's live buffer.
     * @param triggerTime - Time of the crossing sample, in Unix milliseconds.
     * @param settings - The channel's trigger settings.
     */
    function initializeTriggerCapture(
        channel: number,
        data: DataPoint[],
        triggerTime: number,
        settings: TriggerSettings
    ) {
        const { preWindowSec, postWindowSec } = getTriggerWindows(settings);
        const startTime = triggerTime - (preWindowSec * 1000);
        const endTime = triggerTime + (postWindowSec * 1000);

        const frozenData = data.filter(point =>
            point.timestamp >= startTime && point.timestamp <= endTime
        );
        const lastCapturedTimestamp =
            frozenData.length > 0 ? frozenData[frozenData.length - 1].timestamp : triggerTime;
        const capture: TriggerCaptureState = {
            data: frozenData,
            triggerTime,
            captureEndTime: endTime,
            lastCapturedTimestamp,
            preWindowSec,
            postWindowSec,
            complete: lastCapturedTimestamp >= endTime
        };

        channelTriggerCaptures.set(channel, capture);
        channelTriggerCaptures = new Map(channelTriggerCaptures);
        frozenChannelBuffers.set(channel, frozenData);
        markUiSnapshotDirty();
    }

    /**
     * Adds new points to an open capture and marks it complete at the end of the window.
     *
     * Only points newer than the last captured one and not past `captureEndTime` are added.
     *
     * @param channel - LabJack channel number.
     * @param chunk - Points just appended to the live buffer.
     */
    function appendToTriggerCapture(channel: number, chunk: DataPoint[]) {
        const capture = channelTriggerCaptures.get(channel);
        if (!capture || capture.complete) return;

        const appended = chunk.filter(
            (point) =>
                point.timestamp > capture.lastCapturedTimestamp &&
                point.timestamp <= capture.captureEndTime
        );

        if (appended.length === 0) {
            const latestChunkTimestamp = chunk[chunk.length - 1]?.timestamp ?? capture.lastCapturedTimestamp;
            if (latestChunkTimestamp >= capture.captureEndTime) {
                channelTriggerCaptures.set(channel, { ...capture, complete: true });
                channelTriggerCaptures = new Map(channelTriggerCaptures);
            }
            return;
        }

        const data = capture.data.concat(appended);
        const lastCapturedTimestamp = data[data.length - 1]?.timestamp ?? capture.lastCapturedTimestamp;
        const updatedCapture: TriggerCaptureState = {
            ...capture,
            data,
            lastCapturedTimestamp,
            complete: lastCapturedTimestamp >= capture.captureEndTime
        };

        channelTriggerCaptures.set(channel, updatedCapture);
        channelTriggerCaptures = new Map(channelTriggerCaptures);
        frozenChannelBuffers.set(channel, data);
        markUiSnapshotDirty();
    }
    
    
    /**
     * Handles the Re-arm button: clears the trigger and the frozen plot.
     *
     * @param channel - LabJack channel number.
     */
    function resetChannelTrigger(channel: number) {
        clearTriggerState(channel, true);
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
     * clears it. The pre-buffer flag is recomputed from the last snapshot in
     * {@link channelData}, not from the live buffer.
     *
     * @param channel - LabJack channel number.
     * @param nextMode - Mode chosen in the Plot Mode select.
     */
    function setChannelMode(channel: number, nextMode: ChannelPlotMode) {
        const currentMode = channelModes.get(channel) ?? "free_run";
        if (currentMode === nextMode) return;

        if (!isTriggerMode(nextMode) || !isTriggerMode(currentMode)) {
            clearTriggerState(channel, true);
        }

        if (isTriggerMode(nextMode)) {
            const settings = triggerSettings.get(channel);
            const data = channelData.get(channel) || [];
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
     * Merges changes into a channel's axis settings.
     *
     * A missing or non-positive X window is replaced by {@link timeWindow}, and the window
     * is raised to at least 0.1 s. If `yMax` is not above `yMin` it is set to `yMin + 0.001`.
     *
     * @param channel - LabJack channel number.
     * @param updates - Fields to change.
     */
    function updateAxisSettings(channel: number, updates: Partial<AxisSettings>) {
        const current = axisSettings.get(channel);
        if (!current) return;

        const updated = { ...current, ...updates };
        if (!Number.isFinite(updated.xWindowSec) || updated.xWindowSec <= 0) {
            updated.xWindowSec = timeWindow;
        }
        updated.xWindowSec = Math.max(0.1, updated.xWindowSec);
        if (updated.yMax <= updated.yMin) {
            updated.yMax = updated.yMin + 0.001;
        }

        axisSettings.set(channel, updated);
        axisSettings = new Map(axisSettings);
        markUiSnapshotDirty();
    }

    /**
     * Converts trigger settings into pre and post window lengths.
     *
     * The post window is at least 0.01 s. The pre-trigger percent is clamped to 0 to 95 and
     * taken as a share of the whole window, so `pre = post * p / (1 - p)`.
     *
     * @param settings - The channel's trigger settings, or `undefined` for the minimums.
     * @returns `preWindowSec` and `postWindowSec`, in seconds.
     *
     * @example
     * ```ts
     * getTriggerWindows({ type: "rising", threshold: 0, preTriggerPercent: 40, postTriggerWindowSec: 1 });
     * // { preWindowSec: 0.667, postWindowSec: 1 } (approximately)
     * ```
     */
    function getTriggerWindows(settings: TriggerSettings | undefined) {
        const postWindowSec = Math.max(0.01, settings?.postTriggerWindowSec || 0.01);
        const preFraction = Math.min(0.95, Math.max(0, (settings?.preTriggerPercent || 0) / 100));
        const preWindowSec = postWindowSec * (preFraction / (1 - preFraction));
        return { preWindowSec, postWindowSec };
    }

    /**
     * Tells whether a buffer spans the pre-trigger window.
     *
     * @param data - Points sorted by time, oldest first.
     * @param settings - The channel's trigger settings.
     * @returns `true` when there are at least two points and the newest is at least
     *   `preWindowSec` after the oldest.
     */
    function hasRequiredPreBuffer(data: DataPoint[], settings: TriggerSettings): boolean {
        if (data.length < 2) return false;
        const { preWindowSec } = getTriggerWindows(settings);
        const requiredMs = preWindowSec * 1000;
        const oldest = data[0]?.timestamp;
        const latest = data[data.length - 1]?.timestamp;
        if (!Number.isFinite(oldest) || !Number.isFinite(latest)) return false;
        return (latest - oldest) >= requiredMs;
    }

    /**
     * Builds the data and trigger props for one channel's `RealTimePlot`.
     *
     * In a trigger mode with a held trigger it returns mode `"frozen"` with the capture as
     * `frozenData`; otherwise mode `"continuous"`. `frozenCollecting` is true while the
     * capture is still filling. If no capture state exists it falls back to comparing
     * {@link uiNow} with the end of the post-trigger window.
     *
     * @param channel - LabJack channel number.
     * @returns `mode`, `data` (live snapshot), `frozenData`, `isTriggered`, `triggerTime`
     *   (Unix ms), `frozenPreWindowSec`, `frozenPostWindowSec` and `frozenCollecting`.
     */
    function getPlotConfig(channel: number) {
        const mode = channelModes.get(channel) ?? "free_run";
        const liveData = channelDisplayData.get(channel) || [];
        const capture = channelTriggerCaptures.get(channel);
        const frozenData = capture?.data ?? (frozenDisplayData.get(channel) || []);
        const isTriggered = channelTriggered.get(channel) || false;
        const triggerTime = channelTriggerTime.get(channel) || 0;
        const triggerConfig = triggerSettings.get(channel);
        const { preWindowSec, postWindowSec } = capture ?? getTriggerWindows(triggerConfig);
        const isFrozenCollecting =
            isTriggered &&
            triggerTime > 0 &&
            !(capture?.complete ?? (uiNow >= (triggerTime + postWindowSec * 1000)));

        if (isTriggerMode(mode) && isTriggered) {
            return {
                mode: "frozen" as const,
                data: liveData,
                frozenData,
                isTriggered: true,
                triggerTime,
                frozenPreWindowSec: preWindowSec,
                frozenPostWindowSec: postWindowSec,
                frozenCollecting: isFrozenCollecting
            };
        }

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
    
    /** Does a full page load of `/labjacks`. */
    function goBack() {
        window.location.href = "/labjacks";
    }

    /**
     * Formats a date for a `datetime-local` input, in local time.
     *
     * @param date - Date to format.
     * @returns `YYYY-MM-DDTHH:mm`.
     */
    function toLocalInputValue(date: Date): string {
        const pad = (value: number) => value.toString().padStart(2, "0");
        return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
    }

    /**
     * Converts a `datetime-local` value, read as local time, to an RFC 3339 UTC string.
     *
     * @param value - Value such as `2025-01-31T14:05`.
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
     * Opens the export form with all enabled channels and the last five minutes selected.
     */
    function openExportModal() {
        if (!labjackConfig) return;
        const defaults = new Set(labjackConfig.sensor_settings.channels_enabled);
        exportChannels = defaults;
        const now = new Date();
        exportEnd = toLocalInputValue(now);
        const start = new Date(now.getTime() - 5 * 60 * 1000);
        exportStart = toLocalInputValue(start);
        exportError = "";
        exportWarning = "";
        exporting = false;
        exportProgress = 0;
        exportTotal = null;
        showExportModal = true;
    }

    /**
     * Hides the export form.
     *
     * It does not cancel a running download. The Cancel button is disabled during a
     * download, but the backdrop still calls this, and the file is still saved when the
     * download finishes.
     */
    function closeExportModal() {
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
            console.error("Export failed", err);
            exportError = err instanceof Error ? err.message : "Export failed";
        } finally {
            exporting = false;
        }
    }

    // One timer drives both decoding and redraws, so the plots update at most every
    // UI_SNAPSHOT_INTERVAL_MS no matter how fast messages arrive.
    onMount(() => {
        uiNowTimer = setInterval(() => {
            uiNow = Date.now();
            processPendingVisualizationBatches();
            flushUiSnapshots();
        }, UI_SNAPSHOT_INTERVAL_MS);
    });
    
    // Stop the timer, unsubscribe and close this page's connection when leaving the page.
    onDestroy(() => {
        if (uiNowTimer) {
            clearInterval(uiNowTimer);
            uiNowTimer = null;
        }

        // Clean up subscriptions
        subscriptions.forEach(sub => {
            try {
                sub.unsubscribe();
            } catch (err) {
                console.error("Error unsubscribing:", err);
            }
        });
        
        if (natsService) {
            try {
                natsService.connection.close();
            } catch (err) {
                console.error("Error closing NATS connection:", err);
            }
        }
    });
</script>

<!--
@component
Live plot page for one LabJack, with a form to download archived data as CSV.

URL: `/labjacks/plots/[asset_number]?key=<kv key>`
- `asset_number`: the config's `asset_number`. If it is not a positive number the page
  loads nothing and keeps showing the loading spinner.
- `key` (optional): KV key of the config in bucket `avenabox`, such as
  `<site>.<box>.<source>.config`. It is used only when that config's `asset_number`
  matches. Otherwise the page reads every `*.*.*.config` key and takes the first config
  with a matching `asset_number`. The page writes nothing to KV.

Reads `serverName` and `credentialsContent` from sessionStorage (written by the login
page) and opens its own connection to central NATS. Shows an error if either is missing.

Live data: subscribes to one subject per enabled channel, built by
`liveLabJackChannelSubject`: `avenars.<site>.<box>.<source>.live.chNN` for structured
configs or `<root>.<asset>.data.chNN` for legacy ones. Each message is a FlatBuffer
`Scan`. Only the newest undecoded message per channel is kept. Every 100 ms a timer
decodes it for the channels selected for plotting (at most two), applies the channel's
calibration, appends to a rolling buffer, runs the trigger logic and copies the buffers
into reactive state. Each channel can run in Free Run, Trigger Normal or Trigger Single.

Hands off to one `RealTimePlot` per selected channel, passing the live snapshot, the
frozen trigger capture, axis settings and trigger state.

Export: builds an `ExportRequestPayload` and calls `downloadExportViaNats` with subject
`<root>.<site>.<box>.<source>.export.request` (from `archiveExportRequestSubject`).
That helper streams the CSV in chunks and acknowledges each one; this page shows the
progress and saves the result through a temporary download link.
-->
<svelte:head>
    <title>Real-time Plots - LabJack {assetNumber} - Avena-OTR</title>
</svelte:head>

<div class="min-h-screen bg-base-300">
    <!-- Header -->
    <div class="navbar bg-base-100 shadow-xl border-b border-base-200">
        <div class="flex-1">
            <div class="flex items-center">
                <button
                    onclick={goBack}
                    class="btn btn-ghost btn-circle mr-4"
                    title="Back to LabJacks"
                    aria-label="Back to LabJacks"
                >
                    <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
                    </svg>
                </button>
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
                        {:else}
                            Loading...
                        {/if}
                    </p>
                </div>
            </div>
        </div>
        <div class="flex-none">
            <!--
                Connection Status. "Connected" means the live subscriptions were created;
                it does not follow later connection loss.
            -->
            <div class="flex items-center mr-4">
                <div class="w-2 h-2 rounded-full mr-2 {isConnected ? 'bg-success' : 'bg-error'}"></div>
                <span class="text-base-content text-sm">
                    {isConnected ? 'Connected' : 'Disconnected'}
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
                <div>
                    <button
                        onclick={loadLabJackConfig}
                        class="btn btn-sm btn-error"
                    >
                        Retry
                    </button>
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
                            <span class="badge badge-accent badge-sm font-mono">{liveLabJackChannelPattern(labjackConfig)}</span>
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
                                    {@const rate = data.length > 1 ? Math.round(1000 / ((latest?.timestamp - data[0]?.timestamp) / data.length)) : 0}
                                    <span class="badge badge-outline badge-xs">Ch{ch}: {data.length} pts ({rate} Hz)</span>
                                {/each}
                            </div>
                        </div>
                        <div class="flex justify-between">
                            <span>Connection Status:</span>
                            <span class="badge {isConnected ? 'badge-success' : 'badge-error'} badge-sm">
                                <div class="w-2 h-2 rounded-full mr-1 {isConnected ? 'bg-success-content' : 'bg-error-content'}"></div>
                                {isConnected ? 'Connected' : 'Disconnected'}
                            </span>
                        </div>
                        <div class="flex justify-between">
                            <span>Data Parser:</span>
                            <span class="badge badge-info badge-sm">Source-time plot + receive-time lag</span>
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
                                Select up to 2 channels. Only selected channels are parsed and rendered in the browser.
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
                    
                    <!-- Combined Channel Section -->
                    <div
                        class="card bg-base-100 shadow-xl border border-base-200"
                        use:observeChannelVisibility={channel}
                    >
                        <div class="card-body">
                            <!-- Channel Header -->
                            <div class="flex flex-col gap-4 md:flex-row md:items-center md:justify-between mb-6">
                                <h3 class="card-title text-base-content">Channel {channel}</h3>
                                <div class="flex flex-wrap items-center gap-3">
                                    <div class="badge badge-outline badge-sm">
                                        {labjackConfig.sensor_settings.data_formats[index]} 
                                        ({labjackConfig.sensor_settings.measurement_units[index]})
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
                                            min="0.1"
                                            step="0.1"
                                            class="input input-bordered"
                                            value={channelAxis?.xWindowSec ?? timeWindow}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    updateAxisSettings(channel, { xWindowSec: parseFloat(e.target.value) || timeWindow });
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
                                            <span class="label-text">Y Min</span>
                                        </label>
                                        <input
                                            id="y-min-{channel}"
                                            type="number"
                                            step="0.01"
                                            class="input input-bordered"
                                            value={channelAxis?.yMin ?? -1}
                                            disabled={channelAxis?.autoY ?? true}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    updateAxisSettings(channel, { yMin: parseFloat(e.target.value) || -1 });
                                                }
                                            }}
                                        />
                                    </div>

                                    <div class="form-control">
                                        <label class="label" for="y-max-{channel}">
                                            <span class="label-text">Y Max</span>
                                        </label>
                                        <input
                                            id="y-max-{channel}"
                                            type="number"
                                            step="0.01"
                                            class="input input-bordered"
                                            value={channelAxis?.yMax ?? 1}
                                            disabled={channelAxis?.autoY ?? true}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    updateAxisSettings(channel, { yMax: parseFloat(e.target.value) || 1 });
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
                            </div>

                            {#if isTriggerMode(channelMode)}
                                <div class="mb-6 p-4 bg-base-200 rounded-lg">
                                    <!--
                                        The threshold is compared with calibrated values,
                                        so its unit is the channel's unit, not always
                                        volts.
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
                                                <span class="label-text">Threshold (V)</span>
                                            </label>
                                            <input
                                                id="trigger-threshold-{channel}"
                                                type="number"
                                                step="0.01"
                                                value={channelTriggerSetting?.threshold || 0}
                                                onchange={(e) => {
                                                    const setting = triggerSettings.get(channel);
                                                    if (setting && e.target instanceof HTMLInputElement) {
                                                        triggerSettings.set(channel, {
                                                            ...setting,
                                                            threshold: parseFloat(e.target.value) || 0
                                                        });
                                                        triggerSettings = new Map(triggerSettings);
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
                                                value={channelTriggerSetting?.postTriggerWindowSec || timeWindow}
                                                onchange={(e) => {
                                                    const setting = triggerSettings.get(channel);
                                                    if (setting && e.target instanceof HTMLInputElement) {
                                                        const raw = parseFloat(e.target.value) || 0.01;
                                                        triggerSettings.set(channel, {
                                                            ...setting,
                                                            postTriggerWindowSec: Math.max(0.01, raw)
                                                        });
                                                        triggerSettings = new Map(triggerSettings);
                                                    }
                                                }}
                                                class="input input-bordered input-warning"
                                            />
                                        </div>
                                        <div class="form-control justify-end">
                                            <button
                                                class="btn btn-outline btn-warning mt-8"
                                                onclick={() => resetChannelTrigger(channel)}
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
                                    unit={labjackConfig.sensor_settings.measurement_units[index]}
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
                                        bind:value={exportEnd}
                                        min={exportStart || undefined}
                                        required
                                        disabled={exporting}
                                    />
                                </div>
                            </div>

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
                                    disabled={exporting}
                                >
                                    Cancel
                                </button>
                                <button
                                    type="submit"
                                    class="btn btn-warning"
                                    disabled={exporting}
                                >
                                    {exporting ? "Preparing..." : "Start Download"}
                                </button>
                            </div>
                        </form>
                    </div>
                    <div
                        class="modal-backdrop bg-black/40"
                        role="button"
                        tabindex="0"
                        onclick={closeExportModal}
                        onkeydown={(event) => {
                            if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
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
