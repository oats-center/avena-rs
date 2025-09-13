<script lang="ts">
    import { onMount } from "svelte";
    import { connect, getKeys, getKeyValue, updateConfig, deleteKey } from "$lib/nats.svelte";
    import LabJackConfigModal from "$lib/components/LabJackConfigModal.svelte";
    
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
    
    let labjacks = $state<Map<string, LabJackConfig>>(new Map());
    let loading = $state<boolean>(true);
    let error = $state<string>("");
    let showModal = $state<boolean>(false);
    let editingConfig = $state<LabJackConfig | null>(null);
    let editingKey = $state<string>("");
    let isAddingNew = $state<boolean>(false);
    let natsService: any = null;
    
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
            
            // Get all keys from avenabox bucket
            const keys = await getKeys(natsService, "avenabox");
            console.log("Found keys:", keys);
            
            const newLabJacks = new Map<string, LabJackConfig>();
            
            // Load each LabJack configuration
            for (const key of keys) {
                try {
                    const configStr = await getKeyValue(natsService, "avenabox", key);
                    const config: LabJackConfig = JSON.parse(configStr);
                    newLabJacks.set(key, config);
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
                labjack_on_off: false
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

<div class="min-h-screen bg-gradient-to-br from-gray-900 via-gray-800 to-black">
    <!-- Header -->
    <div class="bg-white/10 backdrop-blur-lg border-b border-white/20">
        <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
            <div class="flex justify-between items-center py-4">
                <div class="flex items-center">
                    <div class="inline-flex items-center justify-center w-10 h-10 bg-gradient-to-br from-yellow-400 to-yellow-600 rounded-full mr-3">
                        <svg class="w-6 h-6 text-white" fill="currentColor" viewBox="0 0 20 20">
                            <path d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
                        </svg>
                    </div>
                    <div>
                        <h1 class="text-2xl font-bold text-white">Avena-OTR</h1>
                        <p class="text-gray-300 text-sm">LabJack Management Dashboard</p>
                    </div>
                </div>
                <button
                    onclick={logout}
                    class="px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-lg transition-colors duration-200 flex items-center"
                >
                    <svg class="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"/>
                    </svg>
                    Logout
                </button>
            </div>
        </div>
    </div>

    <!-- Main Content -->
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <!-- Page Header -->
        <div class="flex justify-between items-center mb-8">
            <div>
                <h2 class="text-3xl font-bold text-white mb-2">LabJack Configurations</h2>
                <p class="text-gray-300">Manage your LabJack devices and sensor settings</p>
            </div>
            <button
                onclick={handleAddNew}
                class="px-6 py-3 bg-gradient-to-r from-yellow-500 to-yellow-600 hover:from-yellow-600 hover:to-yellow-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl flex items-center"
            >
                <svg class="w-5 h-5 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6m0 0v6m0-6h6m-6 0H6"/>
                </svg>
                Add New LabJack
            </button>
        </div>

        <!-- Error Message -->
        {#if error}
            <div class="mb-6 p-4 bg-red-500/20 border border-red-500/30 rounded-lg">
                <div class="flex items-center justify-between">
                    <div class="flex items-center">
                        <svg class="w-5 h-5 text-red-400 mr-2" fill="currentColor" viewBox="0 0 20 20">
                            <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd"/>
                        </svg>
                        <span class="text-red-300">{error}</span>
                    </div>
                    <div class="flex space-x-2">
                        {#if error.includes("No NATS connection")}
                            <button
                                onclick={() => window.location.href = "/"}
                                class="px-3 py-1 bg-blue-600 hover:bg-blue-700 text-white text-sm rounded transition-colors duration-200"
                            >
                                Go to Login
                            </button>
                        {/if}
                        <button
                            onclick={loadLabJacks}
                            class="px-3 py-1 bg-red-600 hover:bg-red-700 text-white text-sm rounded transition-colors duration-200"
                        >
                            Retry
                        </button>
                    </div>
                </div>
            </div>
        {/if}

        <!-- Loading State -->
        {#if loading}
            <div class="flex justify-center items-center py-12">
                <div class="animate-spin rounded-full h-12 w-12 border-b-2 border-yellow-500"></div>
                <span class="ml-4 text-white text-lg">Loading LabJack configurations...</span>
            </div>
        {:else if labjacks.size === 0}
            <!-- Empty State -->
            <div class="text-center py-12">
                <div class="inline-flex items-center justify-center w-24 h-24 bg-gray-800/50 rounded-full mb-6">
                    <svg class="w-12 h-12 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"/>
                    </svg>
                </div>
                <h3 class="text-xl font-semibold text-white mb-2">No LabJack Configurations Found</h3>
                <p class="text-gray-400 mb-6">Get started by adding your first LabJack device configuration.</p>
                <button
                    onclick={handleAddNew}
                    class="px-6 py-3 bg-gradient-to-r from-yellow-500 to-yellow-600 hover:from-yellow-600 hover:to-yellow-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl"
                >
                    Add Your First LabJack
                </button>
            </div>
        {:else}
            <!-- LabJack Grid -->
            <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                {#each Array.from(labjacks.entries()) as [key, config]}
                    <div class="bg-white/10 backdrop-blur-lg rounded-xl p-6 border border-white/20 hover:border-yellow-500/50 transition-all duration-200 hover:shadow-xl">
                        <!-- LabJack Header -->
                        <div class="flex justify-between items-start mb-4">
                            <div>
                                <h3 class="text-xl font-semibold text-white mb-1">{config.labjack_name}</h3>
                                <p class="text-gray-400 text-sm">Asset #{config.asset_number}</p>
                            </div>
                            <div class="flex space-x-2">
                                <button
                                    onclick={() => handleEdit(key, config)}
                                    class="p-2 bg-blue-600/20 hover:bg-blue-600/30 text-blue-400 rounded-lg transition-colors duration-200"
                                    title="Edit Configuration"
                                    aria-label="Edit Configuration"
                                >
                                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"/>
                                    </svg>
                                </button>
                                <button
                                    onclick={() => handleDelete(key)}
                                    class="p-2 bg-red-600/20 hover:bg-red-600/30 text-red-400 rounded-lg transition-colors duration-200"
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
                        <div class="space-y-3">
                            <div class="flex justify-between">
                                <span class="text-gray-400">Max Channels:</span>
                                <span class="text-white">{config.max_channels}</span>
                            </div>
                            <div class="flex justify-between">
                                <span class="text-gray-400">Rotate Interval:</span>
                                <span class="text-white">{config.rotate_secs}s</span>
                            </div>
                            <div class="flex justify-between">
                                <span class="text-gray-400">NATS Subject:</span>
                                <span class="text-white font-mono text-sm">{config.nats_subject}</span>
                            </div>
                            <div class="flex justify-between">
                                <span class="text-gray-400">NATS Stream:</span>
                                <span class="text-white font-mono text-sm">{config.nats_stream}</span>
                            </div>
                        </div>

                        <!-- Sensor Settings -->
                        <div class="mt-4 pt-4 border-t border-white/10">
                            <h4 class="text-sm font-medium text-gray-300 mb-2">Sensor Settings</h4>
                            <div class="space-y-2">
                                <div class="flex justify-between text-sm">
                                    <span class="text-gray-400">Scan Rate:</span>
                                    <span class="text-white">{config.sensor_settings.scan_rate} Hz</span>
                                </div>
                                <div class="flex justify-between text-sm">
                                    <span class="text-gray-400">Sampling Rate:</span>
                                    <span class="text-white">{config.sensor_settings.sampling_rate} Hz</span>
                                </div>
                                <div class="text-sm">
                                    <span class="text-gray-400">Channels:</span>
                                    <div class="mt-1 space-y-1">
                                        {#each config.sensor_settings.channels_enabled as channel, index}
                                            <div class="flex justify-between text-xs">
                                                <span class="text-gray-300">Ch {channel}:</span>
                                                <span class="text-white">
                                                    {config.sensor_settings.data_formats[index] || 'N/A'} 
                                                    ({config.sensor_settings.measurement_units[index] || 'N/A'})
                                                </span>
                                            </div>
                                        {/each}
                                    </div>
                                </div>
                                <div class="flex justify-between text-sm">
                                    <span class="text-gray-400">Status:</span>
                                    <span class="text-white flex items-center">
                                        <div class="w-2 h-2 rounded-full mr-2 {config.sensor_settings.labjack_on_off ? 'bg-green-500' : 'bg-red-500'}"></div>
                                        {config.sensor_settings.labjack_on_off ? 'Online' : 'Offline'}
                                    </span>
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
            onSave={handleSave}
            onClose={handleModalClose}
        />
    {/if}
</div>

<style>
    /* Custom scrollbar for webkit browsers */
    ::-webkit-scrollbar {
        width: 8px;
    }
    
    ::-webkit-scrollbar-track {
        background: rgba(255, 255, 255, 0.1);
        border-radius: 4px;
    }
    
    ::-webkit-scrollbar-thumb {
        background: rgba(206, 184, 136, 0.5);
        border-radius: 4px;
    }
    
    ::-webkit-scrollbar-thumb:hover {
        background: rgba(206, 184, 136, 0.7);
    }
    
    /* Smooth transitions */
    * {
        transition-property: color, background-color, border-color, text-decoration-color, fill, stroke, opacity, box-shadow, transform, filter, backdrop-filter;
        transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1);
        transition-duration: 150ms;
    }
</style>
