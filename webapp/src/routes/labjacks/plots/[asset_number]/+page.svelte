<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { page } from "$app/stores";
    import { connect, getKeyValue, getKeys } from "$lib/nats.svelte";
    import { downloadExportViaWebSocket, type ExportRequestPayload } from "$lib/exporter";
    import { applyCalibration, normalizeCalibration, type CalibrationSpec } from "$lib/calibration";
    import RealTimePlot from "$lib/components/RealTimePlot.svelte";
    import { FlatBufferParser, calculateSampleTimestamps } from "$lib/flatbuffer-parser";
    // @ts-ignore - No type definitions available for downsample-lttb
    import downsampler from 'downsample-lttb';

    
    interface SensorSettings {
        scan_rate: number;
        sampling_rate: number;
        channels_enabled: number[];
        gains: number;
        data_formats: string[];
        measurement_units: string[];
        labjack_on_off: boolean;
        calibrations?: Record<string, CalibrationSpec>;
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
        calibrations: {}
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

    type ChannelPlotMode = 'free_run' | 'trigger_normal' | 'trigger_single' | 'hold';

    interface TriggerSettings {
        type: 'rising' | 'falling';
        threshold: number;
        holdoffMs: number;
    }

    interface AxisSettings {
        autoY: boolean;
        yMin: number;
        yMax: number;
        invertX: boolean;
        invertY: boolean;
    }
    
    let assetNumber = $state<number>(0);
    let labjackConfig = $state<LabJackConfig | null>(null);
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    let natsService: any = null;
    let subscriptions: any[] = [];
    let channelData = $state<Map<number, DataPoint[]>>(new Map());
    let frozenChannelData = $state<Map<number, DataPoint[]>>(new Map());
    let heldChannelData = $state<Map<number, DataPoint[]>>(new Map());
    let channelModes = $state<Map<number, ChannelPlotMode>>(new Map());
    let axisSettings = $state<Map<number, AxisSettings>>(new Map());
    const channelLastTimestamp = new Map<number, number>();
    let isConnected = $state<boolean>(false);
    let flatBufferParser = new FlatBufferParser();
    let triggerSettings = $state<Map<number, TriggerSettings>>(new Map());
    let channelTriggered = $state<Map<number, boolean>>(new Map());
    let channelTriggerTime = $state<Map<number, number>>(new Map());
    let channelRearmTime = $state<Map<number, number>>(new Map());
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

    function updateMaxDataPoints() {
        if (!labjackConfig) return;
        const sr = labjackConfig.sensor_settings.sampling_rate;
        // The buffer must be large enough to hold data for the frozen capture window.
        // We want to see `timeWindow` seconds before and after the trigger.
        maxDataPoints = sr * timeWindow * 2; 
        console.log(`Max data points in rolling buffer: ${maxDataPoints}`);
    }

    let channelDisplayData = $derived(
        new Map(
            Array.from(channelData.entries()).map(([channel, data]) => {
                if (data.length <= 1000) { 
                    return [channel, data];
                }
                
                // 1. Convert data to the format the library expects: [x, y][]
                const formattedData = data.map(p => [p.timestamp, p.value]);
                
                // 2. Downsample using the .processData() method
                const downsampled = downsampler.processData(formattedData, 1000);
                
                // 3. Convert it back to our original {timestamp, value} format
                const restoredFormat = downsampled.map((p: [number, number]) => ({ timestamp: p[0], value: p[1] }));
                
                return [channel, restoredFormat];
            })
        )
    );

    let frozenDisplayData = $derived(
        new Map(
            Array.from(frozenChannelData.entries()).map(([channel, data]) => {
                // If the data array is small, don't downsample
                if (data.length <= 1000) { 
                    return [channel, data];
                }
                
                // 1. Convert data to the format the library expects: [x, y][]
                const formattedData = data.map(p => [p.timestamp, p.value]);
                
                // 2. Downsample the data to 1000 points
                const downsampled = downsampler.processData(formattedData, 1000);
                
                // 3. Convert it back to our {timestamp, value} format
                const restoredFormat = downsampled.map((p: [number, number]) => ({ timestamp: p[0], value: p[1] }));
                
                return [channel, restoredFormat];
            })
        )
    );

    
    async function loadLabJackConfig() {
        loading = true;
        error = "";
        
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
            
            if (foundConfig) {
                labjackConfig = foundConfig;
                updateMaxDataPoints();
                initializeChannelData();
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
    
    function initializeChannelData() {
        if (!labjackConfig) return;
        
        const newChannelData = new Map<number, DataPoint[]>();
        const newFrozenChannelData = new Map<number, DataPoint[]>();
        const newHeldChannelData = new Map<number, DataPoint[]>();
        const newChannelModes = new Map<number, ChannelPlotMode>();
        const newAxisSettings = new Map<number, AxisSettings>();
        const newTriggerSettings = new Map<number, TriggerSettings>();
        const newChannelTriggered = new Map<number, boolean>();
        const newChannelTriggerTime = new Map<number, number>();
        const newChannelRearmTime = new Map<number, number>();
        channelLastTimestamp.clear();
        
        labjackConfig.sensor_settings.channels_enabled.forEach(channel => {
            newChannelData.set(channel, []);
            newFrozenChannelData.set(channel, []);
            newHeldChannelData.set(channel, []);
            newChannelModes.set(channel, 'free_run');
            newAxisSettings.set(channel, {
                autoY: true,
                yMin: -1,
                yMax: 1,
                invertX: false,
                invertY: false
            });
            channelLastTimestamp.set(channel, Number.NaN);
            newTriggerSettings.set(channel, {
                type: 'rising',
                threshold: 0,
                holdoffMs: 500
            });
            newChannelTriggered.set(channel, false);
            newChannelTriggerTime.set(channel, 0);
            newChannelRearmTime.set(channel, 0);
        });
        
        channelData = newChannelData;
        frozenChannelData = newFrozenChannelData;
        heldChannelData = newHeldChannelData;
        channelModes = newChannelModes;
        axisSettings = newAxisSettings;
        triggerSettings = newTriggerSettings;
        channelTriggered = newChannelTriggered;
        channelTriggerTime = newChannelTriggerTime;
        channelRearmTime = newChannelRearmTime;
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
            isConnected = true;
        } catch (err) {
            console.error("Error starting data subscription:", err);
            error = "Failed to start data subscription";
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
        }
        
        channelData = new Map(channelData);
    }

    function processTriggerMode(
        channel: number,
        mode: ChannelPlotMode,
        fullData: DataPoint[],
        newChunk: DataPoint[]
    ) {
        const isSingleShot = mode === "trigger_single";
        const lastTimestamp = newChunk[newChunk.length - 1]?.timestamp ?? Date.now();
        const isChannelTriggered = channelTriggered.get(channel) || false;

        if (isChannelTriggered) {
            if (isSingleShot) return;

            const rearmAt = channelRearmTime.get(channel) || 0;
            if (lastTimestamp < rearmAt) {
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
                
                captureFrozenDataForChannel(channel, fullData, currentPoint.timestamp);

                return; 
            }

            previousPoint = currentPoint;
        }
    }

    function captureFrozenDataForChannel(channel: number, data: DataPoint[], triggerTime: number) {
        const startTime = triggerTime - (timeWindow * 1000);
        const endTime = triggerTime + (timeWindow * 1000); 

        const frozenData = data.filter(point => 
            point.timestamp >= startTime && point.timestamp <= endTime
        );
        
        frozenChannelData.set(channel, frozenData);
        frozenChannelData = new Map(frozenChannelData);
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

        if (nextMode === "hold") {
            heldChannelData.set(channel, [...(channelDisplayData.get(channel) || [])]);
            heldChannelData = new Map(heldChannelData);
        } else if (currentMode === "hold") {
            heldChannelData.set(channel, []);
            heldChannelData = new Map(heldChannelData);
        }

        if (!isTriggerMode(nextMode) || !isTriggerMode(currentMode)) {
            clearTriggerState(channel, true);
        }

        channelModes.set(channel, nextMode);
        channelModes = new Map(channelModes);
    }

    function updateAxisSettings(channel: number, updates: Partial<AxisSettings>) {
        const current = axisSettings.get(channel);
        if (!current) return;

        const updated = { ...current, ...updates };
        if (updated.yMax <= updated.yMin) {
            updated.yMax = updated.yMin + 0.001;
        }

        axisSettings.set(channel, updated);
        axisSettings = new Map(axisSettings);
    }

    function getPlotConfig(channel: number) {
        const mode = channelModes.get(channel) ?? "free_run";
        const liveData = channelDisplayData.get(channel) || [];
        const frozenData = frozenDisplayData.get(channel) || [];
        const heldData = heldChannelData.get(channel) || [];
        const isTriggered = channelTriggered.get(channel) || false;
        const triggerTime = channelTriggerTime.get(channel) || 0;

        if (mode === "hold") {
            return {
                mode: "continuous" as const,
                data: heldData.length > 0 ? heldData : liveData,
                frozenData: undefined,
                isTriggered: false,
                triggerTime: 0
            };
        }

        if (isTriggerMode(mode) && isTriggered) {
            return {
                mode: "frozen" as const,
                data: liveData,
                frozenData,
                isTriggered: true,
                triggerTime
            };
        }

        return {
            mode: "continuous" as const,
            data: liveData,
            frozenData: undefined,
            isTriggered: false,
            triggerTime: 0
        };
    }
    
    function goBack() {
        window.location.href = "/labjacks";
    }

    function toLocalInputValue(date: Date): string {
        const pad = (value: number) => value.toString().padStart(2, "0");
        return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
    }

    function toRfc3339(value: string): string {
        const date = new Date(value);
        if (isNaN(date.getTime())) {
            throw new Error("Invalid date/time value");
        }
        return date.toISOString();
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
    
    onDestroy(() => {
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


            <!-- Channel Sections: Combined Trigger Settings + Plots -->
            <div class="space-y-6">
                {#each labjackConfig.sensor_settings.channels_enabled as channel, index}
                    {@const channelTriggerSetting = triggerSettings.get(channel)}
                    {@const channelMode = channelModes.get(channel) ?? 'free_run'}
                    {@const channelAxis = axisSettings.get(channel)}
                    {@const plotConfig = getPlotConfig(channel)}
                    {@const isChannelTriggered = channelTriggered.get(channel) || false}
                    {@const channelTriggerTimeValue = channelTriggerTime.get(channel) || 0}
                    {@const channelRearmAt = channelRearmTime.get(channel) || 0}
                    
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
                                        {:else if channelMode === 'hold'}
                                            Held
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
                                <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-6 gap-4">
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
                                            <option value="hold">Hold</option>
                                        </select>
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
                                    <h4 class="text-md font-medium text-base-content mb-4">Trigger Settings</h4>
                                    <div class="grid grid-cols-1 md:grid-cols-4 gap-4">
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
                                                    const setting = triggerSettings.get(channel);
                                                    if (setting && e.target instanceof HTMLInputElement) {
                                                        triggerSettings.set(channel, {
                                                            ...setting,
                                                            holdoffMs: Math.max(0, parseInt(e.target.value, 10) || 0)
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
                                            {#if isChannelTriggered && channelMode === 'trigger_normal' && channelRearmAt > channelTriggerTimeValue}
                                                <span class="text-xs text-base-content/70 mt-2">
                                                    Holdoff active until {new Date(channelRearmAt).toLocaleTimeString()}
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
                                    timeWindow={timeWindow}
                                    isTriggered={plotConfig.isTriggered}
                                    triggerTime={plotConfig.triggerTime}
                                    mode={plotConfig.mode}
                                    frozenData={plotConfig.frozenData}
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
