<script lang="ts">
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
    
    interface Props {
        config: LabJackConfig;
        isAddingNew: boolean;
        existingLabJacks: Map<string, LabJackConfig>;
        onSave: (config: LabJackConfig) => void;
        onClose: () => void;
    }
    
    let { config, isAddingNew, existingLabJacks, onSave, onClose }: Props = $props();
    
    let formData = $state<LabJackConfig>({ ...config });
    let errors = $state<Record<string, string>>({});
    let saving = $state<boolean>(false);
    
    // Reactive validation for name and asset number
    $effect(() => {
        if (isAddingNew && formData.labjack_name.trim()) {
            const existingNames = Array.from(existingLabJacks.values()).map(lj => lj.labjack_name.toLowerCase());
            if (existingNames.includes(formData.labjack_name.toLowerCase())) {
                errors.labjack_name = "LabJack name already exists";
            } else if (errors.labjack_name === "LabJack name already exists") {
                delete errors.labjack_name;
            }
        }
    });
    
    $effect(() => {
        if (isAddingNew && formData.asset_number > 0) {
            const existingAssetNumbers = Array.from(existingLabJacks.values()).map(lj => lj.asset_number);
            if (existingAssetNumbers.includes(formData.asset_number)) {
                errors.asset_number = "Asset number already exists";
            } else if (errors.asset_number === "Asset number already exists") {
                delete errors.asset_number;
            }
        }
    });
    
    const dataFormats = ["voltage", "temperature", "pressure", "current", "resistance"];
    const measurementUnits = ["V", "°C", "PSI", "A", "Ω", "Pa", "kPa", "bar"];
    
    function validateForm(): boolean {
        errors = {};
        
        if (!formData.labjack_name.trim()) {
            errors.labjack_name = "LabJack name is required";
        } else if (isAddingNew) {
            // Check for duplicate labjack_name (case-insensitive)
            const existingNames = Array.from(existingLabJacks.values()).map(lj => lj.labjack_name.toLowerCase());
            if (existingNames.includes(formData.labjack_name.toLowerCase())) {
                errors.labjack_name = "LabJack name already exists";
            }
        }
        
        if (formData.asset_number <= 0) {
            errors.asset_number = "Asset number must be greater than 0";
        } else if (isAddingNew) {
            // Check for duplicate asset_number
            const existingAssetNumbers = Array.from(existingLabJacks.values()).map(lj => lj.asset_number);
            if (existingAssetNumbers.includes(formData.asset_number)) {
                errors.asset_number = "Asset number already exists";
            }
        }
        
        if (formData.max_channels <= 0 || formData.max_channels > 16) {
            errors.max_channels = "Max channels must be between 1 and 16";
        }
        
        if (formData.rotate_secs <= 0) {
            errors.rotate_secs = "Rotate seconds must be greater than 0";
        }
        
        if (!formData.nats_subject.trim()) {
            errors.nats_subject = "NATS subject is required";
        }
        
        if (!formData.nats_stream.trim()) {
            errors.nats_stream = "NATS stream is required";
        }
        
        if (formData.sensor_settings.scan_rate <= 0) {
            errors.scan_rate = "Scan rate must be greater than 0";
        }
        
        if (formData.sensor_settings.sampling_rate <= 0) {
            errors.sampling_rate = "Sampling rate must be greater than 0";
        }
        
        if (formData.sensor_settings.channels_enabled.length === 0) {
            errors.channels_enabled = "At least one channel must be enabled";
        }
        
        if (formData.sensor_settings.gains <= 0) {
            errors.gains = "Gains must be greater than 0";
        }
        
        if (formData.sensor_settings.data_formats.length !== formData.sensor_settings.channels_enabled.length) {
            errors.data_formats = "Data formats must be configured for all enabled channels";
        }
        
        if (formData.sensor_settings.measurement_units.length !== formData.sensor_settings.channels_enabled.length) {
            errors.measurement_units = "Measurement units must be configured for all enabled channels";
        }
        
        return Object.keys(errors).length === 0;
    }
    
    async function handleSave() {
        if (!validateForm()) {
            return;
        }
        
        saving = true;
        try {
            await onSave(formData);
        } finally {
            saving = false;
        }
    }
    
    function handleChannelToggle(channel: number) {
        const channels = [...formData.sensor_settings.channels_enabled];
        const index = channels.indexOf(channel);
        
        if (index > -1) {
            // Remove channel and corresponding data format/measurement unit
            channels.splice(index, 1);
            formData.sensor_settings.data_formats.splice(index, 1);
            formData.sensor_settings.measurement_units.splice(index, 1);
        } else {
            // Add channel and default data format/measurement unit
            channels.push(channel);
            formData.sensor_settings.data_formats.push("voltage");
            formData.sensor_settings.measurement_units.push("V");
        }
        
        formData.sensor_settings.channels_enabled = channels.sort((a, b) => a - b);
        
        // Reorder data formats and measurement units to match the sorted channels
        const sortedDataFormats = [];
        const sortedMeasurementUnits = [];
        
        for (const sortedChannel of formData.sensor_settings.channels_enabled) {
            const originalIndex = formData.sensor_settings.channels_enabled.indexOf(sortedChannel);
            sortedDataFormats.push(formData.sensor_settings.data_formats[originalIndex] || "voltage");
            sortedMeasurementUnits.push(formData.sensor_settings.measurement_units[originalIndex] || "V");
        }
        
        formData.sensor_settings.data_formats = sortedDataFormats;
        formData.sensor_settings.measurement_units = sortedMeasurementUnits;
    }
    
    
    function handleKeyPress(event: KeyboardEvent) {
        if (event.key === 'Escape') {
            onClose();
        }
    }
