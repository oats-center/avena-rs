<script lang="ts">
    import { onMount } from "svelte";
    import { connect, getKeys, getKeyValue, updateConfig, deleteKey } from "$lib/nats.svelte";
    import { normalizeCalibration, type CalibrationSpec } from "$lib/calibration";
    import LabJackConfigModal from "$lib/components/LabJackConfigModal.svelte";
    
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
    
    let labjacks = $state<Map<string, LabJackConfig>>(new Map());
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    let showModal = $state<boolean>(false);
    let editingConfig = $state<LabJackConfig | null>(null);
    let editingKey = $state<string>("");
    let isAddingNew = $state<boolean>(false);
    let natsService: any = null;
    let availableCalibrations = $state<Map<string, CalibrationSpec>>(new Map());
    
    onMount(async () => {
        await loadLabJacks();
    });
    
    async function loadLabJacks() {
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
            
            await loadCalibrations();

            // Get all LabJack config keys from avenabox bucket
            const keys = await getKeys(natsService, "avenabox", "labjackd.config.*");
            console.log("Found keys:", keys);
            
            const newLabJacks = new Map<string, LabJackConfig>();
            
            // Load each LabJack configuration
            for (const key of keys) {
                try {
                    const configStr = await getKeyValue(natsService, "avenabox", key);
                    const config = normalizeLabJackConfig(JSON.parse(configStr));
                    if (config) newLabJacks.set(key, config);
                } catch (err) {
                    console.error(`Failed to parse config for key ${key}:`, err);
                }
            }
            
            labjacks = newLabJacks;
        } catch (err) {
            console.error("Error loading LabJacks:", err);
            error = "Failed to load LabJack configurations";
        } finally {
            loading = false;
        }
    }

    async function loadCalibrations() {
        try {
            const keys = await getKeys(natsService, "avenabox", "calibration.*");
            const presets = new Map<string, CalibrationSpec>();
            for (const key of keys) {
                try {
                    const raw = await getKeyValue(natsService, "avenabox", key);
                    const parsed = normalizeCalibration(JSON.parse(raw));
                    const id = parsed.id ?? key.replace(/^calibration\./, "");
                    presets.set(id, { ...parsed, id });
                } catch (err) {
                    console.error(`Failed to parse calibration ${key}:`, err);
                }
            }
            availableCalibrations = presets;
        } catch (err) {
            console.error("Error loading calibrations:", err);
        }
    }
    
    function handleEdit(key: string, config: LabJackConfig) {
        editingKey = key;
        editingConfig = { ...config };
        isAddingNew = false;
        showModal = true;
    }
    
    function handleAddNew() {
        editingKey = "";
        editingConfig = {
            labjack_name: "",
            asset_number: 0,
            max_channels: 8,
            nats_subject: "avenabox",
            nats_stream: "labjacks",
            rotate_secs: 60,
            sensor_settings: {
                scan_rate: 200,
                sampling_rate: 1000,
                channels_enabled: [0, 1, 2],
                gains: 1,
                data_formats: ["voltage", "temperature", "pressure"],
                measurement_units: ["V", "°C", "PSI"],
                labjack_on_off: false,
                calibrations: {}
            }
        };
        isAddingNew = true;
        showModal = true;
    }
    
    async function handleDelete(key: string) {
        if (!confirm(`Are you sure you want to delete LabJack "${key}"?`)) {
            return;
        }
        
        try {
            const serverName = sessionStorage.getItem("serverName");
            const credentialsContent = sessionStorage.getItem("credentialsContent");
            
            if (!serverName || !credentialsContent) {
                error = "No NATS connection found";
                return;
            }
            
            // Delete the key using the delete operation
            const success = await deleteKey(serverName, credentialsContent, "avenabox", key);
            
            if (success) {
                // Remove from local state
                const newLabJacks = new Map(labjacks);
                newLabJacks.delete(key);
                labjacks = newLabJacks;
            } else {
                error = "Failed to delete LabJack configuration";
            }
        } catch (err) {
            console.error("Error deleting LabJack:", err);
            error = "Failed to delete LabJack configuration";
        }
    }
    
    async function handleSave(config: LabJackConfig) {
        try {
            const serverName = sessionStorage.getItem("serverName");
            const credentialsContent = sessionStorage.getItem("credentialsContent");
            
            if (!serverName || !credentialsContent) {
                error = "No NATS connection found";
                return;
            }
            
            const key = isAddingNew ? `labjackd.config.${config.labjack_name.toLowerCase()}` : editingKey;
            const success = await updateConfig(serverName, credentialsContent, "avenabox", key, config);
            
            if (success) {
                // Update local state
                const newLabJacks = new Map(labjacks);
                newLabJacks.set(key, config);
                labjacks = newLabJacks;
                
                showModal = false;
                editingConfig = null;
                editingKey = "";
                isAddingNew = false;
            } else {
                error = "Failed to save LabJack configuration";
            }
        } catch (err) {
            console.error("Error saving LabJack:", err);
            error = "Failed to save LabJack configuration";
        }
    }

    async function handleSaveCalibration(spec: CalibrationSpec): Promise<boolean> {
        try {
            const serverName = sessionStorage.getItem("serverName");
            const credentialsContent = sessionStorage.getItem("credentialsContent");
            if (!serverName || !credentialsContent) {
                error = "No NATS connection found";
                return false;
            }
            const sanitizedId = sanitizeCalibrationId(spec.id ?? "");
            if (!sanitizedId) {
                return false;
            }
            const key = `calibration.${sanitizedId}`;
            const normalized = { ...spec, id: sanitizedId };
            const success = await updateConfig(serverName, credentialsContent, "avenabox", key, normalized);
            if (success) {
                const updated = new Map(availableCalibrations);
                updated.set(sanitizedId, normalized);
                availableCalibrations = updated;
            }
            return success;
        } catch (err) {
            console.error("Error saving calibration:", err);
            return false;
        }
    }

    function sanitizeCalibrationId(raw: string): string {
        return raw
            .trim()
            .toLowerCase()
            .replace(/\s+/g, "-")
            .replace(/[^a-z0-9._-]/g, "");
    }
    
    function handleModalClose() {
        showModal = false;
        editingConfig = null;
        editingKey = "";
        isAddingNew = false;
    }
    
    function logout() {
        sessionStorage.removeItem("serverName");
        sessionStorage.removeItem("credentialsContent");
        window.location.href = "/";
    }
