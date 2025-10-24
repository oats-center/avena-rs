<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { page } from "$app/stores";
    import { connect, getKeyValue, getKeys, requestExport, downloadExport, type ExportRequestPayload } from "$lib/nats.svelte";
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
    
    interface DataPoint {
        timestamp: number;
        value: number;
    }
    
    let assetNumber = $state<number>(0);
    let labjackConfig = $state<LabJackConfig | null>(null);
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    let natsService: any = null;
    let subscriptions: any[] = [];
    let channelData = $state<Map<number, DataPoint[]>>(new Map());
    let frozenChannelData = $state<Map<number, DataPoint[]>>(new Map());
    let isConnected = $state<boolean>(false);
    let flatBufferParser = new FlatBufferParser();
    let triggerSettings = $state<Map<number, {
        enabled: boolean;
        type: 'rising' | 'falling';
        threshold: number;
    }>>(new Map());
    let channelTriggered = $state<Map<number, boolean>>(new Map());
    let channelTriggerTime = $state<Map<number, number>>(new Map());
    let timeWindow = $state<number>(5); // seconds
    let maxDataPoints = $state<number>(10000);
    let showExportModal = $state<boolean>(false);
    let exportFormat = $state<"csv" | "parquet">("csv");
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
            const keys = await getKeys(natsService, "avenabox");
            let foundConfig: LabJackConfig | null = null;
            
            for (const key of keys) {
                try {
                    const configStr = await getKeyValue(natsService, "avenabox", key);
                    const config: LabJackConfig = JSON.parse(configStr);
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
        const newTriggerSettings = new Map<number, {
            enabled: boolean;
            type: 'rising' | 'falling';
            threshold: number;
        }>();
        const newChannelTriggered = new Map<number, boolean>();
        const newChannelTriggerTime = new Map<number, number>();
        
        labjackConfig.sensor_settings.channels_enabled.forEach(channel => {
            newChannelData.set(channel, []);
            newFrozenChannelData.set(channel, []);
            newTriggerSettings.set(channel, {
                enabled: false,
                type: 'rising',
                threshold: 0
            });
            newChannelTriggered.set(channel, false);
            newChannelTriggerTime.set(channel, 0);
        });
        
        channelData = newChannelData;
        frozenChannelData = newFrozenChannelData;
        triggerSettings = newTriggerSettings;
        channelTriggered = newChannelTriggered;
        channelTriggerTime = newChannelTriggerTime;
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
                            
                            const sampleTimestamps = calculateSampleTimestamps(
                                scanData.timestamp, 
                                scanData.values, 
                                labjackConfig.sensor_settings.sampling_rate
                            );
                            
                            // **CHANGE: Create a chunk of new data points**
                            const newPoints: DataPoint[] = [];
                            for (let i = 0; i < scanData.values.length; i++) {
                                const value = scanData.values[i];
                                const timestamp = sampleTimestamps[i];
                                
                                if (typeof value === 'number' && typeof timestamp === 'number' && !isNaN(value) && isFinite(value) && Math.abs(value) < 10) {
                                    newPoints.push({ timestamp, value });
                                }
                            }

                            // **CHANGE: Process the entire chunk at once**
                            if (newPoints.length > 0) {
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
        
        // **Efficiently add the new chunk of data**
        let newData = currentData.concat(chunk);
        
        // **Trim the buffer only if it's oversized**
        if (newData.length > maxDataPoints) {
            newData = newData.slice(newData.length - maxDataPoints);
        }
        
        // **Update the map with the new array**
        channelData.set(channel, newData);
        
        // Check for trigger conditions
        const channelTriggerSetting = triggerSettings.get(channel);
        const isChannelTriggered = channelTriggered.get(channel) || false;
        
        if (channelTriggerSetting?.enabled && !isChannelTriggered) {
            // We pass the new chunk to the trigger check function
            checkTriggerCondition(channel, newData, chunk);
        }
        
        // If channel is triggered, add the new points to the frozen data
        if (isChannelTriggered) {
            const channelTriggerTimeValue = channelTriggerTime.get(channel) || 0;
            if (channelTriggerTimeValue > 0) {
                const timeSinceTrigger = (Date.now() - channelTriggerTimeValue) / 1000;
                if (timeSinceTrigger <= timeWindow) {
                    // Filter the chunk for points within the frozen window and add them
                    const relevantPoints = chunk.filter(p => p.timestamp >= (channelTriggerTimeValue - timeWindow * 1000) && p.timestamp <= (channelTriggerTimeValue + timeWindow * 1000));
                    if(relevantPoints.length > 0) {
                        updateFrozenDataChunk(channel, relevantPoints);
                    }
                }
            }
        }
        
        // **Trigger Svelte reactivity ONCE for the entire batch**
        channelData = new Map(channelData);
    }

    // Helper to update frozen data in chunks
    function updateFrozenDataChunk(channel: number, chunk: DataPoint[]) {
        const currentFrozenData = frozenChannelData.get(channel) || [];
        const updatedFrozenData = currentFrozenData.concat(chunk);
        frozenChannelData.set(channel, updatedFrozenData);
        frozenChannelData = new Map(frozenChannelData);
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
                channelTriggered.set(channel, true);
                channelTriggerTime.set(channel, currentPoint.timestamp);
                channelTriggered = new Map(channelTriggered);
                channelTriggerTime = new Map(channelTriggerTime);
                
                // Capture the full frozen data based on the full dataset
                captureFrozenDataForChannel(channel, fullData);

                // Once a trigger is found in the chunk, we can stop checking
                return; 
            }

            // Move to the next point
            previousPoint = currentPoint;
        }
    }

    // Modify captureFrozenDataForChannel to accept the full data array
    function captureFrozenDataForChannel(channel: number, data: DataPoint[]) {
        const triggerTime = channelTriggerTime.get(channel) || 0;
        const startTime = triggerTime - (timeWindow * 1000);
        const endTime = triggerTime + (timeWindow * 1000); 

        const frozenData = data.filter(point => 
            point.timestamp >= startTime && point.timestamp <= endTime
        );
        
        frozenChannelData.set(channel, frozenData);
        frozenChannelData = new Map(frozenChannelData);
    }
    
    
    function resetChannelTrigger(channel: number) {
        // Reset trigger for a specific channel
        channelTriggered.set(channel, false);
        channelTriggerTime.set(channel, 0);
        frozenChannelData.set(channel, []);
        
        channelTriggered = new Map(channelTriggered); // Trigger reactivity
        channelTriggerTime = new Map(channelTriggerTime); // Trigger reactivity
        frozenChannelData = new Map(frozenChannelData); // Trigger reactivity
        
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
        exportFormat = "csv";
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
        if (!labjackConfig || !natsService) {
            exportError = "NATS connection is not ready";
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
                format: exportFormat,
                download_name: labjackConfig.labjack_name
                    ? `${labjackConfig.labjack_name.replace(/\s+/g, "_")}.${exportFormat}`
                    : undefined,
            };

            const resp = await requestExport(natsService, payload);
            if (resp.status === "empty") {
                exportError = resp.missing_channels && resp.missing_channels.length > 0
                    ? `No data found. Missing channels: ${resp.missing_channels.map((ch) => ch.toString().padStart(2, "0")).join(", ")}`
                    : "No data found for the selected window";
                exporting = false;
                return;
            }

            if (resp.status !== "ok") {
                throw new Error(resp.error || "Export failed");
            }

            if (resp.missing_channels && resp.missing_channels.length > 0) {
                const formatted = resp.missing_channels
                    .map((ch) => ch.toString().padStart(2, "0"))
                    .join(", ");
                exportWarning = `No samples found for channels: ${formatted}. Continuing with remaining channels.`;
            }

            const result = await downloadExport(natsService, resp, (received, total) => {
                exportProgress = received;
                exportTotal = total ?? null;
            });

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
                    {@const isChannelTriggered = channelTriggered.get(channel) || false}
                    {@const channelTriggerTimeValue = channelTriggerTime.get(channel) || 0}
                    
                    <!-- Combined Channel Section -->
                    <div class="card bg-base-100 shadow-xl border border-base-200">
                        <div class="card-body">
                            <!-- Channel Header -->
                            <div class="flex items-center justify-between mb-6">
                                <h3 class="card-title text-base-content">Channel {channel}</h3>
                                <div class="flex items-center space-x-4">
                                    <div class="badge badge-outline badge-sm">
                                        {labjackConfig.sensor_settings.data_formats[index]} 
                                        ({labjackConfig.sensor_settings.measurement_units[index]})
                                    </div>
                                    <div class="flex items-center space-x-3">
                                        {#if isChannelTriggered}
                                            <div class="flex items-center">
                                                <div class="w-2 h-2 rounded-full bg-success mr-2"></div>
                                                <span class="text-success text-sm">
                                                    Triggered at {new Date(channelTriggerTimeValue).toLocaleTimeString()}
                                                </span>
                                            </div>
                                            <button
                                                onclick={() => resetChannelTrigger(channel)}
                                                class="btn btn-error btn-sm"
                                                title="Reset Channel Trigger"
                                            >
                                                Reset
                                            </button>
                                        {:else}
                                            <div class="flex items-center">
                                                <div class="w-2 h-2 rounded-full bg-base-content/30 mr-2"></div>
                                                <span class="text-base-content/60 text-sm">Waiting for trigger...</span>
                                            </div>
                                        {/if}
                                    </div>
                                </div>
                            </div>
                        
                            <!-- Trigger Settings -->
                            <div class="mb-6 p-4 bg-base-200 rounded-lg">
                                <h4 class="text-md font-medium text-base-content mb-4">Trigger Settings</h4>
                                <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
                                    <!-- Trigger Enable -->
                                    <div class="form-control">
                                        <label class="label cursor-pointer">
                                            <span class="label-text">Enable Trigger</span>
                                            <input
                                                id="enable-trigger-{channel}"
                                                type="checkbox"
                                                checked={channelTriggerSetting?.enabled || false}
                                                onchange={(e) => {
                                                    const setting = triggerSettings.get(channel);
                                                    if (setting && e.target instanceof HTMLInputElement) {
                                                        setting.enabled = e.target.checked;
                                                        triggerSettings = new Map(triggerSettings);
                                                    }
                                                }}
                                                class="checkbox checkbox-warning"
                                            />
                                        </label>
                                    </div>
                                    
                                    <!-- Trigger Type -->
                                    <div class="form-control">
                                        <label class="label" for="trigger-type-{channel}">
                                            <span class="label-text">Trigger Type</span>
                                        </label>
                                        <select
                                            id="trigger-type-{channel}"
                                            value={channelTriggerSetting?.type || 'rising'}
                                            onchange={(e) => {
                                                const setting = triggerSettings.get(channel);
                                                if (setting && e.target instanceof HTMLSelectElement) {
                                                    setting.type = e.target.value as 'rising' | 'falling';
                                                    triggerSettings = new Map(triggerSettings);
                                                }
                                            }}
                                            class="select select-bordered select-warning"
                                        >
                                            <option value="rising">Rising Edge</option>
                                            <option value="falling">Falling Edge</option>
                                        </select>
                                    </div>
                                    
                                    <!-- Trigger Threshold -->
                                    <div class="form-control">
                                        <label class="label" for="trigger-threshold-{channel}">
                                            <span class="label-text">Threshold (V)</span>
                                        </label>
                                        <input
                                            id="trigger-threshold-{channel}"
                                            type="number"
                                            step="0.1"
                                            value={channelTriggerSetting?.threshold || 0}
                                            onchange={(e) => {
                                                const setting = triggerSettings.get(channel);
                                                if (setting && e.target instanceof HTMLInputElement) {
                                                    setting.threshold = parseFloat(e.target.value) || 0;
                                                    triggerSettings = new Map(triggerSettings);
                                                }
                                            }}
                                            class="input input-bordered input-warning"
                                        />
                                    </div>
                                </div>
                            </div>
                        
                            <!-- Data Plots -->
                            <div>
                                <h4 class="text-md font-medium text-base-content mb-4">Data Plots</h4>
                                <!-- Side by side plots -->
                                <div class="grid grid-cols-1 xl:grid-cols-2 gap-8">
                                    <!-- Continuous Plot (Left) -->
                                    <div>
                                        <div class="flex items-center mb-4">
                                            <div class="w-2 h-2 rounded-full bg-primary mr-2"></div>
                                            <h5 class="text-sm font-medium text-base-content">Live Data</h5>
                                        </div>
                                        <RealTimePlot
                                            data={channelDisplayData.get(channel) || []}
                                            unit={labjackConfig.sensor_settings.measurement_units[index]}
                                            timeWindow={timeWindow}
                                            isTriggered={channelTriggered.get(channel) || false}
                                            triggerTime={channelTriggerTime.get(channel) || 0}
                                            mode="continuous"
                                        />
                                    </div>
                                    
                                    <!-- Frozen Plot (Right) -->
                                    <div>
                                        <div class="flex items-center mb-4">
                                            <div class="w-2 h-2 rounded-full bg-warning mr-2"></div>
                                            <h5 class="text-sm font-medium text-base-content">
                                                {(channelTriggered.get(channel) || false) ? 'Triggered Data' : 'Waiting for Trigger'}
                                            </h5>
                                        </div>
                                        <RealTimePlot
                                            data={channelDisplayData.get(channel) || []}
                                            unit={labjackConfig.sensor_settings.measurement_units[index]}
                                            timeWindow={timeWindow}
                                            isTriggered={channelTriggered.get(channel) || false}
                                            triggerTime={channelTriggerTime.get(channel) || 0}
                                            mode="frozen"
                                            frozenData={frozenDisplayData.get(channel) || []}
                                        />
                                    </div>
                                </div>
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

                            <div class="form-control">
                                <label class="label" for="export-format">
                                    <span class="label-text">File Format</span>
                                </label>
                                <select
                                    id="export-format"
                                    class="select select-bordered"
                                    bind:value={exportFormat}
                                    disabled={exporting}
                                >
                                    <option value="csv">CSV</option>
                                    <option value="parquet">Parquet</option>
                                </select>
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
