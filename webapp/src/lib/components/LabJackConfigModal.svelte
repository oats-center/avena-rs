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

<!-- Modal -->
<div class="modal modal-open" onclick={onClose} role="button" tabindex="0" onkeydown={(e) => e.key === 'Escape' && onClose()}>
    <div class="modal-box w-11/12 max-w-4xl h-[90vh] flex flex-col bg-base-100 shadow-2xl border border-base-200" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="0" onkeydown={(e) => e.key === 'Escape' && onClose()}>
        <!-- Modal Header -->
        <div class="flex justify-between items-center mb-6 pb-4 border-b border-base-200 flex-shrink-0">
            <div>
                <h2 class="text-2xl font-bold text-base-content">
                    {isAddingNew ? 'Add New LabJack' : 'Edit LabJack Configuration'}
                </h2>
                <p class="text-base-content/70 text-sm mt-1">
                    {isAddingNew ? 'Configure a new LabJack device' : 'Update LabJack settings and sensor configuration'}
                </p>
            </div>
            <button
                onclick={onClose}
                class="btn btn-sm btn-circle btn-ghost hover:bg-base-200"
                aria-label="Close modal"
            >
                <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
                </svg>
            </button>
        </div>

        <!-- Modal Body -->
        <div class="flex-1 overflow-y-auto">
            <form onsubmit={(e) => { e.preventDefault(); handleSave(); }} class="space-y-8">
                <!-- Basic Configuration -->
                <div>
                    <h3 class="text-lg font-semibold mb-6 text-base-content">Basic Configuration</h3>
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
                        <!-- LabJack Name -->
                        <div class="form-control">
                            <label class="label" for="labjack_name">
                                <span class="label-text font-medium">LabJack Name *</span>
                            </label>
                            <input
                                id="labjack_name"
                                type="text"
                                bind:value={formData.labjack_name}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="Enter LabJack name"
                            />
                            {#if errors.labjack_name}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.labjack_name}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- Asset Number -->
                        <div class="form-control">
                            <label class="label" for="asset_number">
                                <span class="label-text font-medium">Asset Number *</span>
                            </label>
                            <input
                                id="asset_number"
                                type="number"
                                bind:value={formData.asset_number}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="Enter asset number"
                            />
                            {#if errors.asset_number}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.asset_number}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- Max Channels -->
                        <div class="form-control">
                            <label class="label" for="max_channels">
                                <span class="label-text font-medium">Max Channels *</span>
                            </label>
                            <input
                                id="max_channels"
                                type="number"
                                min="1"
                                max="16"
                                bind:value={formData.max_channels}
                                class="input input-bordered w-full focus:input-primary"
                            />
                            {#if errors.max_channels}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.max_channels}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- Rotate Seconds -->
                        <div class="form-control">
                            <label class="label" for="rotate_secs">
                                <span class="label-text font-medium">Rotate Interval (seconds) *</span>
                            </label>
                            <input
                                id="rotate_secs"
                                type="number"
                                min="1"
                                bind:value={formData.rotate_secs}
                                class="input input-bordered w-full focus:input-primary"
                            />
                            {#if errors.rotate_secs}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.rotate_secs}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- NATS Subject -->
                        <div class="form-control">
                            <label class="label" for="nats_subject">
                                <span class="label-text font-medium">NATS Subject *</span>
                            </label>
                            <input
                                id="nats_subject"
                                type="text"
                                bind:value={formData.nats_subject}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="e.g., avenabox"
                            />
                            {#if errors.nats_subject}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.nats_subject}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- NATS Stream -->
                        <div class="form-control">
                            <label class="label" for="nats_stream">
                                <span class="label-text font-medium">NATS Stream *</span>
                            </label>
                            <input
                                id="nats_stream"
                                type="text"
                                bind:value={formData.nats_stream}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="e.g., labjacks"
                            />
                            {#if errors.nats_stream}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.nats_stream}</span>
                                </div>
                            {/if}
                        </div>
                    </div>
                </div>

                <!-- Sensor Settings -->
                <div>
                    <h3 class="text-lg font-semibold mb-6 text-base-content">Sensor Settings</h3>
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
                        <!-- Scan Rate -->
                        <div class="form-control">
                            <label class="label" for="scan_rate">
                                <span class="label-text font-medium">Scan Rate (Hz) *</span>
                            </label>
                            <input
                                id="scan_rate"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.scan_rate}
                                class="input input-bordered w-full focus:input-primary"
                            />
                            {#if errors.scan_rate}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.scan_rate}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- Sampling Rate -->
                        <div class="form-control">
                            <label class="label" for="sampling_rate">
                                <span class="label-text font-medium">Sampling Rate (Hz) *</span>
                            </label>
                            <input
                                id="sampling_rate"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.sampling_rate}
                                class="input input-bordered w-full focus:input-primary"
                            />
                            {#if errors.sampling_rate}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.sampling_rate}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- Gains -->
                        <div class="form-control">
                            <label class="label" for="gains">
                                <span class="label-text font-medium">Gains *</span>
                            </label>
                            <input
                                id="gains"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.gains}
                                class="input input-bordered w-full focus:input-primary"
                            />
                            {#if errors.gains}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.gains}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- LabJack Status -->
                        <div class="form-control">
                            <div class="label">
                                <span class="label-text font-medium">LabJack Status</span>
                            </div>
                            <div class="flex items-center space-x-6">
                                <label class="label cursor-pointer">
                                    <input
                                        type="radio"
                                        bind:group={formData.sensor_settings.labjack_on_off}
                                        value={true}
                                        class="radio radio-primary"
                                    />
                                    <span class="label-text ml-2">Online</span>
                                </label>
                                <label class="label cursor-pointer">
                                    <input
                                        type="radio"
                                        bind:group={formData.sensor_settings.labjack_on_off}
                                        value={false}
                                        class="radio radio-primary"
                                    />
                                    <span class="label-text ml-2">Offline</span>
                                </label>
                            </div>
                        </div>
                    </div>
                </div>

                <!-- Enabled Channels -->
                <div>
                    <h3 class="text-lg font-semibold mb-6 text-base-content">Enabled Channels *</h3>
                    <div class="grid grid-cols-4 md:grid-cols-8 gap-3">
                        {#each Array.from({length: formData.max_channels}, (_, i) => i) as channel}
                            <label class="btn btn-outline btn-sm {formData.sensor_settings.channels_enabled.includes(channel) ? 'btn-primary' : 'btn-ghost'}">
                                <input
                                    type="checkbox"
                                    checked={formData.sensor_settings.channels_enabled.includes(channel)}
                                    onchange={() => handleChannelToggle(channel)}
                                    class="sr-only"
                                />
                                {channel}
                            </label>
                        {/each}
                    </div>
                    {#if errors.channels_enabled}
                        <div class="label">
                            <span class="label-text-alt text-error">{errors.channels_enabled}</span>
                        </div>
                    {/if}
                </div>

                <!-- Channel Configuration -->
                {#if formData.sensor_settings.channels_enabled.length > 0}
                    <div>
                        <h3 class="text-lg font-semibold mb-6 text-base-content">Channel Configuration *</h3>
                        <div class="space-y-4">
                            {#each formData.sensor_settings.channels_enabled as channel, index}
                                <div class="card bg-base-200 border border-base-300">
                                    <div class="card-body p-4">
                                        <h4 class="card-title text-md text-base-content">Channel {channel}</h4>
                                        <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                                            <!-- Data Format for this channel -->
                                            <div class="form-control">
                                                <label class="label" for="data-format-{channel}">
                                                    <span class="label-text font-medium">Data Format</span>
                                                </label>
                                                <select
                                                    id="data-format-{channel}"
                                                    bind:value={formData.sensor_settings.data_formats[index]}
                                                    class="select select-bordered w-full focus:select-primary"
                                                >
                                                    {#each dataFormats as format}
                                                        <option value={format}>
                                                            {format.charAt(0).toUpperCase() + format.slice(1)}
                                                        </option>
                                                    {/each}
                                                </select>
                                            </div>
                                            
                                            <!-- Measurement Unit for this channel -->
                                            <div class="form-control">
                                                <label class="label" for="measurement-unit-{channel}">
                                                    <span class="label-text font-medium">Measurement Unit</span>
                                                </label>
                                                <select
                                                    id="measurement-unit-{channel}"
                                                    bind:value={formData.sensor_settings.measurement_units[index]}
                                                    class="select select-bordered w-full focus:select-primary"
                                                >
                                                    {#each measurementUnits as unit}
                                                        <option value={unit}>{unit}</option>
                                                    {/each}
                                                </select>
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            {/each}
                        </div>
                        {#if errors.data_formats || errors.measurement_units}
                            <div class="label">
                                <span class="label-text-alt text-error">
                                    {errors.data_formats || errors.measurement_units}
                                </span>
                            </div>
                        {/if}
                    </div>
                {/if}
            </form>
        </div>

        <!-- Modal Footer -->
        <div class="modal-action pt-4 border-t border-base-200 flex-shrink-0">
            <button
                type="button"
                onclick={onClose}
                class="btn btn-ghost"
            >
                Cancel
            </button>
            <button
                type="button"
                onclick={handleSave}
                disabled={saving}
                class="btn btn-primary"
            >
                {#if saving}
                    <span class="loading loading-spinner loading-sm"></span>
                    Saving...
                {:else}
                    <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                    </svg>
                    {isAddingNew ? 'Add LabJack' : 'Save Changes'}
                {/if}
            </button>
        </div>
    </div>
</div>