</script>

<svelte:window on:keydown={handleKeyPress} />

<!-- Modal Backdrop -->
<div class="fixed inset-0 bg-black/50 backdrop-blur-sm z-50 flex items-center justify-center p-4" onclick={onClose} role="button" tabindex="0" onkeydown={(e) => e.key === 'Escape' && onClose()}>
    <!-- Modal Content -->
    <div class="bg-gray-900 rounded-2xl shadow-2xl border border-white/20 w-full max-w-4xl max-h-[90vh] overflow-hidden" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="0" onkeydown={(e) => e.key === 'Escape' && onClose()}>
        <!-- Modal Header -->
        <div class="bg-white/10 backdrop-blur-lg border-b border-white/20 px-6 py-4">
            <div class="flex justify-between items-center">
                <div>
                    <h2 class="text-2xl font-bold text-white">
                        {isAddingNew ? 'Add New LabJack' : 'Edit LabJack Configuration'}
                    </h2>
                    <p class="text-gray-300 text-sm mt-1">
                        {isAddingNew ? 'Configure a new LabJack device' : 'Update LabJack settings and sensor configuration'}
                    </p>
                </div>
                <button
                    onclick={onClose}
                    class="p-2 hover:bg-white/10 rounded-lg transition-colors duration-200"
                    aria-label="Close modal"
                >
                    <svg class="w-6 h-6 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
                    </svg>
                </button>
            </div>
        </div>

        <!-- Modal Body -->
        <div class="p-6 overflow-y-auto max-h-[calc(90vh-140px)]">
            <form onsubmit={(e) => { e.preventDefault(); handleSave(); }} class="space-y-8">
                <!-- Basic Configuration -->
                <div>
                    <h3 class="text-lg font-semibold text-white mb-4">Basic Configuration</h3>
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
                        <!-- LabJack Name -->
                        <div>
                            <label for="labjack_name" class="block text-sm font-medium text-gray-300 mb-2">
                                LabJack Name *
                            </label>
                            <input
                                id="labjack_name"
                                type="text"
                                bind:value={formData.labjack_name}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                                placeholder="Enter LabJack name"
                            />
                            {#if errors.labjack_name}
                                <p class="mt-1 text-sm text-red-400">{errors.labjack_name}</p>
                            {/if}
                        </div>

                        <!-- Asset Number -->
                        <div>
                            <label for="asset_number" class="block text-sm font-medium text-gray-300 mb-2">
                                Asset Number *
                            </label>
                            <input
                                id="asset_number"
                                type="number"
                                bind:value={formData.asset_number}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                                placeholder="Enter asset number"
                            />
                            {#if errors.asset_number}
                                <p class="mt-1 text-sm text-red-400">{errors.asset_number}</p>
                            {/if}
                        </div>

                        <!-- Max Channels -->
                        <div>
                            <label for="max_channels" class="block text-sm font-medium text-gray-300 mb-2">
                                Max Channels *
                            </label>
                            <input
                                id="max_channels"
                                type="number"
                                min="1"
                                max="16"
                                bind:value={formData.max_channels}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                            />
                            {#if errors.max_channels}
                                <p class="mt-1 text-sm text-red-400">{errors.max_channels}</p>
                            {/if}
                        </div>

                        <!-- Rotate Seconds -->
                        <div>
                            <label for="rotate_secs" class="block text-sm font-medium text-gray-300 mb-2">
                                Rotate Interval (seconds) *
                            </label>
                            <input
                                id="rotate_secs"
                                type="number"
                                min="1"
                                bind:value={formData.rotate_secs}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                            />
                            {#if errors.rotate_secs}
                                <p class="mt-1 text-sm text-red-400">{errors.rotate_secs}</p>
                            {/if}
                        </div>

                        <!-- NATS Subject -->
                        <div>
                            <label for="nats_subject" class="block text-sm font-medium text-gray-300 mb-2">
                                NATS Subject *
                            </label>
                            <input
                                id="nats_subject"
                                type="text"
                                bind:value={formData.nats_subject}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                                placeholder="e.g., avenabox"
                            />
                            {#if errors.nats_subject}
                                <p class="mt-1 text-sm text-red-400">{errors.nats_subject}</p>
                            {/if}
                        </div>

                        <!-- NATS Stream -->
                        <div>
                            <label for="nats_stream" class="block text-sm font-medium text-gray-300 mb-2">
                                NATS Stream *
                            </label>
                            <input
                                id="nats_stream"
                                type="text"
                                bind:value={formData.nats_stream}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                                placeholder="e.g., labjacks"
                            />
                            {#if errors.nats_stream}
                                <p class="mt-1 text-sm text-red-400">{errors.nats_stream}</p>
                            {/if}
                        </div>
                    </div>
                </div>

                <!-- Sensor Settings -->
                <div>
                    <h3 class="text-lg font-semibold text-white mb-4">Sensor Settings</h3>
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
                        <!-- Scan Rate -->
                        <div>
                            <label for="scan_rate" class="block text-sm font-medium text-gray-300 mb-2">
                                Scan Rate (Hz) *
                            </label>
                            <input
                                id="scan_rate"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.scan_rate}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                            />
                            {#if errors.scan_rate}
                                <p class="mt-1 text-sm text-red-400">{errors.scan_rate}</p>
                            {/if}
                        </div>

                        <!-- Sampling Rate -->
                        <div>
                            <label for="sampling_rate" class="block text-sm font-medium text-gray-300 mb-2">
                                Sampling Rate (Hz) *
                            </label>
                            <input
                                id="sampling_rate"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.sampling_rate}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                            />
                            {#if errors.sampling_rate}
                                <p class="mt-1 text-sm text-red-400">{errors.sampling_rate}</p>
                            {/if}
                        </div>

                        <!-- Gains -->
                        <div>
                            <label for="gains" class="block text-sm font-medium text-gray-300 mb-2">
                                Gains *
                            </label>
                            <input
                                id="gains"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.gains}
                                class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                            />
                            {#if errors.gains}
                                <p class="mt-1 text-sm text-red-400">{errors.gains}</p>
                            {/if}
                        </div>

                        <!-- LabJack Status -->
                        <div>
                            <div class="block text-sm font-medium text-gray-300 mb-2">
                                LabJack Status
                            </div>
                            <div class="flex items-center space-x-4">
                                <label class="flex items-center">
                                    <input
                                        type="radio"
                                        bind:group={formData.sensor_settings.labjack_on_off}
                                        value={true}
                                        class="w-4 h-4 text-yellow-600 bg-gray-800 border-gray-600 focus:ring-yellow-500 focus:ring-2"
                                    />
                                    <span class="ml-2 text-white">Online</span>
                                </label>
                                <label class="flex items-center">
                                    <input
                                        type="radio"
                                        bind:group={formData.sensor_settings.labjack_on_off}
                                        value={false}
                                        class="w-4 h-4 text-yellow-600 bg-gray-800 border-gray-600 focus:ring-yellow-500 focus:ring-2"
                                    />
                                    <span class="ml-2 text-white">Offline</span>
                                </label>
                            </div>
                        </div>
                    </div>
                </div>

                <!-- Enabled Channels -->
                <div>
                    <h3 class="text-lg font-semibold text-white mb-4">Enabled Channels *</h3>
                    <div class="grid grid-cols-4 md:grid-cols-8 gap-3">
                        {#each Array.from({length: formData.max_channels}, (_, i) => i) as channel}
                            <label class="flex items-center justify-center p-3 bg-gray-800/50 border border-gray-600/50 rounded-lg cursor-pointer hover:bg-gray-700/50 transition-colors duration-200 {formData.sensor_settings.channels_enabled.includes(channel) ? 'border-yellow-500 bg-yellow-500/20' : ''}">
                                <input
                                    type="checkbox"
                                    checked={formData.sensor_settings.channels_enabled.includes(channel)}
                                    onchange={() => handleChannelToggle(channel)}
                                    class="sr-only"
                                />
                                <span class="text-white font-medium">{channel}</span>
                            </label>
                        {/each}
                    </div>
                    {#if errors.channels_enabled}
                        <p class="mt-2 text-sm text-red-400">{errors.channels_enabled}</p>
                    {/if}
                </div>

                <!-- Channel Configuration -->
                {#if formData.sensor_settings.channels_enabled.length > 0}
                    <div>
                        <h3 class="text-lg font-semibold text-white mb-4">Channel Configuration *</h3>
                        <div class="space-y-4">
                            {#each formData.sensor_settings.channels_enabled as channel, index}
                                <div class="bg-gray-800/30 rounded-lg p-4 border border-gray-600/30">
                                    <h4 class="text-md font-medium text-white mb-3">Channel {channel}</h4>
                                    <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                                        <!-- Data Format for this channel -->
                                        <div>
                                            <label for="data-format-{channel}" class="block text-sm font-medium text-gray-300 mb-2">
                                                Data Format
                                            </label>
                                            <select
                                                id="data-format-{channel}"
                                                bind:value={formData.sensor_settings.data_formats[index]}
                                                class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                                            >
                                                {#each dataFormats as format}
                                                    <option value={format} class="bg-gray-800 text-white">
                                                        {format.charAt(0).toUpperCase() + format.slice(1)}
                                                    </option>
                                                {/each}
                                            </select>
                                        </div>
                                        
                                        <!-- Measurement Unit for this channel -->
                                        <div>
                                            <label for="measurement-unit-{channel}" class="block text-sm font-medium text-gray-300 mb-2">
                                                Measurement Unit
                                            </label>
                                            <select
                                                id="measurement-unit-{channel}"
                                                bind:value={formData.sensor_settings.measurement_units[index]}
                                                class="w-full px-3 py-2 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                                            >
                                                {#each measurementUnits as unit}
                                                    <option value={unit} class="bg-gray-800 text-white">
                                                        {unit}
                                                    </option>
                                                {/each}
                                            </select>
                                        </div>
                                    </div>
                                </div>
                            {/each}
                        </div>
                        {#if errors.data_formats || errors.measurement_units}
                            <p class="mt-2 text-sm text-red-400">
                                {errors.data_formats || errors.measurement_units}
                            </p>
                        {/if}
                    </div>
                {/if}
            </form>
        </div>

        <!-- Modal Footer -->
        <div class="bg-white/10 backdrop-blur-lg border-t border-white/20 px-6 py-4">
            <div class="flex justify-end space-x-4">
                <button
                    type="button"
                    onclick={onClose}
                    class="px-6 py-3 bg-gray-700 hover:bg-gray-600 text-white font-semibold rounded-lg transition-colors duration-200"
                >
                    Cancel
                </button>
                <button
                    type="button"
                    onclick={handleSave}
                    disabled={saving}
                    class="px-6 py-3 bg-gradient-to-r from-yellow-500 to-yellow-600 hover:from-yellow-600 hover:to-yellow-700 disabled:from-gray-600 disabled:to-gray-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] disabled:transform-none disabled:cursor-not-allowed shadow-lg hover:shadow-xl flex items-center"
                >
                    {#if saving}
                        <svg class="animate-spin -ml-1 mr-3 h-5 w-5 text-white" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
                        </svg>
                        Saving...
                    {:else}
                        <svg class="w-5 h-5 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                        </svg>
                        {isAddingNew ? 'Add LabJack' : 'Save Changes'}
                    {/if}
                </button>
            </div>
        </div>
    </div>
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