</script>

<svelte:head>
    <title>LabJack Management - Avena-OTR</title>
</svelte:head>

<div class="min-h-screen bg-base-300">
    <!-- Header -->
    <div class="navbar bg-base-100 shadow-xl border-b border-base-200">
        <div class="flex-1">
            <div class="flex items-center">
                <div class="avatar placeholder mr-4">
                    <div class="text-primary-content h-12 w-12">
                        <div class="flex items-center justify-center">
                            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-12 h-12">
                                <path stroke-linecap="round" stroke-linejoin="round" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>
                            </svg>
                        </div>
                    </div>
                </div>
                <div>
                    <h1 class="text-2xl font-bold text-base-content">Avena-OTR</h1>
                    <p class="text-base-content/70 text-sm">LabJack Management Dashboard</p>
                </div>
            </div>
        </div>
        <div class="flex-none">
            <button
                onclick={logout}
                class="btn btn-error btn-sm"
            >
                <svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"/>
                </svg>
                Logout
            </button>
        </div>
    </div>

    <!-- Main Content -->
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <!-- Page Header -->
        <div class="flex justify-between items-center mb-8">
            <div>
                <h2 class="text-3xl font-bold mb-2">LabJack Configurations</h2>
                <p class="text-base-content/70">Manage your LabJack devices and sensor settings</p>
            </div>
            <button
                onclick={handleAddNew}
                class="btn btn-warning"
            >
                <svg class="w-5 h-5 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6m0 0v6m0-6h6m-6 0H6"/>
                </svg>
                Add New LabJack
            </button>
        </div>

        <!-- Error Message -->
        {#if error}
            <div class="alert alert-error mb-6">
                <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                    <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd"/>
                </svg>
                <span>{error}</span>
                <div class="flex space-x-2">
                    {#if error.includes("No NATS connection")}
                        <button
                            onclick={() => window.location.href = "/"}
                            class="btn btn-sm btn-primary"
                        >
                            Go to Login
                        </button>
                    {/if}
                    <button
                        onclick={loadLabJacks}
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
                <span class="ml-4 text-lg">Loading LabJack configurations...</span>
            </div>
        {:else if labjacks.size === 0}
            <!-- Empty State -->
            <div class="text-center py-12">
                <div class="avatar placeholder mb-6">
                    <div class="bg-base-200 text-base-content rounded-full w-24">
                        <svg class="w-12 h-12" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"/>
                        </svg>
                    </div>
                </div>
                <h3 class="text-xl font-semibold mb-2">No LabJack Configurations Found</h3>
                <p class="text-base-content/70 mb-6">Get started by adding your first LabJack device configuration.</p>
                <button
                    onclick={handleAddNew}
                    class="btn btn-warning"
                >
                    Add Your First LabJack
                </button>
            </div>
        {:else}
            <!-- LabJack Grid -->
            <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                {#each Array.from(labjacks.entries()) as [key, config]}
                    <div class="card bg-base-100 shadow-xl hover:shadow-2xl transition-all duration-300 hover:-translate-y-1 border border-base-200">
                        <div class="card-body p-6">
                            <!-- LabJack Header -->
                            <div class="flex justify-between items-start mb-6">
                                <div>
                                    <h3 class="card-title text-xl text-base-content">{config.labjack_name}</h3>
                                    <p class="text-base-content/70 text-sm mt-1">Asset #{config.asset_number}</p>
                                </div>
                                <div class="flex space-x-1">
                                    <button
                                        onclick={() => window.location.href = `/labjacks/plots/${config.asset_number}`}
                                        class="btn btn-sm btn-success btn-circle"
                                        title="View Real-time Plots"
                                        aria-label="View Real-time Plots"
                                    >
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>
                                        </svg>
                                    </button>
                                    <button
                                        onclick={() => handleEdit(key, config)}
                                        class="btn btn-sm btn-primary btn-circle"
                                        title="Edit Configuration"
                                        aria-label="Edit Configuration"
                                    >
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/>
                                        </svg>
                                    </button>
                                    <button
                                        onclick={() => handleDelete(key)}
                                        class="btn btn-sm btn-error btn-circle"
                                        title="Delete Configuration"
                                        aria-label="Delete Configuration"
                                    >
                                        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
                                        </svg>
                                    </button>
                                </div>
                            </div>

                            <!-- Configuration Details -->
                            <div class="space-y-3 mb-4">
                                <div class="flex justify-between items-center">
                                    <span class="text-base-content/70 text-sm">Max Channels:</span>
                                    <span class="badge badge-primary badge-sm">{config.max_channels}</span>
                                </div>
                                <div class="flex justify-between items-center">
                                    <span class="text-base-content/70 text-sm">Rotate Interval:</span>
                                    <span class="badge badge-secondary badge-sm">{config.rotate_secs}s</span>
                                </div>
                                <div class="flex justify-between items-center">
                                    <span class="text-base-content/70 text-sm">NATS Subject:</span>
                                    <span class="badge badge-accent badge-sm font-mono">{config.nats_subject}</span>
                                </div>
                                <div class="flex justify-between items-center">
                                    <span class="text-base-content/70 text-sm">NATS Stream:</span>
                                    <span class="badge badge-accent badge-sm font-mono">{config.nats_stream}</span>
                                </div>
                            </div>

                            <!-- Sensor Settings -->
                            <div class="divider my-4"></div>
                            <div>
                                <h4 class="text-sm font-semibold mb-3 text-base-content">Sensor Settings</h4>
                                <div class="space-y-2">
                                    <div class="flex justify-between items-center text-sm">
                                        <span class="text-base-content/70">Scan Rate:</span>
                                        <span class="badge badge-info badge-sm">{config.sensor_settings.scan_rate} Hz</span>
                                    </div>
                                    <div class="flex justify-between items-center text-sm">
                                        <span class="text-base-content/70">Sampling Rate:</span>
                                        <span class="badge badge-info badge-sm">{config.sensor_settings.sampling_rate} Hz</span>
                                    </div>
                                    <div class="text-sm">
                                        <span class="text-base-content/70 mb-2 block">Channels:</span>
                                        <div class="flex flex-wrap gap-1">
                                            {#each config.sensor_settings.channels_enabled as channel, index}
                                                <span class="badge badge-outline badge-sm">
                                                    Ch {channel}: {config.sensor_settings.data_formats[index] || 'N/A'} 
                                                    ({config.sensor_settings.measurement_units[index] || 'N/A'})
                                                </span>
                                            {/each}
                                        </div>
                                    </div>
                                    <div class="flex justify-between items-center text-sm mt-3">
                                        <span class="text-base-content/70">Status:</span>
                                        <span class="badge {config.sensor_settings.labjack_on_off ? 'badge-success' : 'badge-error'} badge-sm">
                                            <div class="w-2 h-2 rounded-full mr-1 {config.sensor_settings.labjack_on_off ? 'bg-success-content' : 'bg-error-content'}"></div>
                                            {config.sensor_settings.labjack_on_off ? 'Online' : 'Offline'}
                                        </span>
                                    </div>
                                </div>
                            </div>
                        </div>
                    </div>
                {/each}
            </div>
        {/if}
    </div>

    <!-- Configuration Modal -->
    {#if showModal && editingConfig}
        <LabJackConfigModal
            config={editingConfig}
            isAddingNew={isAddingNew}
            existingLabJacks={labjacks}
            availableCalibrations={availableCalibrations}
            onSaveCalibration={handleSaveCalibration}
            onSave={handleSave}
            onClose={handleModalClose}
        />
    {/if}
</div>
