<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { page } from "$app/stores";
    import { connect, getKeyValue, getKeys, putKeyValue } from "$lib/nats.svelte";
    import { downloadExportViaWebSocket, type ExportRequestPayload } from "$lib/exporter";
    import {
        fetchVideoCameras,
        requestVideoClip,
        requestVideoObject,
        type VideoClipRequestPayload,
        type VideoCameraCoverage
    } from "$lib/video";
    import { applyCalibration, normalizeCalibration, type CalibrationSpec } from "$lib/calibration";
    import RealTimePlot from "$lib/components/RealTimePlot.svelte";
    import { FlatBufferParser, calculateSampleTimestamps } from "$lib/flatbuffer-parser";

    
    interface SensorSettings {
        scan_rate: number;
        sampling_rate: number;
        channels_enabled: number[];
        gains: number;
        data_formats: string[];
        measurement_units: string[];
        labjack_on_off: boolean;
        calibrations?: Record<string, CalibrationSpec>;
        trigger_settings?: Record<string, BackendTriggerSetting>;
    }

    interface BackendTriggerSetting {
        enabled: boolean;
        type: "rising" | "falling";
        threshold: number;
        holdoff_ms: number;
    }
    
    interface LabJackConfig {
        labjack_name: string;
        asset_number: number;
        max_channels: number;
        nats_subject: string;
        nats_stream: string;
        rotate_secs: number;
        sensor_settings: SensorSettings;
    }

    const DEFAULT_SENSOR_SETTINGS: SensorSettings = {
        scan_rate: 200,
        sampling_rate: 1000,
        channels_enabled: [],
        gains: 1,
        data_formats: [],
        measurement_units: [],
        labjack_on_off: false,
        calibrations: {},
        trigger_settings: {}
    };

    function normalizeLabJackConfig(raw: any): LabJackConfig | null {
        if (!raw || typeof raw !== "object") return null;
        const sensor = { ...DEFAULT_SENSOR_SETTINGS, ...(raw.sensor_settings ?? {}) } as SensorSettings;
        if (!Array.isArray(sensor.channels_enabled)) sensor.channels_enabled = [];
        if (!Array.isArray(sensor.data_formats)) sensor.data_formats = [];
        if (!Array.isArray(sensor.measurement_units)) sensor.measurement_units = [];
        if (!Number.isFinite(sensor.scan_rate)) sensor.scan_rate = DEFAULT_SENSOR_SETTINGS.scan_rate;
        if (!Number.isFinite(sensor.sampling_rate)) sensor.sampling_rate = DEFAULT_SENSOR_SETTINGS.sampling_rate;
        if (!Number.isFinite(sensor.gains)) sensor.gains = DEFAULT_SENSOR_SETTINGS.gains;
        if (!sensor.calibrations || typeof sensor.calibrations !== "object") sensor.calibrations = {};
        if (!sensor.trigger_settings || typeof sensor.trigger_settings !== "object") sensor.trigger_settings = {};
        while (sensor.data_formats.length < sensor.channels_enabled.length) sensor.data_formats.push("voltage");
        while (sensor.measurement_units.length < sensor.channels_enabled.length) sensor.measurement_units.push("V");

        return {
            labjack_name: raw.labjack_name ?? "unknown",
            asset_number: Number(raw.asset_number ?? 0),
            max_channels: Number(raw.max_channels ?? 8),
            nats_subject: raw.nats_subject ?? "avenabox",
            nats_stream: raw.nats_stream ?? "labjacks",
            rotate_secs: Number(raw.rotate_secs ?? 60),
            sensor_settings: sensor
        };
    }
    
    interface DataPoint {
        timestamp: number;
        value: number;
    }

    type ChannelPlotMode = 'free_run' | 'trigger_normal' | 'trigger_single';

    interface TriggerSettings {
        type: 'rising' | 'falling';
        threshold: number;
        holdoffMs: number;
        preTriggerPercent: number;
        postTriggerWindowSec: number;
    }

    interface BackendTriggerEvent {
        asset: number;
        channel: number;
        trigger_time: string;
        trigger_time_unix_ms: number;
        raw_value: number;
        calibrated_value: number;
        threshold: number;
        trigger_type: "rising" | "falling";
        holdoff_ms: number;
        calibration_id: string;
        record_id?: string;
        record_status?: TriggerRecordStatus;
        record_clip_keys?: string[];
        record_last_error?: string;
        record_updated_at?: string;
    }

    type TriggerRecordStatus = "pending" | "processed";

    interface TriggerRecordEntry {
        id?: string;
        event?: unknown;
        status?: string;
        clip_keys?: unknown;
        last_error?: unknown;
        updated_at?: unknown;
    }

    interface ChannelVideoRoute {
        enabled: boolean;
        cameraId: string;
    }

    interface TriggerFetchReadiness {
        ready: boolean;
        message: string;
    }

    interface AxisSettings {
        autoY: boolean;
        yMin: number;
        yMax: number;
        xWindowSec: number;
        invertX: boolean;
        invertY: boolean;
    }

    interface FrozenCaptureState {
        triggerTime: number;
        startTime: number;
        endTime: number;
        lastCapturedTimestamp: number;
    }

    const DEFAULT_MANUAL_Y_MIN = -10;
    const DEFAULT_MANUAL_Y_MAX = 10;
    
    let assetNumber = $state<number>(0);
    let labjackConfig = $state<LabJackConfig | null>(null);
    let labjackConfigKey = $state<string>("");
    let labjackConfigRaw = $state<any>(null);
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    let natsService: any = null;
    let subscriptions: any[] = [];
    let channelData = $state<Map<number, DataPoint[]>>(new Map());
    let frozenChannelData = $state<Map<number, DataPoint[]>>(new Map());
    let channelModes = $state<Map<number, ChannelPlotMode>>(new Map());
    let axisSettings = $state<Map<number, AxisSettings>>(new Map());
    const channelLastTimestamp = new Map<number, number>();
    const channelFrozenCapture = new Map<number, FrozenCaptureState>();
    let isConnected = $state<boolean>(false);
    let flatBufferParser = new FlatBufferParser();
    let triggerSettings = $state<Map<number, TriggerSettings>>(new Map());
    let channelTriggered = $state<Map<number, boolean>>(new Map());
    let channelTriggerTime = $state<Map<number, number>>(new Map());
    let channelRearmTime = $state<Map<number, number>>(new Map());
    let channelPrebufferReady = $state<Map<number, boolean>>(new Map());
    let uiNow = $state<number>(Date.now());
    let uiNowTimer: ReturnType<typeof setInterval> | null = null;
    let timeWindow = $state<number>(5); // seconds
    let maxDataPoints = $state<number>(10000);
    let showExportModal = $state<boolean>(false);
    let exportStart = $state<string>("");
    let exportEnd = $state<string>("");
    let exportChannels = $state<Set<number>>(new Set());
    let exportError = $state<string>("");
    let exportWarning = $state<string>("");
    let exporting = $state<boolean>(false);
    let exportProgress = $state<number>(0);
    let exportTotal = $state<number | null>(null);
    let availableCameraIds = $state<string[]>([]);
    let cameraCoverageById = $state<Map<string, VideoCameraCoverage>>(new Map());
    let cameraListLoading = $state<boolean>(false);
    let cameraListError = $state<string>("");
    let videoLoading = $state<boolean>(false);
    let videoError = $state<string>("");
    let videoUrl = $state<string>("");
    let videoFileName = $state<string>("");
    let backendTriggerSaving = $state<boolean>(false);
    let backendTriggerError = $state<string>("");
    let backendTriggerStatus = $state<string>("");
    let backendTriggerEvents = $state<BackendTriggerEvent[]>([]);
    const MAX_BACKEND_TRIGGER_EVENTS = 10;
    const seenBackendTriggerKeys = new Set<string>();
    let triggerRecordRefreshTimer: ReturnType<typeof setInterval> | null = null;
    let manualVideoChannel = $state<number>(0);
    let channelVideoRoutes = $state<Map<number, ChannelVideoRoute>>(new Map());
    let triggerSettingsDirty = $state<boolean>(false);
    let triggerAutoSaveTimer: ReturnType<typeof setTimeout> | null = null;
    let triggerRangeStart = $state<string>("");
    let triggerRangeEnd = $state<string>("");
    let triggerRangeLoading = $state<boolean>(false);
    let triggerRangeError = $state<string>("");
    let triggerRangeStatus = $state<string>("");
    let triggerRangeResults = $state<BackendTriggerEvent[]>([]);
    let triggerRangeFetchLoading = $state<boolean>(false);
    
    // Get asset number from URL params
    $effect(() => {
        assetNumber = parseInt($page.params.asset_number || '0');
        if (assetNumber > 0) {
            loadLabJackConfig();
        }
    });

    $effect(() => {
        if (labjackConfig) updateMaxDataPoints();
    });

    $effect(() => {
        if (!labjackConfig) return;
        axisSettings;
        triggerSettings;
        updateMaxDataPoints();
    });

    function updateMaxDataPoints() {
        if (!labjackConfig) return;
        const sr = labjackConfig.sensor_settings.sampling_rate;
        let requiredSeconds = timeWindow * 2;

        for (const axis of axisSettings.values()) {
            requiredSeconds = Math.max(requiredSeconds, Math.max(0.1, axis.xWindowSec) * 2);
        }

        for (const trigger of triggerSettings.values()) {
            const windows = getTriggerWindows(trigger);
            requiredSeconds = Math.max(requiredSeconds, windows.preWindowSec + windows.postWindowSec);
        }

        maxDataPoints = Math.ceil(sr * requiredSeconds);
        console.log(`Max data points in rolling buffer: ${maxDataPoints}`);
    }

    function downsampleForDisplay(data: DataPoint[], targetPoints: number = 2000): DataPoint[] {
        if (data.length <= targetPoints) return data;
        if (targetPoints < 4) return [data[0], data[data.length - 1]];

        const bucketCount = Math.max(2, Math.floor(targetPoints / 2));
        const bucketSize = Math.ceil(data.length / bucketCount);
        const sampled: DataPoint[] = [];

        for (let start = 0; start < data.length; start += bucketSize) {
            const end = Math.min(data.length, start + bucketSize);
            let minPoint: DataPoint | null = null;
            let maxPoint: DataPoint | null = null;

            for (let i = start; i < end; i++) {
                const point = data[i];
                if (!minPoint || point.value < minPoint.value) minPoint = point;
                if (!maxPoint || point.value > maxPoint.value) maxPoint = point;
            }

            if (!minPoint || !maxPoint) continue;
            if (minPoint.timestamp <= maxPoint.timestamp) {
                sampled.push(minPoint);
                if (maxPoint !== minPoint) sampled.push(maxPoint);
            } else {
                sampled.push(maxPoint);
                if (maxPoint !== minPoint) sampled.push(minPoint);
            }
        }

        return sampled.length > 1 ? sampled : data;
    }

    let channelDisplayData = $derived(
        new Map(
            Array.from(channelData.entries()).map(([channel, data]) => {
                return [channel, downsampleForDisplay(data)];
            })
        )
    );

    let frozenDisplayData = $derived(
        new Map(
            Array.from(frozenChannelData.entries()).map(([channel, data]) => {
                return [channel, downsampleForDisplay(data)];
            })
        )
    );

    
    async function loadLabJackConfig() {
        loading = true;
        error = "";
        backendTriggerError = "";
        backendTriggerStatus = "";
        backendTriggerEvents = [];
        seenBackendTriggerKeys.clear();
        availableCameraIds = [];
        cameraCoverageById = new Map();
        cameraListError = "";
        
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
            
            // Find the LabJack config by asset number
            const keys = await getKeys(natsService, "avenabox", "labjackd.config.*");
            let foundConfig: LabJackConfig | null = null;
            let foundConfigKey = "";
            let foundConfigRaw: any = null;
            
            for (const key of keys) {
                try {
                    const configStr = await getKeyValue(natsService, "avenabox", key);
                    const parsed = JSON.parse(configStr);
                    const config = normalizeLabJackConfig(parsed);
                    if (!config) continue;
                    if (config.asset_number === assetNumber) {
                        foundConfig = config;
                        foundConfigKey = key;
                        foundConfigRaw = parsed;
                        break;
                    }
                } catch (err) {
                    console.error(`Failed to parse config for key ${key}:`, err);
                }
            }
            
            if (foundConfig) {
                labjackConfig = foundConfig;
                labjackConfigKey = foundConfigKey;
                labjackConfigRaw = foundConfigRaw;
                updateMaxDataPoints();
                initializeChannelData();
                await startDataSubscription();
                await refreshAvailableCameras();
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
    
    function initializeChannelData() {
        if (!labjackConfig) return;
        
        const newChannelData = new Map<number, DataPoint[]>();
        const newFrozenChannelData = new Map<number, DataPoint[]>();
        const newChannelModes = new Map<number, ChannelPlotMode>();
        const newAxisSettings = new Map<number, AxisSettings>();
        const newTriggerSettings = new Map<number, TriggerSettings>();
        const newChannelTriggered = new Map<number, boolean>();
        const newChannelTriggerTime = new Map<number, number>();
        const newChannelRearmTime = new Map<number, number>();
        const newChannelPrebufferReady = new Map<number, boolean>();
        const nextVideoRoutes = new Map<number, ChannelVideoRoute>(channelVideoRoutes);
        channelLastTimestamp.clear();
        channelFrozenCapture.clear();
        
        labjackConfig.sensor_settings.channels_enabled.forEach(channel => {
            const backendTriggerSetting = labjackConfig?.sensor_settings.trigger_settings?.[String(channel)];
            newChannelData.set(channel, []);
            newFrozenChannelData.set(channel, []);
            newChannelModes.set(channel, 'free_run');
            newAxisSettings.set(channel, {
                autoY: true,
                yMin: DEFAULT_MANUAL_Y_MIN,
                yMax: DEFAULT_MANUAL_Y_MAX,
                xWindowSec: timeWindow,
                invertX: false,
                invertY: false
            });
            channelLastTimestamp.set(channel, Number.NaN);
            newTriggerSettings.set(channel, {
                type: backendTriggerSetting?.type === "falling" ? "falling" : "rising",
                threshold: Number.isFinite(backendTriggerSetting?.threshold) ? backendTriggerSetting!.threshold : 0,
                holdoffMs: Number.isFinite(backendTriggerSetting?.holdoff_ms) ? Math.max(0, backendTriggerSetting!.holdoff_ms) : 500,
                preTriggerPercent: 40,
                postTriggerWindowSec: timeWindow
            });
            newChannelTriggered.set(channel, false);
            newChannelTriggerTime.set(channel, 0);
            newChannelRearmTime.set(channel, 0);
            newChannelPrebufferReady.set(channel, false);

            const existingRoute = nextVideoRoutes.get(channel);
            nextVideoRoutes.set(channel, {
                enabled: existingRoute?.enabled ?? true,
                cameraId: existingRoute?.cameraId ?? ""
            });
        });
        
        channelData = newChannelData;
        frozenChannelData = newFrozenChannelData;
        channelModes = newChannelModes;
        axisSettings = newAxisSettings;
        triggerSettings = newTriggerSettings;
        channelTriggered = newChannelTriggered;
        channelTriggerTime = newChannelTriggerTime;
        channelRearmTime = newChannelRearmTime;
        channelPrebufferReady = newChannelPrebufferReady;
        channelVideoRoutes = nextVideoRoutes;
        manualVideoChannel = labjackConfig.sensor_settings.channels_enabled[0] ?? 0;
    }
    
    async function startDataSubscription() {
        if (!natsService || !labjackConfig) return;
        
        try {
            for (const channel of labjackConfig.sensor_settings.channels_enabled) {
                const subject = `${labjackConfig.nats_subject}.${labjackConfig.asset_number}.data.ch${channel.toString().padStart(2, '0')}`;
                const subscription = natsService.connection.subscribe(subject);
                subscriptions.push(subscription);
                
                (async () => {
                    for await (const msg of subscription) {
                        try {
                            let arrayBuffer: ArrayBuffer;
                            if (msg.data instanceof ArrayBuffer) {
                                arrayBuffer = msg.data;
                            } else {
                                const uint8Array = new Uint8Array(msg.data);
                                arrayBuffer = uint8Array.buffer;
                            }
                            
                            const scanData = flatBufferParser.parse(arrayBuffer);
                            if (!scanData) {
                                console.warn(`Failed to parse FlatBuffer for channel ${channel}`);
                                continue;
                            }
                            
                            const previousLastTimestamp = channelLastTimestamp.get(channel);
                            const sampleTimestamps = calculateSampleTimestamps(
                                scanData.timestamp, 
                                scanData.values, 
                                labjackConfig.sensor_settings.sampling_rate,
                                previousLastTimestamp
                            );
                            const calibrationSpec = normalizeCalibration(
                                labjackConfig?.sensor_settings.calibrations?.[String(channel)]
                            );
                            
                            // **CHANGE: Create a chunk of new data points**
                            const newPoints: DataPoint[] = [];
                            for (let i = 0; i < scanData.values.length; i++) {
                                const rawValue = scanData.values[i];
                                const timestamp = sampleTimestamps[i];
                                
                                if (typeof rawValue === 'number' && typeof timestamp === 'number' && !isNaN(rawValue) && isFinite(rawValue) && Math.abs(rawValue) < 100) {
                                    const calibrated = applyCalibration(calibrationSpec, rawValue);
                                    newPoints.push({ timestamp, value: calibrated });
                                }
                            }

                            // **CHANGE: Process the entire chunk at once**
                            if (newPoints.length > 0) {
                                channelLastTimestamp.set(channel, newPoints[newPoints.length - 1].timestamp);
                                addDataChunk(channel, newPoints);
                            }
                            
                        } catch (err) {
                            console.error(`Error processing message for channel ${channel}:`, err);
                        }
                    }
                })();
            }
            await startTriggerEventSubscription();
            isConnected = true;
        } catch (err) {
            console.error("Error starting data subscription:", err);
            error = "Failed to start data subscription";
        }
    }

    function decodeMessageText(data: any): string {
        if (typeof data === "string") return data;
        if (data instanceof Uint8Array) {
            return new TextDecoder().decode(data);
        }
        if (data instanceof ArrayBuffer) {
            return new TextDecoder().decode(new Uint8Array(data));
        }
        return "";
    }

    function normalizeTriggerRecordStatus(value: unknown): TriggerRecordStatus | undefined {
        if (value === "pending" || value === "processed") {
            return value;
        }
        return undefined;
    }

    function normalizeTriggerRecordClipKeys(value: unknown): string[] {
        if (!Array.isArray(value)) return [];
        return value.filter((entry): entry is string => typeof entry === "string" && entry.length > 0);
    }

    function parseBackendTriggerEventObject(parsed: any): BackendTriggerEvent | null {
        if (!parsed || typeof parsed !== "object") return null;
        if (!Number.isInteger(parsed.asset) || !Number.isInteger(parsed.channel)) return null;
        if (typeof parsed.trigger_time !== "string") return null;
        const triggerTimeMs = Number(parsed.trigger_time_unix_ms);
        const resolvedTriggerTimeMs =
            Number.isFinite(triggerTimeMs) && triggerTimeMs > 0
                ? Math.trunc(triggerTimeMs)
                : Math.trunc(parseIsoMs(parsed.trigger_time));
        return {
            asset: parsed.asset,
            channel: parsed.channel,
            trigger_time: parsed.trigger_time,
            trigger_time_unix_ms: Number.isFinite(resolvedTriggerTimeMs) ? resolvedTriggerTimeMs : 0,
            raw_value: Number(parsed.raw_value ?? 0),
            calibrated_value: Number(parsed.calibrated_value ?? 0),
            threshold: Number(parsed.threshold ?? 0),
            trigger_type: parsed.trigger_type === "falling" ? "falling" : "rising",
            holdoff_ms: Math.max(0, Number(parsed.holdoff_ms ?? 0)),
            calibration_id:
                typeof parsed.calibration_id === "string" ? parsed.calibration_id : "identity"
        };
    }

    function parseTriggerRecordEntryObject(parsed: unknown): BackendTriggerEvent | null {
        if (!parsed || typeof parsed !== "object") return null;
        const record = parsed as TriggerRecordEntry;
        const eventPayload =
            record.event && typeof record.event === "object" ? record.event : parsed;
        const event = parseBackendTriggerEventObject(eventPayload);
        if (!event) return null;

        event.record_id = typeof record.id === "string" ? record.id : backendTriggerRecordId(event);
        event.record_status = normalizeTriggerRecordStatus(record.status);
        event.record_clip_keys = normalizeTriggerRecordClipKeys(record.clip_keys);
        if (typeof record.last_error === "string" && record.last_error.trim().length > 0) {
            event.record_last_error = record.last_error.trim();
        }
        if (typeof record.updated_at === "string" && record.updated_at.trim().length > 0) {
            event.record_updated_at = record.updated_at.trim();
        }
        return event;
    }

    function parseBackendTriggerEvent(data: any): BackendTriggerEvent | null {
        try {
            return parseBackendTriggerEventObject(JSON.parse(decodeMessageText(data)));
        } catch {
            return null;
        }
    }

    function backendTriggerEventKey(event: BackendTriggerEvent): string {
        return `${event.asset}:${event.channel}:${event.trigger_time_unix_ms}:${event.trigger_type}`;
    }

    function backendTriggerRecordId(event: BackendTriggerEvent): string {
        const triggerKind = event.trigger_type === "falling" ? "f" : "r";
        const triggerMs = Number.isFinite(event.trigger_time_unix_ms) && event.trigger_time_unix_ms > 0
            ? Math.trunc(event.trigger_time_unix_ms)
            : Math.trunc(parseIsoMs(event.trigger_time));
        return `${event.asset}_${event.channel}_${Number.isFinite(triggerMs) ? triggerMs : 0}_${triggerKind}`;
    }

    function backendTriggerRecordKey(event: BackendTriggerEvent): string {
        return `event.${backendTriggerRecordId(event)}`;
    }

    function prependBackendTriggerEvent(event: BackendTriggerEvent) {
        const key = backendTriggerEventKey(event);
        const deduped = backendTriggerEvents.filter((existing) => backendTriggerEventKey(existing) !== key);
        backendTriggerEvents = [event, ...deduped].slice(0, MAX_BACKEND_TRIGGER_EVENTS);
    }

    function mergeBackendTriggerEventFromRecord(event: BackendTriggerEvent) {
        const key = backendTriggerEventKey(event);
        let found = false;
        const merged = backendTriggerEvents.map((existing) => {
            if (backendTriggerEventKey(existing) !== key) return existing;
            found = true;
            return { ...existing, ...event };
        });
        backendTriggerEvents = found
            ? merged
            : [event, ...merged].slice(0, MAX_BACKEND_TRIGGER_EVENTS);
    }

    async function refreshTriggerRecordForEvent(event: BackendTriggerEvent) {
        if (!natsService) return;
        try {
            const raw = await getKeyValue(natsService, "video_trigger_events", backendTriggerRecordKey(event));
            if (!raw || raw === "Key value does not exist") return;
            const parsed = parseTriggerRecordEntryObject(JSON.parse(raw));
            if (!parsed) return;
            if (backendTriggerEventKey(parsed) !== backendTriggerEventKey(event)) return;
            mergeBackendTriggerEventFromRecord(parsed);
        } catch {
            // Ignore transient KV/read errors for background status refresh.
        }
    }

    async function refreshRecentTriggerRecordStates() {
        if (!natsService || backendTriggerEvents.length === 0) return;
        const candidates = backendTriggerEvents
            .filter((event) => !event.record_clip_keys || event.record_clip_keys.length === 0)
            .slice(0, MAX_BACKEND_TRIGGER_EVENTS);
        for (const event of candidates) {
            await refreshTriggerRecordForEvent(event);
        }
    }

    function rememberSeenTriggerKey(key: string) {
        seenBackendTriggerKeys.add(key);
        if (seenBackendTriggerKeys.size > 200) {
            const first = seenBackendTriggerKeys.values().next().value;
            if (first) seenBackendTriggerKeys.delete(first);
        }
    }

    async function startTriggerEventSubscription() {
        if (!natsService || !labjackConfig) return;

        const subject = `${labjackConfig.nats_subject}.${labjackConfig.asset_number}.trigger.*`;
        const subscription = natsService.connection.subscribe(subject);
        subscriptions.push(subscription);

        (async () => {
            for await (const msg of subscription) {
                const event = parseBackendTriggerEvent(msg.data);
                if (!event) continue;
                const key = backendTriggerEventKey(event);
                if (seenBackendTriggerKeys.has(key)) continue;
                rememberSeenTriggerKey(key);
                prependBackendTriggerEvent(event);
                void refreshTriggerRecordForEvent(event);
            }
        })();
    }

    async function refreshAvailableCameras() {
        if (!labjackConfig) return;
        cameraListLoading = true;
        cameraListError = "";
        try {
            const result = await fetchVideoCameras(labjackConfig.asset_number);
            const cameras = result.cameras;
            availableCameraIds = cameras;
            cameraCoverageById = new Map(
                result.coverage.map((entry) => [entry.camera_id, entry] as const)
            );
            const preferred =
                [...cameraCoverageById.values()]
                    .sort((a, b) => {
                        const aMs = parseIsoMs(a.recommended_center_max);
                        const bMs = parseIsoMs(b.recommended_center_max);
                        const aScore = Number.isFinite(aMs) ? aMs : Number.NEGATIVE_INFINITY;
                        const bScore = Number.isFinite(bMs) ? bMs : Number.NEGATIVE_INFINITY;
                        return bScore - aScore;
                    })
                    .map((entry) => entry.camera_id)
                    .find((camera) => cameras.includes(camera)) ??
                cameras.find((camera) => camera !== "default") ??
                cameras[0] ??
                "";
            const nextRoutes = new Map(channelVideoRoutes);
            for (const channel of labjackConfig.sensor_settings.channels_enabled) {
                const existing = nextRoutes.get(channel);
                const currentCamera = existing?.cameraId?.trim() ?? "";
                const cameraId = cameras.length > 0
                    ? (cameras.includes(currentCamera) ? currentCamera : preferred)
                    : currentCamera;
                nextRoutes.set(channel, {
                    enabled: existing?.enabled ?? true,
                    cameraId
                });
            }
            channelVideoRoutes = nextRoutes;
        } catch (err) {
            cameraListError =
                err instanceof Error ? err.message : "Failed to load available cameras";
        } finally {
            cameraListLoading = false;
        }
    }
    
    function addDataChunk(channel: number, chunk: DataPoint[]) {
        const currentData = channelData.get(channel) || [];
        
        let newData = currentData.concat(chunk);
        
        if (newData.length > maxDataPoints) {
            newData = newData.slice(newData.length - maxDataPoints);
        }
        
        channelData.set(channel, newData);

        const channelMode = channelModes.get(channel) ?? "free_run";
        if (channelMode === "trigger_normal" || channelMode === "trigger_single") {
            processTriggerMode(channel, channelMode, newData, chunk);
        } else {
            if (!(channelPrebufferReady.get(channel) ?? false)) {
                channelPrebufferReady.set(channel, true);
                channelPrebufferReady = new Map(channelPrebufferReady);
            }
        }
        
        channelData = new Map(channelData);
    }

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
            if (triggerTime > 0 && newChunk.length > 0) {
                const state = channelFrozenCapture.get(channel);
                if (!state || state.triggerTime !== triggerTime) {
                    initializeFrozenCapture(channel, fullData, triggerTime, channelTriggerSetting);
                }
                appendFrozenDataFromChunk(channel, newChunk);
            }

            if (isSingleShot) {
                return;
            }

            const rearmAt = channelRearmTime.get(channel) || 0;
            const releaseAt = Math.max(rearmAt, postWindowEnd);
            if (lastTimestamp < releaseAt) {
                return;
            }

            clearTriggerState(channel, false);
        }

        checkTriggerCondition(channel, fullData, newChunk);
    }

    function clearTriggerState(channel: number, clearFrozen: boolean = true) {
        channelTriggered.set(channel, false);
        channelTriggerTime.set(channel, 0);
        channelRearmTime.set(channel, 0);
        channelFrozenCapture.delete(channel);

        if (clearFrozen) {
            frozenChannelData.set(channel, []);
            frozenChannelData = new Map(frozenChannelData);
        }

        channelTriggered = new Map(channelTriggered);
        channelTriggerTime = new Map(channelTriggerTime);
        channelRearmTime = new Map(channelRearmTime);
    }
    
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
                const holdoffMs = Math.max(0, channelTriggerSetting.holdoffMs || 0);
                channelTriggered.set(channel, true);
                channelTriggerTime.set(channel, currentPoint.timestamp);
                channelRearmTime.set(channel, currentPoint.timestamp + holdoffMs);
                channelTriggered = new Map(channelTriggered);
                channelTriggerTime = new Map(channelTriggerTime);
                channelRearmTime = new Map(channelRearmTime);
                
                initializeFrozenCapture(
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

    function initializeFrozenCapture(
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

        const lastCapturedTimestamp = frozenData.length > 0
            ? frozenData[frozenData.length - 1].timestamp
            : startTime;

        channelFrozenCapture.set(channel, {
            triggerTime,
            startTime,
            endTime,
            lastCapturedTimestamp
        });
        
        frozenChannelData.set(channel, frozenData);
        frozenChannelData = new Map(frozenChannelData);
    }

    function appendFrozenDataFromChunk(channel: number, newChunk: DataPoint[]) {
        const state = channelFrozenCapture.get(channel);
        if (!state || newChunk.length === 0) return;

        const additions: DataPoint[] = [];
        let lastCaptured = state.lastCapturedTimestamp;

        for (const point of newChunk) {
            if (point.timestamp <= lastCaptured) continue;
            if (point.timestamp < state.startTime) continue;
            if (point.timestamp > state.endTime) break;
            additions.push(point);
            lastCaptured = point.timestamp;
        }

        if (additions.length > 0) {
            const currentFrozen = frozenChannelData.get(channel) || [];
            frozenChannelData.set(channel, currentFrozen.concat(additions));
            frozenChannelData = new Map(frozenChannelData);
        }

        state.lastCapturedTimestamp = Math.max(state.lastCapturedTimestamp, lastCaptured);
        channelFrozenCapture.set(channel, state);
    }
    
    
    function resetChannelTrigger(channel: number) {
        clearTriggerState(channel, true);
    }

    function isTriggerMode(mode: ChannelPlotMode): boolean {
        return mode === "trigger_normal" || mode === "trigger_single";
    }

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
        triggerAutoSaveBackendSettings();
    }

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
    }

    function getTriggerWindows(settings: TriggerSettings | undefined) {
        const postWindowSec = Math.max(0.01, settings?.postTriggerWindowSec || 0.01);
        const preFraction = Math.min(0.95, Math.max(0, (settings?.preTriggerPercent || 0) / 100));
        const preWindowSec = postWindowSec * (preFraction / (1 - preFraction));
        return { preWindowSec, postWindowSec };
    }

    function hasRequiredPreBuffer(data: DataPoint[], settings: TriggerSettings): boolean {
        if (data.length < 2) return false;
        const { preWindowSec } = getTriggerWindows(settings);
        const requiredMs = preWindowSec * 1000;
        const oldest = data[0]?.timestamp;
        const latest = data[data.length - 1]?.timestamp;
        if (!Number.isFinite(oldest) || !Number.isFinite(latest)) return false;
        return (latest - oldest) >= requiredMs;
    }

    function getHoldoffRemainingMs(channel: number): number {
        const rearmAt = channelRearmTime.get(channel) || 0;
        if (rearmAt <= 0) return 0;
        const latestChannelPoint = channelData.get(channel)?.at(-1);
        const referenceTime =
            latestChannelPoint && Number.isFinite(latestChannelPoint.timestamp)
                ? latestChannelPoint.timestamp
                : uiNow;
        return Math.max(0, rearmAt - referenceTime);
    }

    function getPlotConfig(channel: number) {
        const mode = channelModes.get(channel) ?? "free_run";
        const liveData = channelDisplayData.get(channel) || [];
        const frozenData = frozenDisplayData.get(channel) || [];
        const isTriggered = channelTriggered.get(channel) || false;
        const triggerTime = channelTriggerTime.get(channel) || 0;
        const triggerConfig = triggerSettings.get(channel);
        const { preWindowSec, postWindowSec } = getTriggerWindows(triggerConfig);
        const isFrozenCollecting =
            isTriggered &&
            triggerTime > 0 &&
            uiNow < (triggerTime + postWindowSec * 1000);

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
    
    function goBack() {
        window.location.href = "/labjacks";
    }

    function toLocalInputValue(date: Date, includeSeconds: boolean = false): string {
        const pad = (value: number) => value.toString().padStart(2, "0");
        const base = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
        if (!includeSeconds) return base;
        return `${base}:${pad(date.getSeconds())}`;
    }

    function toRfc3339(value: string): string {
        const date = new Date(value);
        if (isNaN(date.getTime())) {
            throw new Error("Invalid date/time value");
        }
        return date.toISOString();
    }

    function getChannelVideoRoute(channel: number): ChannelVideoRoute {
        const current = channelVideoRoutes.get(channel);
        if (current) return current;
        return {
            enabled: true,
            cameraId: availableCameraIds.find((camera) => camera !== "default") ?? availableCameraIds[0] ?? ""
        };
    }

    function setChannelVideoRoute(channel: number, patch: Partial<ChannelVideoRoute>) {
        const current = getChannelVideoRoute(channel);
        channelVideoRoutes.set(channel, { ...current, ...patch });
        channelVideoRoutes = new Map(channelVideoRoutes);
    }

    function resolveCameraIdForChannel(channel: number, requireEnabled: boolean): string | undefined {
        const route = getChannelVideoRoute(channel);
        if (requireEnabled && !route.enabled) {
            return undefined;
        }
        const fallbackCamera = availableCameraIds.find((camera) => camera !== "default") ?? availableCameraIds[0] ?? "";
        const trimmed = (route.cameraId.trim() || fallbackCamera).trim();
        return trimmed.length > 0 ? trimmed : undefined;
    }

    function parseIsoMs(value: string): number {
        const ms = new Date(value).getTime();
        return Number.isFinite(ms) ? ms : Number.NaN;
    }

    function getCameraCoverage(cameraId: string | undefined): VideoCameraCoverage | undefined {
        if (!cameraId) return undefined;
        return cameraCoverageById.get(cameraId);
    }

    function formatCoverageTime(iso: string): string {
        const ms = parseIsoMs(iso);
        if (!Number.isFinite(ms)) return iso;
        return new Date(ms).toLocaleString();
    }

    function parseCameraIdFromObjectKey(objectKey: string): string | undefined {
        const parts = objectKey.split("/");
        const cameraPart = parts.find((part) => part.startsWith("camera_"));
        if (!cameraPart) return undefined;
        const cameraId = cameraPart.slice("camera_".length).trim();
        return cameraId.length > 0 ? cameraId : undefined;
    }

    function selectClipObjectKey(event: BackendTriggerEvent, preferredCameraId?: string): string | undefined {
        const clipKeys = event.record_clip_keys ?? [];
        if (clipKeys.length === 0) return undefined;
        if (preferredCameraId) {
            const preferred = clipKeys.find((key) => parseCameraIdFromObjectKey(key) === preferredCameraId);
            if (preferred) return preferred;
        }
        return clipKeys[0];
    }

    function getCoverageReadinessForCamera(event: BackendTriggerEvent, cameraId: string): TriggerFetchReadiness {
        const coverage = getCameraCoverage(cameraId);
        if (!coverage) {
            return {
                ready: false,
                message: `Coverage not available yet for ${cameraId}.`
            };
        }

        const eventMs = Number.isFinite(event.trigger_time_unix_ms)
            ? event.trigger_time_unix_ms
            : parseIsoMs(event.trigger_time);
        const contiguousStartMs = parseIsoMs(coverage.contiguous_start);
        const contiguousEndMs = parseIsoMs(coverage.contiguous_end);
        if (Number.isFinite(eventMs) && Number.isFinite(contiguousStartMs) && eventMs < contiguousStartMs) {
            return {
                ready: true,
                message: `${cameraId} historical trigger (before latest contiguous window); fetch will try archived objects`
            };
        }
        if (
            Number.isFinite(eventMs) &&
            Number.isFinite(contiguousStartMs) &&
            Number.isFinite(contiguousEndMs) &&
            eventMs > contiguousEndMs
        ) {
            return {
                ready: false,
                message: `${cameraId} not caught up yet. Latest video ends at ${formatCoverageTime(coverage.contiguous_end)}`
            };
        }

        const minCenterMs = parseIsoMs(coverage.recommended_center_min);
        const maxCenterMs = parseIsoMs(coverage.recommended_center_max);
        if (Number.isFinite(minCenterMs) && Number.isFinite(maxCenterMs) && minCenterMs > maxCenterMs) {
            return {
                ready: false,
                message: `${cameraId} latest contiguous window is shorter than 10s; waiting for more post-roll`
            };
        }
        if ((!Number.isFinite(minCenterMs) && !Number.isFinite(maxCenterMs)) || !Number.isFinite(eventMs)) {
            return { ready: true, message: `${cameraId} coverage is available` };
        }
        if (Number.isFinite(minCenterMs) && eventMs < minCenterMs) {
            return {
                ready: false,
                message: `${cameraId} has no continuous pre-roll for this trigger. Earliest fetchable center is ${formatCoverageTime(
                    coverage.recommended_center_min
                )}`
            };
        }
        if (eventMs > maxCenterMs) {
            return {
                ready: false,
                message: `${cameraId} not caught up yet. Fetchable until ${formatCoverageTime(
                    coverage.recommended_center_max
                )}`
            };
        }
        return {
            ready: true,
            message: `${cameraId} raw coverage ready (${formatCoverageTime(
                coverage.recommended_center_min
            )} to ${formatCoverageTime(coverage.recommended_center_max)})`
        };
    }

    function getRawCoverageReadiness(event: BackendTriggerEvent): TriggerFetchReadiness {
        const primaryCameraId = resolveCameraIdForChannel(event.channel, false);
        if (!primaryCameraId) {
            return {
                ready: false,
                message: `Video disabled for ch${event.channel.toString().padStart(2, "0")}`
            };
        }

        const primaryReadiness = getCoverageReadinessForCamera(event, primaryCameraId);
        if (primaryReadiness.ready) {
            return primaryReadiness;
        }

        for (const fallbackCameraId of availableCameraIds) {
            if (fallbackCameraId === primaryCameraId) continue;
            const fallbackReadiness = getCoverageReadinessForCamera(event, fallbackCameraId);
            if (fallbackReadiness.ready) {
                return {
                    ready: true,
                    message: `${primaryCameraId} not ready, using fallback ${fallbackCameraId}`
                };
            }
        }

        return primaryReadiness;
    }

    function getTriggerFetchReadiness(event: BackendTriggerEvent): TriggerFetchReadiness {
        const preferredCameraId = resolveCameraIdForChannel(event.channel, false);
        const compactedClipKey = selectClipObjectKey(event, preferredCameraId);
        if (compactedClipKey) {
            const compactedCamera = parseCameraIdFromObjectKey(compactedClipKey) ?? "camera";
            return {
                ready: true,
                message: `Compacted clip ready for ${compactedCamera}`
            };
        }

        const rawReadiness = getRawCoverageReadiness(event);
        if (event.record_status === "pending" && !rawReadiness.ready) {
            const reason = event.record_last_error
                ? ` Last worker error: ${event.record_last_error}`
                : "";
            return {
                ready: false,
                message: `Clip is pending upload/compaction.${reason} ${rawReadiness.message}`.trim()
            };
        }
        if (event.record_status === "processed" && !rawReadiness.ready) {
            return {
                ready: false,
                message: `Clip record is processed but no clip object key is available. ${rawReadiness.message}`
            };
        }
        return rawReadiness;
    }

    function triggerAutoSaveBackendSettings() {
        triggerSettingsDirty = true;
        backendTriggerStatus = "Trigger settings changed. Saving to backend...";
        if (triggerAutoSaveTimer) {
            clearTimeout(triggerAutoSaveTimer);
        }
        triggerAutoSaveTimer = setTimeout(() => {
            void saveBackendTriggerSettings(true);
        }, 500);
    }

    function updateTriggerSetting(channel: number, patch: Partial<TriggerSettings>, saveToBackend: boolean = false) {
        const setting = triggerSettings.get(channel);
        if (!setting) return;
        triggerSettings.set(channel, { ...setting, ...patch });
        triggerSettings = new Map(triggerSettings);
        if (saveToBackend) {
            triggerAutoSaveBackendSettings();
        }
    }

    function downloadBlob(blob: Blob, filename: string) {
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = filename;
        document.body.appendChild(link);
        link.click();
        link.remove();
        URL.revokeObjectURL(url);
    }

    function clearVideoPreview() {
        if (videoUrl) {
            URL.revokeObjectURL(videoUrl);
            videoUrl = "";
        }
        videoFileName = "";
    }

    async function requestClip(centerIso: string, cameraIdOverride?: string) {
        if (!labjackConfig) {
            throw new Error("Configuration not loaded");
        }
        const payload: VideoClipRequestPayload = {
            asset: labjackConfig.asset_number,
            camera_id: cameraIdOverride,
            center_time: centerIso,
            pre_sec: 5,
            post_sec: 5
        };
        return await requestVideoClip(payload);
    }

    async function fetchClipByCenterIso(
        centerIso: string,
        sourceLabel: string,
        cameraIdOverride?: string
    ) {
        if (!labjackConfig) {
            videoError = "Configuration not loaded";
            return;
        }

        videoLoading = true;
        videoError = "";

        try {
            const result = await requestClip(centerIso, cameraIdOverride);
            clearVideoPreview();
            videoUrl = URL.createObjectURL(result.blob);
            videoFileName = result.filename;
        } catch (err) {
            clearVideoPreview();
            videoError = `${sourceLabel}: ${
                err instanceof Error ? err.message : "Failed to fetch video clip"
            }`;
        } finally {
            videoLoading = false;
        }
    }

    async function fetchClipForTriggerEvent(event: BackendTriggerEvent, sourceLabel: string) {
        manualVideoChannel = event.channel;
        const readiness = getTriggerFetchReadiness(event);
        const preferredCameraId = resolveCameraIdForChannel(event.channel, false);
        const compactedClipKey = selectClipObjectKey(event, preferredCameraId);

        videoLoading = true;
        videoError = "";

        let lastError = readiness.ready ? "" : readiness.message;
        try {
            if (compactedClipKey) {
                try {
                    const result = await requestVideoObject({ object_key: compactedClipKey });
                    clearVideoPreview();
                    videoUrl = URL.createObjectURL(result.blob);
                    videoFileName = result.filename;
                    backendTriggerStatus = `Fetched compacted clip ${result.filename}`;
                    return;
                } catch (err) {
                    lastError = err instanceof Error ? err.message : "Failed to fetch compacted clip";
                }
            }

            const primaryCameraId = resolveCameraIdForChannel(event.channel, false);
            if (!primaryCameraId) {
                videoError = readiness.message || "Camera is not configured for this channel";
                return;
            }

            const fallbackCandidates = availableCameraIds.filter((cameraId) => cameraId !== primaryCameraId);
            const candidateCameraIds = [primaryCameraId, ...fallbackCandidates];
            if (!readiness.ready && candidateCameraIds.length === 1) {
                videoError = readiness.message;
                return;
            }

            for (const cameraId of candidateCameraIds) {
                try {
                    const result = await requestClip(event.trigger_time, cameraId);
                    clearVideoPreview();
                    videoUrl = URL.createObjectURL(result.blob);
                    videoFileName = result.filename;
                    if (cameraId !== primaryCameraId) {
                        backendTriggerStatus = `Fetched clip using fallback camera ${cameraId} (primary ${primaryCameraId} had no full 10s coverage).`;
                    }
                    return;
                } catch (err) {
                    lastError = err instanceof Error ? err.message : "Failed to fetch video clip";
                }
            }

            clearVideoPreview();
            videoError = `${sourceLabel}: ${lastError || "Failed to fetch video clip"}`;
        } finally {
            videoLoading = false;
        }
    }

    function buildBackendTriggerSettings() {
        const settings: Record<string, BackendTriggerSetting> = {};
        for (const [channel, trigger] of triggerSettings.entries()) {
            const mode = channelModes.get(channel) ?? "free_run";
            settings[String(channel)] = {
                enabled: mode === "trigger_normal",
                type: trigger.type,
                threshold: Number.isFinite(trigger.threshold) ? trigger.threshold : 0,
                holdoff_ms: Math.max(0, Math.round(trigger.holdoffMs || 0))
            };
        }
        return settings;
    }

    async function saveBackendTriggerSettings(auto: boolean = false) {
        if (!natsService || !labjackConfig || !labjackConfigKey) {
            backendTriggerError = "LabJack config key not loaded";
            return;
        }

        backendTriggerSaving = true;
        backendTriggerError = "";
        if (!auto) {
            backendTriggerStatus = "";
        }

        try {
            const nextConfig = labjackConfigRaw && typeof labjackConfigRaw === "object"
                ? JSON.parse(JSON.stringify(labjackConfigRaw))
                : JSON.parse(JSON.stringify(labjackConfig));

            if (!nextConfig.sensor_settings || typeof nextConfig.sensor_settings !== "object") {
                nextConfig.sensor_settings = {};
            }
            nextConfig.sensor_settings.trigger_settings = buildBackendTriggerSettings();

            await putKeyValue(
                natsService,
                "avenabox",
                labjackConfigKey,
                JSON.stringify(nextConfig, null, 2)
            );

            labjackConfigRaw = nextConfig;
            labjackConfig = normalizeLabJackConfig(nextConfig);
            triggerSettingsDirty = false;
            backendTriggerStatus = `Saved trigger settings to ${labjackConfigKey}`;
        } catch (err) {
            backendTriggerError =
                err instanceof Error ? err.message : "Failed to save trigger settings";
        } finally {
            backendTriggerSaving = false;
        }
    }

    function formatTriggerEventTime(value: string): string {
        const dt = new Date(value);
        if (isNaN(dt.getTime())) return value;
        return dt.toLocaleString();
    }

    async function searchTriggerEventsInRange() {
        if (!natsService || !labjackConfig) {
            triggerRangeError = "NATS connection or config unavailable";
            return;
        }

        const hasStart = triggerRangeStart.trim().length > 0;
        const hasEnd = triggerRangeEnd.trim().length > 0;
        if (!hasStart || !hasEnd) {
            triggerRangeError = "Please set both start and end time";
            return;
        }

        let startMs = Number.NaN;
        let endMs = Number.NaN;
        try {
            startMs = new Date(toRfc3339(triggerRangeStart)).getTime();
            endMs = new Date(toRfc3339(triggerRangeEnd)).getTime();
        } catch {
            triggerRangeError = "Invalid date/time range";
            return;
        }
        if (!Number.isFinite(startMs) || !Number.isFinite(endMs) || endMs < startMs) {
            triggerRangeError = "Invalid range (end must be after start)";
            return;
        }

        triggerRangeLoading = true;
        triggerRangeError = "";
        triggerRangeStatus = "";
        triggerRangeResults = [];

        try {
            const keys = (await getKeys(natsService, "video_trigger_events")).filter((key) =>
                key.startsWith("event.")
            );
            const matching: BackendTriggerEvent[] = [];

            for (const key of keys) {
                try {
                    const raw = await getKeyValue(natsService, "video_trigger_events", key);
                    if (!raw || raw === "Key value does not exist") continue;
                    const event = parseTriggerRecordEntryObject(JSON.parse(raw));
                    if (!event) continue;
                    if (event.asset !== labjackConfig.asset_number) continue;
                    const eventMs = Number.isFinite(event.trigger_time_unix_ms)
                        ? event.trigger_time_unix_ms
                        : new Date(event.trigger_time).getTime();
                    if (!Number.isFinite(eventMs)) continue;
                    if (eventMs >= startMs && eventMs <= endMs) {
                        matching.push(event);
                    }
                } catch {
                    continue;
                }
            }

            matching.sort((a, b) => b.trigger_time_unix_ms - a.trigger_time_unix_ms);
            triggerRangeResults = matching.slice(0, 200);
            triggerRangeStatus = `Found ${matching.length} trigger(s) in range${matching.length > 200 ? " (showing latest 200)" : ""}.`;
            if (matching.length === 0 && backendTriggerEvents.length > 0) {
                const latestRecent = backendTriggerEvents[0];
                const latestRecentMs = Number.isFinite(latestRecent.trigger_time_unix_ms)
                    ? latestRecent.trigger_time_unix_ms
                    : parseIsoMs(latestRecent.trigger_time);
                if (Number.isFinite(latestRecentMs) && latestRecentMs > endMs) {
                    triggerRangeStatus += ` Latest recent trigger is at ${formatTriggerEventTime(
                        latestRecent.trigger_time
                    )}, which is after the selected End time.`;
                }
            }
        } catch (err) {
            triggerRangeError =
                err instanceof Error ? err.message : "Failed searching trigger history";
        } finally {
            triggerRangeLoading = false;
        }
    }

    async function fetchSelectedRangeClips() {
        if (triggerRangeResults.length === 0) {
            triggerRangeError = "No triggers in range to fetch";
            return;
        }
        triggerRangeFetchLoading = true;
        triggerRangeError = "";
        let fetched = 0;
        let skipped = 0;
        let failed = 0;
        const failures: string[] = [];
        const maxFetch = 10;

        for (const event of triggerRangeResults.slice(0, maxFetch)) {
            const readiness = getTriggerFetchReadiness(event);
            if (!readiness.ready) {
                skipped += 1;
                continue;
            }

            try {
                const preferredCameraId = resolveCameraIdForChannel(event.channel, false);
                const compactedClipKey = selectClipObjectKey(event, preferredCameraId);
                const rawCameraId = resolveCameraIdForChannel(event.channel, false);
                if (compactedClipKey) {
                    const result = await requestVideoObject({ object_key: compactedClipKey });
                    downloadBlob(result.blob, result.filename || `clip_${event.channel}_${event.trigger_time_unix_ms}.mp4`);
                } else {
                    if (!rawCameraId) {
                        throw new Error(`Video disabled for ch${event.channel.toString().padStart(2, "0")}`);
                    }
                    const result = await requestVideoClip({
                        asset: event.asset,
                        camera_id: rawCameraId,
                        center_time: event.trigger_time,
                        pre_sec: 5,
                        post_sec: 5
                    });
                    downloadBlob(result.blob, result.filename || `clip_${event.channel}_${event.trigger_time_unix_ms}.mp4`);
                }
                fetched += 1;
            } catch (err) {
                failed += 1;
                failures.push(err instanceof Error ? err.message : "Failed fetching clip");
            }
        }

        triggerRangeStatus = `Fetched ${fetched} clip(s) from range (max ${maxFetch} per action).` +
            (skipped > 0 ? ` Skipped ${skipped} not-ready trigger(s).` : "");
        if (failed > 0) {
            triggerRangeError = `Failed ${failed} clip(s): ${failures[0]}`;
        }
        triggerRangeFetchLoading = false;
    }

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

    function closeExportModal() {
        showExportModal = false;
        exporting = false;
        exportWarning = "";
    }

    function toggleExportChannel(channel: number, checked: boolean) {
        const updated = new Set(exportChannels);
        if (checked) {
            updated.add(channel);
        } else {
            updated.delete(channel);
        }
        exportChannels = updated;
    }

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
                download_name: labjackConfig.labjack_name
                    ? `${labjackConfig.labjack_name.replace(/\s+/g, "_")}.csv`
                    : undefined,
            };

            const result = await downloadExportViaWebSocket(payload, {
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

    onMount(() => {
        uiNowTimer = setInterval(() => {
            uiNow = Date.now();
        }, 100);
        triggerRecordRefreshTimer = setInterval(() => {
            void refreshRecentTriggerRecordStates();
        }, 3000);
        if (!triggerRangeEnd) {
            triggerRangeEnd = toLocalInputValue(new Date(), true);
        }
        if (!triggerRangeStart) {
            triggerRangeStart = toLocalInputValue(new Date(Date.now() - 30 * 60 * 1000), true);
        }
    });
    
    onDestroy(() => {
        clearVideoPreview();

        if (triggerAutoSaveTimer) {
            clearTimeout(triggerAutoSaveTimer);
            triggerAutoSaveTimer = null;
        }

        if (uiNowTimer) {
            clearInterval(uiNowTimer);
            uiNowTimer = null;
        }
        if (triggerRecordRefreshTimer) {
            clearInterval(triggerRecordRefreshTimer);
            triggerRecordRefreshTimer = null;
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
            <!-- Connection Status -->
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
                            <span>Sampling Rate:</span>
                            <span class="badge badge-info badge-sm">{labjackConfig.sensor_settings.sampling_rate} Hz</span>
                        </div>
                        <div class="flex justify-between">
                            <span>Enabled Channels:</span>
                            <span class="badge badge-secondary badge-sm">{labjackConfig.sensor_settings.channels_enabled.join(', ')}</span>
                        </div>
                        <div class="flex justify-between">
                            <span>NATS Subject Pattern:</span>
                            <span class="badge badge-accent badge-sm font-mono">{labjackConfig.nats_subject}.{labjackConfig.asset_number}.data.ch##</span>
                        </div>
                        <div class="flex justify-between">
                            <span>Channel Data Status:</span>
                            <div class="flex flex-wrap gap-1">
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
                            <span class="badge badge-info badge-sm">FlatBuffer with RFC3339 timestamps</span>
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
                <div class="card-body gap-4">
                    <div class="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                        <div>
                            <h4 class="card-title text-base-content">Backend Trigger Integration</h4>
                            <p class="text-sm text-base-content/70">
                                Threshold/edge/holdoff auto-save to backend KV. Only channels in Trigger Normal are enabled on backend.
                            </p>
                        </div>
                        <button
                            class="btn btn-secondary"
                            onclick={() => saveBackendTriggerSettings(false)}
                            disabled={backendTriggerSaving || !labjackConfigKey}
                        >
                            {backendTriggerSaving ? "Saving..." : "Save Now"}
                        </button>
                    </div>

                    {#if triggerSettingsDirty}
                        <div class="badge badge-warning">Pending backend save</div>
                    {/if}

                    {#if backendTriggerError}
                        <div class="alert alert-error text-sm">
                            <span>{backendTriggerError}</span>
                        </div>
                    {/if}

                    {#if backendTriggerStatus}
                        <div class="alert alert-success text-sm">
                            <span>{backendTriggerStatus}</span>
                        </div>
                    {/if}

                    <div class="divider my-1">Per-Channel Video Routing</div>
                    <p class="text-sm text-base-content/70">
                        Video fetch stays manual. Toggle per channel to choose camera feed used for trigger clip actions.
                    </p>
                    <div class="overflow-x-auto">
                        <table class="table table-zebra table-sm">
                            <thead>
                                <tr>
                                    <th>Channel</th>
                                    <th>Use Video Feed</th>
                                    <th>Camera</th>
                                </tr>
                            </thead>
                            <tbody>
                                {#each labjackConfig.sensor_settings.channels_enabled as channel}
                                    {@const route = getChannelVideoRoute(channel)}
                                    <tr>
                                        <td>ch{channel.toString().padStart(2, "0")}</td>
                                        <td>
                                            <input
                                                type="checkbox"
                                                class="toggle toggle-primary toggle-sm"
                                                checked={route.enabled}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLInputElement) {
                                                        setChannelVideoRoute(channel, { enabled: e.target.checked });
                                                    }
                                                }}
                                            />
                                        </td>
                                        <td>
                                            {#if availableCameraIds.length > 0}
                                                <select
                                                    class="select select-bordered select-sm"
                                                    value={route.cameraId}
                                                    disabled={cameraListLoading}
                                                    onchange={(e) => {
                                                        if (e.target instanceof HTMLSelectElement) {
                                                            setChannelVideoRoute(channel, { cameraId: e.target.value });
                                                        }
                                                    }}
                                                >
                                                    {#each availableCameraIds as cameraId}
                                                        <option value={cameraId}>{cameraId}</option>
                                                    {/each}
                                                </select>
                                            {:else}
                                                <span class="text-xs text-base-content/70">No cameras discovered</span>
                                            {/if}
                                        </td>
                                    </tr>
                                {/each}
                            </tbody>
                        </table>
                    </div>
                    <div class="flex items-center gap-3">
                        <button
                            class="btn btn-outline btn-sm"
                            onclick={refreshAvailableCameras}
                            disabled={cameraListLoading || videoLoading}
                        >
                            {cameraListLoading ? "Refreshing..." : "Refresh Camera List"}
                        </button>
                        {#if cameraListError}
                            <span class="text-xs text-error">{cameraListError}</span>
                        {/if}
                    </div>
                    <div class="text-xs text-base-content/70">
                        {#if cameraCoverageById.size === 0}
                            Coverage not available yet. Refresh camera list to update fetch readiness.
                        {:else}
                            {#each Array.from(cameraCoverageById.entries()) as [cameraId, coverage]}
                                <div>
                                    <span class="font-semibold">{cameraId}</span>:
                                    fetchable center time {formatCoverageTime(coverage.recommended_center_min)} to {formatCoverageTime(coverage.recommended_center_max)}
                                </div>
                            {/each}
                        {/if}
                    </div>

                    <div class="divider my-1">Recent Backend Trigger Events (Latest 10)</div>
                    {#if backendTriggerEvents.length === 0}
                        <p class="text-sm text-base-content/70">No backend trigger events received yet.</p>
                    {:else}
                        <div class="overflow-x-auto">
                            <table class="table table-zebra table-sm">
                                <thead>
                                    <tr>
                                        <th>Time</th>
                                        <th>Channel</th>
                                        <th>Type</th>
                                        <th>Calibrated</th>
                                        <th>Threshold</th>
                                        <th>Raw</th>
                                        <th>Clip</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {#each backendTriggerEvents.slice(0, 10) as event}
                                        {@const readiness = getTriggerFetchReadiness(event)}
                                        <tr>
                                            <td>{formatTriggerEventTime(event.trigger_time)}</td>
                                            <td>ch{event.channel.toString().padStart(2, "0")}</td>
                                            <td>{event.trigger_type}</td>
                                            <td>{event.calibrated_value.toFixed(4)}</td>
                                            <td>{event.threshold.toFixed(4)}</td>
                                            <td>{event.raw_value.toFixed(4)}</td>
                                            <td>
                                                <button
                                                    class="btn btn-primary btn-xs"
                                                    onclick={() => fetchClipForTriggerEvent(event, "Recent trigger")}
                                                    disabled={videoLoading || !readiness.ready}
                                                    title={readiness.message}
                                                >
                                                    Fetch
                                                </button>
                                            </td>
                                        </tr>
                                    {/each}
                                </tbody>
                            </table>
                        </div>
                    {/if}
                </div>
            </div>

            <div class="card bg-base-100 shadow-xl mb-6">
                <div class="card-body gap-4">
                    <h4 class="card-title text-base-content">Trigger History Search</h4>
                    <p class="text-sm text-base-content/70">
                        Find triggers in a date/time range, then fetch individual clips manually.
                    </p>

                    <div class="grid grid-cols-1 md:grid-cols-4 gap-4 items-end">
                        <div class="form-control">
                            <label class="label" for="trigger-range-start">
                                <span class="label-text">Start</span>
                            </label>
                            <input
                                id="trigger-range-start"
                                type="datetime-local"
                                step="1"
                                class="input input-bordered"
                                bind:value={triggerRangeStart}
                                disabled={triggerRangeLoading}
                            />
                        </div>
                        <div class="form-control">
                            <label class="label" for="trigger-range-end">
                                <span class="label-text">End</span>
                            </label>
                            <input
                                id="trigger-range-end"
                                type="datetime-local"
                                step="1"
                                class="input input-bordered"
                                bind:value={triggerRangeEnd}
                                disabled={triggerRangeLoading}
                            />
                        </div>
                        <div class="form-control">
                            <label class="label" for="manual-video-channel">
                                <span class="label-text">Manual Channel</span>
                            </label>
                            <select
                                id="manual-video-channel"
                                class="select select-bordered"
                                bind:value={manualVideoChannel}
                            >
                                {#each labjackConfig.sensor_settings.channels_enabled as channel}
                                    <option value={channel}>ch{channel.toString().padStart(2, "0")}</option>
                                {/each}
                            </select>
                        </div>
                        <div class="flex gap-2">
                            <button
                                class="btn btn-primary"
                                onclick={searchTriggerEventsInRange}
                                disabled={triggerRangeLoading}
                            >
                                {triggerRangeLoading ? "Searching..." : "Search"}
                            </button>
                            <button
                                class="btn btn-outline"
                                onclick={fetchSelectedRangeClips}
                                disabled={triggerRangeFetchLoading || triggerRangeResults.length === 0}
                            >
                                {triggerRangeFetchLoading ? "Fetching..." : "Fetch Top 10"}
                            </button>
                        </div>
                    </div>

                    {#if triggerRangeError}
                        <div class="alert alert-error text-sm">
                            <span>{triggerRangeError}</span>
                        </div>
                    {/if}
                    {#if triggerRangeStatus}
                        <div class="alert alert-info text-sm">
                            <span>{triggerRangeStatus}</span>
                        </div>
                    {/if}

                    {#if triggerRangeResults.length > 0}
                        <div class="overflow-x-auto">
                            <table class="table table-zebra table-sm">
                                <thead>
                                    <tr>
                                        <th>Time</th>
                                        <th>Channel</th>
                                        <th>Type</th>
                                        <th>Calibrated</th>
                                        <th>Clip</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {#each triggerRangeResults as event}
                                        {@const readiness = getTriggerFetchReadiness(event)}
                                        <tr>
                                            <td>{formatTriggerEventTime(event.trigger_time)}</td>
                                            <td>ch{event.channel.toString().padStart(2, "0")}</td>
                                            <td>{event.trigger_type}</td>
                                            <td>{event.calibrated_value.toFixed(4)}</td>
                                            <td>
                                                <button
                                                    class="btn btn-primary btn-xs"
                                                    onclick={() => fetchClipForTriggerEvent(event, "Range trigger")}
                                                    disabled={videoLoading || !readiness.ready}
                                                    title={readiness.message}
                                                >
                                                    Fetch
                                                </button>
                                            </td>
                                        </tr>
                                    {/each}
                                </tbody>
                            </table>
                        </div>
                    {/if}

                    {#if videoError}
                        <div class="alert alert-error text-sm">
                            <span>{videoError}</span>
                        </div>
                    {/if}

                    {#if videoUrl}
                        <div class="space-y-3">
                            <video class="w-full rounded-lg bg-black max-h-96" controls src={videoUrl}>
                                <track kind="captions" srclang="en" label="English captions" src="/empty-captions.vtt" />
                            </video>
                            <a class="btn btn-outline btn-sm" href={videoUrl} download={videoFileName || `clip_asset${assetNumber}.mp4`}>
                                Download {videoFileName || "clip"}
                            </a>
                        </div>
                    {/if}
                </div>
            </div>


            <!-- Channel Sections: Combined Trigger Settings + Plots -->
            <div class="space-y-6">
                {#each labjackConfig.sensor_settings.channels_enabled as channel, index}
                    {@const channelTriggerSetting = triggerSettings.get(channel)}
                    {@const channelMode = channelModes.get(channel) ?? 'free_run'}
                    {@const channelAxis = axisSettings.get(channel)}
                    {@const plotConfig = getPlotConfig(channel)}
                    {@const isChannelTriggered = channelTriggered.get(channel) || false}
                    {@const channelTriggerTimeValue = channelTriggerTime.get(channel) || 0}
                    {@const isPrebufferReady = channelPrebufferReady.get(channel) ?? false}
                    
                    <!-- Combined Channel Section -->
                    <div class="card bg-base-100 shadow-xl border border-base-200">
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
                                        {#if channelMode === 'trigger_normal' && getHoldoffRemainingMs(channel) > 0}
                                            <span class="text-xs text-warning">
                                                Holdoff: {(getHoldoffRemainingMs(channel) / 1000).toFixed(2)}s
                                            </span>
                                        {/if}
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
                                                    const parsed = parseFloat(e.target.value);
                                                    updateAxisSettings(channel, {
                                                        xWindowSec: Number.isFinite(parsed) ? parsed : timeWindow
                                                    });
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
                                            value={channelAxis?.yMin ?? DEFAULT_MANUAL_Y_MIN}
                                            disabled={channelAxis?.autoY ?? true}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    const parsed = parseFloat(e.target.value);
                                                    updateAxisSettings(channel, {
                                                        yMin: Number.isFinite(parsed) ? parsed : DEFAULT_MANUAL_Y_MIN
                                                    });
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
                                            value={channelAxis?.yMax ?? DEFAULT_MANUAL_Y_MAX}
                                            disabled={channelAxis?.autoY ?? true}
                                            onchange={(e) => {
                                                if (e.target instanceof HTMLInputElement) {
                                                    const parsed = parseFloat(e.target.value);
                                                    updateAxisSettings(channel, {
                                                        yMax: Number.isFinite(parsed) ? parsed : DEFAULT_MANUAL_Y_MAX
                                                    });
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
                                    <h4 class="text-md font-medium text-base-content mb-4">Trigger Settings</h4>
                                    <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-6 gap-4">
                                        <div class="form-control">
                                            <label class="label" for="trigger-type-{channel}">
                                                <span class="label-text">Trigger Edge</span>
                                            </label>
                                            <select
                                                id="trigger-type-{channel}"
                                                value={channelTriggerSetting?.type || 'rising'}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLSelectElement) {
                                                        updateTriggerSetting(
                                                            channel,
                                                            { type: e.target.value as 'rising' | 'falling' },
                                                            true
                                                        );
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
                                                    if (e.target instanceof HTMLInputElement) {
                                                        updateTriggerSetting(
                                                            channel,
                                                            { threshold: parseFloat(e.target.value) || 0 },
                                                            true
                                                        );
                                                    }
                                                }}
                                                class="input input-bordered input-warning"
                                            />
                                        </div>
                                        <div class="form-control">
                                            <label class="label" for="trigger-holdoff-{channel}">
                                                <span class="label-text">Holdoff (ms)</span>
                                            </label>
                                            <input
                                                id="trigger-holdoff-{channel}"
                                                type="number"
                                                min="0"
                                                step="10"
                                                value={channelTriggerSetting?.holdoffMs || 0}
                                                onchange={(e) => {
                                                    if (e.target instanceof HTMLInputElement) {
                                                        updateTriggerSetting(
                                                            channel,
                                                            { holdoffMs: Math.max(0, parseInt(e.target.value, 10) || 0) },
                                                            true
                                                        );
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
                                                    if (e.target instanceof HTMLInputElement) {
                                                        const raw = parseInt(e.target.value, 10) || 0;
                                                        updateTriggerSetting(channel, {
                                                            preTriggerPercent: Math.min(95, Math.max(0, raw))
                                                        });
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
                                                    if (e.target instanceof HTMLInputElement) {
                                                        const raw = parseFloat(e.target.value) || 0.01;
                                                        updateTriggerSetting(channel, {
                                                            postTriggerWindowSec: Math.max(0.01, raw)
                                                        });
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
                                            {#if isChannelTriggered && channelMode === 'trigger_normal' && getHoldoffRemainingMs(channel) > 0}
                                                <span class="text-xs text-base-content/70 mt-2">
                                                    Holdoff remaining {(getHoldoffRemainingMs(channel) / 1000).toFixed(2)}s
                                                </span>
                                            {/if}
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
                                    holdoffRemainingMs={getHoldoffRemainingMs(channel)}
                                    prebuffering={isTriggerMode(channelMode) && !isChannelTriggered && !isPrebufferReady}
                                    yAutoScale={channelAxis?.autoY ?? true}
                                    yMin={channelAxis?.yMin ?? DEFAULT_MANUAL_Y_MIN}
                                    yMax={channelAxis?.yMax ?? DEFAULT_MANUAL_Y_MAX}
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
