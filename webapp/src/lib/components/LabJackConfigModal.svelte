<script lang="ts">
    import { normalizeCalibration, type CalibrationSpec } from "$lib/calibration";

    /** `sensor_settings` of a LabJack config document. See `docs/src/reference/kv-config.md`. */
    interface SensorSettings {
        /** Scans per read, and so samples per published message on each channel. */
        scans_per_read: number;
        /** Scans per second, per channel. */
        scan_rate_hz: number;
        /** Analog inputs to stream (`AIN<n>`), kept sorted ascending by this form. */
        channels_enabled: number[];
        /** Edited here but not used by the streamer. */
        gains: number;
        /** What each enabled channel measures, same order as `channels_enabled`. Label only. */
        data_formats: string[];
        /** Unit of each enabled channel after calibration, same order. Label only. */
        measurement_units: string[];
        /** `false` stops streaming. */
        labjack_on_off: boolean;
        /** Volts-to-units conversion per channel, keyed by channel number as a string. */
        calibrations?: Record<string, CalibrationSpec>;
    }
    
    /** One LabJack config document, stored in KV bucket `avenabox`. */
    interface LabJackConfig {
        /** Display name; must be unique (case-insensitive) when adding. */
        labjack_name: string;
        /** Asset number, > 0; must be unique when adding. Used in the archive path. */
        asset_number: number;
        /** Number of inputs offered as channel toggles, 1 to 16. Not used by the streamer. */
        max_channels: number;
        /** Site name, first subject token. */
        site_id?: string;
        /** Edge node name, second subject token. */
        box_id?: string;
        /** Kind of source, normally `labjack`. */
        source_type?: string;
        /** Name of this LabJack in subjects, third token. */
        source_id?: string;
        /** Subject root, normally `avenars`. Labeled "NATS Root" in the form. */
        nats_subject: string;
        /** JetStream stream for live samples, normally `labjacks`. */
        nats_stream: string;
        /** Archive file window, seconds. */
        rotate_secs: number;
        /** What to record. */
        sensor_settings: SensorSettings;
    }
    
    /** Component props. See the `@component` block below. */
    interface Props {
        config: LabJackConfig;
        isAddingNew: boolean;
        existingLabJacks: Map<string, LabJackConfig>;
        availableCalibrations: Map<string, CalibrationSpec>;
        onSaveCalibration: (spec: CalibrationSpec) => Promise<boolean>;
        onSave: (config: LabJackConfig) => void;
        onClose: () => void;
    }
    
    let {
        config,
        isAddingNew,
        existingLabJacks,
        availableCalibrations,
        onSaveCalibration,
        onSave,
        onClose
    }: Props = $props();
    
    /** Working copy being edited. Shallow copy of `config`, taken once at mount. */
    let formData = $state<LabJackConfig>({ ...config });
    /** Validation messages keyed by field name (`labjack_name`, `gains`, ...). */
    let errors = $state<Record<string, string>>({});
    let saving = $state<boolean>(false);
    /** Result of the last Save Preset per channel, keyed by channel number as a string. */
    let calibrationStatus = $state<Record<string, string>>({});
    /** Text of the "Save as preset" id box per channel. */
    let presetIdInputs = $state<Record<string, string>>({});
    /**
     * Raw text of the polynomial coefficient box per channel, kept so partial input such
     * as `1, ` is not overwritten by the parsed coefficients while typing.
     */
    let coeffInputs = $state<Record<string, string>>({});

    /** Makes sure `sensor_settings.calibrations` exists so per-channel edits can write to it. */
    $effect(() => {
        if (!formData.sensor_settings.calibrations) {
            formData.sensor_settings.calibrations = {};
        }
    });
    
    /**
     * Live duplicate check while adding: flags a `labjack_name` already used by another
     * LabJack (case-insensitive) and clears only that message when it no longer applies.
     */
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
    
    /** Same live duplicate check for `asset_number` while adding. */
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
    
    /** Choices for each channel's data format label. */
    const dataFormats = ["voltage", "temperature", "pressure", "current", "resistance"];
    /** Choices for each channel's unit label. */
    const measurementUnits = ["V", "°C", "PSI", "A", "Ω", "Pa", "kPa", "bar"];

    /**
     * Returns the calibration for a channel, normalized to a valid spec.
     *
     * @param channel - Channel number.
     * @returns The stored spec passed through `normalizeCalibration`, which gives identity
     *   when none is stored or it is malformed.
     */
    function getCalibration(channel: number): CalibrationSpec {
        const calibrations = formData.sensor_settings.calibrations ?? {};
        const raw = calibrations[String(channel)] as CalibrationSpec | undefined;
        return normalizeCalibration(raw);
    }

    /**
     * Stores a calibration for a channel in `formData`, replacing the calibrations object
     * so Svelte sees the change.
     *
     * @param channel - Channel number.
     * @param spec - New calibration.
     */
    function setCalibration(channel: number, spec: CalibrationSpec) {
        const calibrations = { ...(formData.sensor_settings.calibrations ?? {}) };
        calibrations[String(channel)] = spec;
        formData.sensor_settings.calibrations = calibrations;
    }

    /**
     * Applies a choice from the Preset dropdown to a channel.
     *
     * @param channel - Channel number.
     * @param presetId - `custom` (no change), `identity`, or the id of a saved preset in
     *   `availableCalibrations`, which is copied including its `id`. Unknown ids do
     *   nothing.
     */
    function applyPreset(channel: number, presetId: string) {
        if (presetId === "custom") {
            return;
        }
        if (presetId === "identity") {
            setCalibration(channel, { type: "identity" });
            return;
        }
        const preset = availableCalibrations.get(presetId);
        if (preset) {
            setCalibration(channel, { ...preset });
            if (preset.type === "polynomial") {
                coeffInputs[String(channel)] = preset.coeffs.join(", ");
            } else {
                delete coeffInputs[String(channel)];
            }
        }
    }

    /**
     * Returns the Preset dropdown value that matches a channel's current calibration.
     *
     * @param channel - Channel number.
     * @returns `identity` for an identity spec without an id, the spec's `id` when that
     *   preset exists, otherwise `custom`.
     */
    function getPresetSelection(channel: number): string {
        const current = getCalibration(channel);
        if (current.type === "identity" && !current.id) {
            return "identity";
        }
        if (current.id && availableCalibrations.has(current.id)) {
            return current.id;
        }
        return "custom";
    }

    /**
     * Switches a channel's calibration type and resets it to that type's neutral values:
     * linear `a = 1, b = 0`, polynomial `[0, 1]`, or identity. Drops any preset id.
     *
     * @param channel - Channel number.
     * @param type - New calibration type.
     */
    function setCalibrationType(channel: number, type: CalibrationSpec["type"]) {
        if (type === "linear") {
            setCalibration(channel, { type: "linear", a: 1, b: 0 });
            delete coeffInputs[String(channel)];
        } else if (type === "polynomial") {
            setCalibration(channel, { type: "polynomial", coeffs: [0, 1] });
            coeffInputs[String(channel)] = "0, 1";
        } else {
            setCalibration(channel, { type: "identity" });
            delete coeffInputs[String(channel)];
        }
    }

    /**
     * Sets the slope or offset of a channel's linear calibration and drops its preset id,
     * since the values no longer match the preset.
     *
     * @param channel - Channel number.
     * @param field - `a` (slope) or `b` (offset).
     * @param value - New value; non-finite input is stored as 0.
     */
    function updateLinearField(channel: number, field: "a" | "b", value: number) {
        const current = getCalibration(channel);
        if (current.type !== "linear") {
            return;
        }
        const next = {
            ...current,
            [field]: Number.isFinite(value) ? value : 0,
        };
        delete next.id;
        setCalibration(channel, next);
    }

    /**
     * Parses the coefficient box and stores a polynomial calibration for a channel.
     *
     * Coefficients are comma-separated, lowest order first (`c0, c1, c2, ...`). Parts that
     * are not finite numbers are skipped; if none are left the spec falls back to `[0, 1]`.
     * The raw text is kept in `coeffInputs`. Drops any preset id.
     *
     * @param channel - Channel number.
     * @param value - Raw text from the input.
     */
    function updatePolynomialCoeffs(channel: number, value: string) {
        coeffInputs[String(channel)] = value;
        const coeffs = value
            .split(",")
            .map((part) => Number(part.trim()))
            .filter((num) => Number.isFinite(num));
        const next: CalibrationSpec = {
            type: "polynomial",
            coeffs: coeffs.length > 0 ? coeffs : [0, 1],
        };
        setCalibration(channel, next);
    }

    /**
     * Saves a channel's current calibration as a named preset.
     *
     * Sanitizes the typed id with {@link sanitizeCalibrationId} (and writes the sanitized
     * form back to the input), then calls `onSaveCalibration`. On success the channel's
     * calibration is tagged with the new id. The outcome is shown under the channel.
     *
     * @param channel - Channel number.
     * @returns A promise that resolves when the save attempt finishes. Does not reject
     *   unless `onSaveCalibration` does.
     */
    async function handleSavePreset(channel: number) {
        const raw = presetIdInputs[String(channel)] ?? "";
        const sanitized = sanitizeCalibrationId(raw);
        if (!sanitized) {
            calibrationStatus[String(channel)] = "Preset id is required.";
            return;
        }
        const current = getCalibration(channel);
        if (sanitized !== raw.trim()) {
            presetIdInputs[String(channel)] = sanitized;
        }
        const spec: CalibrationSpec = { ...current, id: sanitized };
        const ok = await onSaveCalibration(spec);
        if (ok) {
            setCalibration(channel, spec);
            calibrationStatus[String(channel)] = `Saved preset '${sanitized}'.`;
        } else {
            calibrationStatus[String(channel)] = "Failed to save preset.";
        }
    }

    /**
     * Turns free text into a preset id: trimmed, lowercased, whitespace runs replaced by
     * `-`, and everything except `a-z`, `0-9`, `.`, `_` and `-` removed.
     *
     * @param raw - Text typed by the user.
     * @returns The id, possibly empty.
     *
     * @example
     * ```ts
     * sanitizeCalibrationId(" TP 3505 (new) "); // "tp-3505-new"
     * ```
     */
    function sanitizeCalibrationId(raw: string): string {
        return raw
            .trim()
            .toLowerCase()
            .replace(/\s+/g, "-")
            .replace(/[^a-z0-9._-]/g, "");
    }
    
    /**
     * Checks the whole form and replaces `errors` with the problems found.
     *
     * Requires a name and, when adding, a name and asset number not used by another
     * LabJack; asset number, rotate interval, scans per read, scan rate and gains above 0;
     * max channels from 1 to 16; non-empty NATS root and stream; at least one enabled
     * channel; and one data format and one unit per enabled channel. Site, box, source
     * and calibrations are not checked.
     *
     * @returns `true` if the form is valid.
     */
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
        
        if (formData.sensor_settings.scans_per_read <= 0) {
            errors.scans_per_read = "Scans per read must be greater than 0";
        }
        
        if (formData.sensor_settings.scan_rate_hz <= 0) {
            errors.scan_rate_hz = "Scan rate must be greater than 0";
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
    
    /**
     * Validates the form and, if valid, passes `formData` to `onSave`, showing the
     * spinner until it settles. Validation errors are shown inline and nothing is sent.
     *
     * @returns A promise that resolves when `onSave` finishes. Rejects if `onSave`
     *   rejects.
     */
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
    
    /**
     * Enables or disables a channel.
     *
     * Disabling removes the channel's data format, unit and calibration. Enabling appends
     * `voltage`, `V` and an identity calibration. `channels_enabled` is then sorted.
     *
     * @param channel - Channel number, 0 to `max_channels - 1`.
     */
    function handleChannelToggle(channel: number) {
        const channels = [...formData.sensor_settings.channels_enabled];
        const index = channels.indexOf(channel);
        const calibrations = { ...(formData.sensor_settings.calibrations ?? {}) };
        
        if (index > -1) {
            // Remove channel and corresponding data format/measurement unit
            channels.splice(index, 1);
            formData.sensor_settings.data_formats.splice(index, 1);
            formData.sensor_settings.measurement_units.splice(index, 1);
            delete calibrations[String(channel)];
        } else {
            // Add channel and default data format/measurement unit
            channels.push(channel);
            formData.sensor_settings.data_formats.push("voltage");
            formData.sensor_settings.measurement_units.push("V");
            calibrations[String(channel)] = { type: "identity" };
        }
        
        formData.sensor_settings.channels_enabled = channels.sort((a, b) => a - b);
        
        // Meant to reorder data formats and units to match the sorted channels, but
        // `originalIndex` is looked up in the already sorted list, so it equals the loop
        // index and the arrays keep their order. Enabling a channel below an existing one
        // therefore leaves its `voltage`/`V` defaults at the end, shifting labels by one.
        const sortedDataFormats = [];
        const sortedMeasurementUnits = [];
        
        for (const sortedChannel of formData.sensor_settings.channels_enabled) {
            const originalIndex = formData.sensor_settings.channels_enabled.indexOf(sortedChannel);
            sortedDataFormats.push(formData.sensor_settings.data_formats[originalIndex] || "voltage");
            sortedMeasurementUnits.push(formData.sensor_settings.measurement_units[originalIndex] || "V");
        }
        
        formData.sensor_settings.data_formats = sortedDataFormats;
        formData.sensor_settings.measurement_units = sortedMeasurementUnits;
        formData.sensor_settings.calibrations = calibrations;
    }
    
    
    /**
     * Closes the modal on Escape, from anywhere in the window.
     *
     * @param event - Window keydown event.
     */
    function handleKeyPress(event: KeyboardEvent) {
        if (event.key === 'Escape') {
            onClose();
        }
    }
</script>

<!--
@component
Modal form to add or edit one LabJack config document. The document lives in KV bucket
`avenabox` under `<site_id>.<box_id>.<source_id>.config` (see
`docs/src/reference/kv-config.md`). This component does no NATS I/O itself: it edits
a local copy and hands it to `onSave`; the `/labjacks` page writes it to KV. When
adding, the page builds the key from `site_id`, `box_id` and `source_id` (or
`labjack_name`); when editing, it keeps the existing key.

Fields edited:
- Top level: `labjack_name`, `asset_number`, `max_channels`, `rotate_secs`,
  `nats_subject` (labeled "NATS Root"), `nats_stream`, `site_id`, `box_id`,
  `source_type`, `source_id`.
- `sensor_settings`: `scans_per_read`, `scan_rate_hz`, `gains`, `labjack_on_off`
  (Online/Offline), `channels_enabled` (toggles 0 to `max_channels - 1`), and per
  enabled channel `data_formats`, `measurement_units` and `calibrations`
  (identity, linear `a`/`b`, or polynomial coefficients).

A channel's calibration can be picked from saved presets or saved as a new preset
through `onSaveCalibration` (the page stores presets in `avenabox` under
`calibration.<id>`). Saving runs full validation first; name and asset number
duplicates are also flagged live while adding. Escape, the close button, Cancel, or a
click on the backdrop close the modal without saving.

Props:
- `config: LabJackConfig`: document to edit, or the defaults for a new one. Copied
  once at mount.
- `isAddingNew: boolean`: new LabJack (enables duplicate checks, changes titles).
- `existingLabJacks: Map<string, LabJackConfig>`: all loaded configs by KV key, used
  for the duplicate name and asset number checks.
- `availableCalibrations: Map<string, CalibrationSpec>`: saved presets by id.
- `onSaveCalibration: (spec: CalibrationSpec) => Promise<boolean>`: saves `spec` (with
  its sanitized `id`) as a preset; resolves `true` on success.
- `onSave: (config: LabJackConfig) => void`: called with the edited document after
  validation passes. Awaited, so it may return a promise.
- `onClose: () => void`: called to close the modal.

No props have defaults.
-->

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
                                <span class="label-text font-medium">NATS Root *</span>
                            </label>
                            <input
                                id="nats_subject"
                                type="text"
                                bind:value={formData.nats_subject}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="e.g., avenars"
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

                        <!-- Site ID -->
                        <div class="form-control">
                            <label class="label" for="site_id">
                                <span class="label-text font-medium">Site ID</span>
                            </label>
                            <input
                                id="site_id"
                                type="text"
                                bind:value={formData.site_id}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="e.g., i69"
                            />
                        </div>

                        <!-- Box ID -->
                        <div class="form-control">
                            <label class="label" for="box_id">
                                <span class="label-text font-medium">Box ID</span>
                            </label>
                            <input
                                id="box_id"
                                type="text"
                                bind:value={formData.box_id}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="e.g., i69-mu2"
                            />
                        </div>

                        <!-- Source Type -->
                        <div class="form-control">
                            <label class="label" for="source_type">
                                <span class="label-text font-medium">Source Type</span>
                            </label>
                            <input
                                id="source_type"
                                type="text"
                                bind:value={formData.source_type}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="e.g., labjack"
                            />
                        </div>

                        <!-- Source ID -->
                        <div class="form-control">
                            <label class="label" for="source_id">
                                <span class="label-text font-medium">Source ID</span>
                            </label>
                            <input
                                id="source_id"
                                type="text"
                                bind:value={formData.source_id}
                                class="input input-bordered w-full focus:input-primary"
                                placeholder="e.g., i69-lj2"
                            />
                        </div>
                    </div>
                </div>

                <!-- Sensor Settings -->
                <div>
                    <h3 class="text-lg font-semibold mb-6 text-base-content">Sensor Settings</h3>
                    <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
                        <!-- Scans Per Read -->
                        <div class="form-control">
                            <label class="label" for="scans_per_read">
                                <span class="label-text font-medium">Scans Per Read *</span>
                            </label>
                            <input
                                id="scans_per_read"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.scans_per_read}
                                class="input input-bordered w-full focus:input-primary"
                            />
                            {#if errors.scans_per_read}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.scans_per_read}</span>
                                </div>
                            {/if}
                        </div>

                        <!-- Scan Rate -->
                        <div class="form-control">
                            <label class="label" for="scan_rate_hz">
                                <span class="label-text font-medium">Scan Rate (Hz) *</span>
                            </label>
                            <input
                                id="scan_rate_hz"
                                type="number"
                                min="1"
                                bind:value={formData.sensor_settings.scan_rate_hz}
                                class="input input-bordered w-full focus:input-primary"
                            />
                            {#if errors.scan_rate_hz}
                                <div class="label">
                                    <span class="label-text-alt text-error">{errors.scan_rate_hz}</span>
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

                                        <div class="mt-4 border-t border-base-300 pt-4 space-y-4">
                                            <div class="flex items-center justify-between">
                                                <h5 class="text-sm font-semibold text-base-content">Calibration</h5>
                                                {#if getCalibration(channel).id}
                                                    <span class="text-xs text-base-content/60">
                                                        Active preset: {getCalibration(channel).id}
                                                    </span>
                                                {/if}
                                            </div>
                                            <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                                                <div class="form-control">
                                                    <label class="label" for="calibration-preset-{channel}">
                                                        <span class="label-text font-medium">Preset</span>
                                                    </label>
                                                    <select
                                                        id="calibration-preset-{channel}"
                                                        value={getPresetSelection(channel)}
                                                        onchange={(event) => applyPreset(channel, (event.currentTarget as HTMLSelectElement).value)}
                                                        class="select select-bordered w-full focus:select-primary"
                                                    >
                                                        <option value="identity">Identity (raw)</option>
                                                        <option value="custom">Custom / Unsaved</option>
                                                        {#each Array.from(availableCalibrations.values()).sort((a, b) => (a.id ?? "").localeCompare(b.id ?? "")) as preset}
                                                            <option value={preset.id ?? ""}>
                                                                {preset.id ?? "Unnamed preset"}
                                                            </option>
                                                        {/each}
                                                    </select>
                                                </div>
                                                <div class="form-control">
                                                    <label class="label" for="calibration-type-{channel}">
                                                        <span class="label-text font-medium">Type</span>
                                                    </label>
                                                    <select
                                                        id="calibration-type-{channel}"
                                                        value={getCalibration(channel).type}
                                                        onchange={(event) => setCalibrationType(channel, (event.currentTarget as HTMLSelectElement).value as CalibrationSpec["type"])}
                                                        class="select select-bordered w-full focus:select-primary"
                                                    >
                                                        <option value="identity">Identity</option>
                                                        <option value="linear">Linear</option>
                                                        <option value="polynomial">Polynomial</option>
                                                    </select>
                                                </div>
                                            </div>

                                            {#if getCalibration(channel).type === "linear"}
                                                <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                                                    <div class="form-control">
                                                        <label class="label" for="calibration-linear-a-{channel}">
                                                            <span class="label-text font-medium">Slope (a)</span>
                                                        </label>
                                                        <input
                                                            id="calibration-linear-a-{channel}"
                                                            type="number"
                                                            step="any"
                                                            value={getCalibration(channel).type === "linear" ? getCalibration(channel).a : 1}
                                                            oninput={(event) => updateLinearField(channel, "a", Number((event.currentTarget as HTMLInputElement).value))}
                                                            class="input input-bordered w-full focus:input-primary"
                                                        />
                                                    </div>
                                                    <div class="form-control">
                                                        <label class="label" for="calibration-linear-b-{channel}">
                                                            <span class="label-text font-medium">Offset (b)</span>
                                                        </label>
                                                        <input
                                                            id="calibration-linear-b-{channel}"
                                                            type="number"
                                                            step="any"
                                                            value={getCalibration(channel).type === "linear" ? getCalibration(channel).b : 0}
                                                            oninput={(event) => updateLinearField(channel, "b", Number((event.currentTarget as HTMLInputElement).value))}
                                                            class="input input-bordered w-full focus:input-primary"
                                                        />
                                                    </div>
                                                </div>
                                            {:else if getCalibration(channel).type === "polynomial"}
                                                <div class="form-control">
                                                    <label class="label" for="calibration-poly-{channel}">
                                                        <span class="label-text font-medium">Coefficients (c0, c1, c2...)</span>
                                                    </label>
                                                    <input
                                                        id="calibration-poly-{channel}"
                                                        type="text"
                                                        value={coeffInputs[String(channel)] ?? getCalibration(channel).coeffs.join(", ")}
                                                        oninput={(event) => updatePolynomialCoeffs(channel, (event.currentTarget as HTMLInputElement).value)}
                                                        class="input input-bordered w-full focus:input-primary"
                                                    />
                                                </div>
                                            {/if}

                                            <div class="grid grid-cols-1 md:grid-cols-[1fr_auto] gap-4 items-end">
                                                <div class="form-control">
                                                    <label class="label" for="calibration-save-id-{channel}">
                                                        <span class="label-text font-medium">Save as preset</span>
                                                    </label>
                                                    <input
                                                        id="calibration-save-id-{channel}"
                                                        type="text"
                                                        placeholder="preset id"
                                                        bind:value={presetIdInputs[String(channel)]}
                                                        class="input input-bordered w-full focus:input-primary"
                                                    />
                                                </div>
                                                <button
                                                    type="button"
                                                    onclick={() => handleSavePreset(channel)}
                                                    class="btn btn-outline btn-primary"
                                                >
                                                    Save Preset
                                                </button>
                                            </div>
                                            {#if calibrationStatus[String(channel)]}
                                                <p class="text-xs text-base-content/70">
                                                    {calibrationStatus[String(channel)]}
                                                </p>
                                            {/if}
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
