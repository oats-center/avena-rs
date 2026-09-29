<script lang="ts">
    import {
        applyCalibration,
        formatCalibration,
        normalizeCalibration,
        strainBridgeCalibration,
        RAW_UNIT,
        type CalibrationSpec
    } from "$lib/calibration";
    import {
        channelsMissingUnit,
        findSensorType,
        normalizeSensorSettings,
        SENSOR_TYPES,
        syncChannelCalibrations,
        type LabJackConfig
    } from "$lib/labjack-config";
    import {
        describeFilters,
        normalizeChannelFilters,
        type ChannelFilterSettings
    } from "$lib/filter-settings";
    import { planFilters } from "$lib/filters";

    /** Component props. See the `@component` block below. */
    interface Props {
        config: LabJackConfig;
        isAddingNew: boolean;
        existingLabJacks: Map<string, LabJackConfig>;
        onSave: (config: LabJackConfig) => void;
        onClose: () => void;
    }
    
    let {
        config,
        isAddingNew,
        existingLabJacks,
        onSave,
        onClose
    }: Props = $props();

    /**
     * Deep copy of `config` taken once at mount, with every enabled channel's
     * calibration normalized and its unit filled in (see `syncChannelCalibrations`), so
     * older configs open with the unit they were showing.
     */
    function prepareForm(source: LabJackConfig): LabJackConfig {
        const copy = $state.snapshot(source) as LabJackConfig;
        copy.sensor_settings = syncChannelCalibrations(normalizeSensorSettings(copy.sensor_settings));
        return copy;
    }
    
    /**
     * Working copy being edited, from {@link prepareForm}, so Cancel leaves the caller's
     * object (including `sensor_settings`) untouched.
     */
    let formData = $state<LabJackConfig>(prepareForm(config));
    /** `formData` as JSON when the modal opened, used to tell whether there are unsaved edits. */
    const initialJson = serializeForm(formData);
    /** Validation messages keyed by field name (`labjack_name`, `gains`, ...). */
    let errors = $state<Record<string, string>>({});
    let saving = $state<boolean>(false);
    /**
     * Raw text of the polynomial coefficient box per channel, kept so partial input such
     * as `1, ` is not overwritten by the parsed coefficients while typing.
     */
    let coeffInputs = $state<Record<string, string>>({});
    /**
     * How a strain channel's linear calibration is entered, per channel: `direct` (type
     * `a` and `b`) or `bridge` (compute them from the gauge and amplifier settings).
     */
    let linearModes = $state<Record<string, "direct" | "bridge">>({});
    /** Text of the bridge helper boxes per channel. */
    let bridgeInputs = $state<Record<string, { factor: string; excitation: string; gain: string; zero: string }>>({});

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
    
    /**
     * Returns the calibration for a channel, normalized to a valid spec.
     *
     * @param channel - Channel number.
     * @returns The stored spec passed through `normalizeCalibration`, which gives identity
     *   when none is stored or it is malformed.
     */
    function getCalibration(channel: number): CalibrationSpec {
        const calibrations = formData.sensor_settings.calibrations ?? {};
        return normalizeCalibration(calibrations[String(channel)]);
    }

    /**
     * Stores a calibration for a channel and copies its unit to `measurement_units`,
     * replacing the calibrations object so Svelte sees the change.
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     * @param spec - New calibration.
     */
    function setCalibration(channel: number, index: number, spec: CalibrationSpec) {
        const calibrations = { ...(formData.sensor_settings.calibrations ?? {}) };
        calibrations[String(channel)] = spec;
        formData.sensor_settings.calibrations = calibrations;
        formData.sensor_settings.measurement_units[index] =
            spec.type === "identity" ? RAW_UNIT : (spec.unit ?? "");
    }

    /**
     * Unit choices for a calibrated channel: the units of its sensor type (all known
     * units for a type not in `SENSOR_TYPES`), plus its current unit when it is not one
     * of them, so an unusual unit from KV is shown and kept.
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     * @returns Unit options for the channel's select.
     */
    function unitOptions(channel: number, index: number): string[] {
        const type = findSensorType(formData.sensor_settings.data_formats[index]);
        const units = type ? [...type.units] : [...new Set(SENSOR_TYPES.flatMap((t) => t.units))];
        const current = getCalibration(channel).unit;
        return current && !units.includes(current) ? [...units, current] : units;
    }

    /**
     * Default unit for a new calibration on a channel: its current unit when it has one,
     * otherwise the first unit of its sensor type.
     */
    function defaultUnit(channel: number, index: number): string {
        const current = getCalibration(channel);
        if (current.type !== "identity" && current.unit) return current.unit;
        return findSensorType(formData.sensor_settings.data_formats[index])?.units[0] ?? RAW_UNIT;
    }

    /**
     * Changes a channel's sensor type. A calibrated channel whose unit the new type does
     * not offer is moved to the type's first unit.
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     * @param format - New `data_formats` value.
     */
    function setSensorType(channel: number, index: number, format: string) {
        formData.sensor_settings.data_formats[index] = format;
        const type = findSensorType(format);
        const current = getCalibration(channel);
        if (type && current.type !== "identity" && (!current.unit || !type.units.includes(current.unit))) {
            setCalibration(channel, index, { ...current, unit: type.units[0] });
        }
        if (format !== "strain") {
            delete linearModes[String(channel)];
        }
    }

    /**
     * Sets the unit of a channel's calibration (and `measurement_units`).
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     * @param unit - New unit.
     */
    function setUnit(channel: number, index: number, unit: string) {
        const current = getCalibration(channel);
        if (current.type === "identity") return;
        setCalibration(channel, index, { ...current, unit });
    }

    /**
     * Switches a channel's calibration type and resets it to that type's neutral values:
     * linear `a = 1, b = 0`, polynomial `[0, 1]`, or identity (unit `V`). A new linear
     * or polynomial calibration keeps the channel's unit, or takes its sensor type's
     * first unit. Drops any `id` from an older config.
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     * @param type - New calibration type.
     */
    function setCalibrationType(channel: number, index: number, type: CalibrationSpec["type"]) {
        const unit = defaultUnit(channel, index);
        if (type === "linear") {
            setCalibration(channel, index, { type: "linear", a: 1, b: 0, unit });
            delete coeffInputs[String(channel)];
        } else if (type === "polynomial") {
            setCalibration(channel, index, { type: "polynomial", coeffs: [0, 1], unit });
            coeffInputs[String(channel)] = "0, 1";
        } else {
            setCalibration(channel, index, { type: "identity", unit: RAW_UNIT });
            delete coeffInputs[String(channel)];
        }
    }

    /**
     * Sets the slope or offset of a channel's linear calibration and drops any `id`
     * from an older config, since the formula no longer matches it.
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     * @param field - `a` (slope) or `b` (offset).
     * @param value - New value; non-finite input is stored as 0.
     */
    function updateLinearField(channel: number, index: number, field: "a" | "b", value: number) {
        const current = getCalibration(channel);
        if (current.type !== "linear") {
            return;
        }
        const next = {
            ...current,
            [field]: Number.isFinite(value) ? value : 0,
        };
        delete next.id;
        setCalibration(channel, index, next);
    }

    /**
     * Parses the coefficient box and stores a polynomial calibration for a channel,
     * keeping its unit.
     *
     * Coefficients are comma-separated, lowest order first (`c0, c1, c2, ...`). Parts that
     * are not finite numbers are skipped; if none are left the spec falls back to `[0, 1]`.
     * The raw text is kept in `coeffInputs`. Drops any `id` from an older config.
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     * @param value - Raw text from the input.
     */
    function updatePolynomialCoeffs(channel: number, index: number, value: string) {
        coeffInputs[String(channel)] = value;
        const coeffs = value
            .split(",")
            .map((part) => part.trim())
            .filter((part) => part !== "")
            .map(Number)
            .filter((num) => Number.isFinite(num));
        const unit = getCalibration(channel).unit;
        const next: CalibrationSpec = {
            type: "polynomial",
            coeffs: coeffs.length > 0 ? coeffs : [0, 1],
            ...(unit ? { unit } : {}),
        };
        setCalibration(channel, index, next);
    }

    /**
     * Switches how a strain channel's linear calibration is entered. Opening the bridge
     * helper creates its (empty) boxes the first time.
     *
     * @param channel - Channel number.
     * @param mode - `direct` or `bridge`.
     */
    function setLinearMode(channel: number, mode: "direct" | "bridge") {
        const key = String(channel);
        if (mode === "bridge" && !bridgeInputs[key]) {
            bridgeInputs[key] = { factor: "", excitation: "", gain: "", zero: "" };
        }
        linearModes[key] = mode;
    }

    /**
     * Reads a bridge helper box.
     *
     * @param text - Box contents.
     * @param blank - Value of an empty box.
     * @returns The number, `blank` for an empty box, or `NaN` when it is not a number.
     */
    function parseBox(text: string | number | null | undefined, blank: number): number {
        const trimmed = String(text ?? "").trim();
        return trimmed === "" ? blank : Number(trimmed);
    }

    /**
     * Linear calibration computed by the bridge helper for a channel, from
     * `strainBridgeCalibration`.
     *
     * @param channel - Channel number.
     * @returns The calibration, or `null` while the inputs are incomplete or invalid.
     */
    function bridgeResult(channel: number): CalibrationSpec | null {
        const inputs = bridgeInputs[String(channel)];
        if (!inputs) return null;
        return strainBridgeCalibration({
            factor: parseBox(inputs.factor, Number.NaN),
            excitation: parseBox(inputs.excitation, Number.NaN),
            gain: parseBox(inputs.gain, Number.NaN),
            zero: parseBox(inputs.zero, 0),
        });
    }

    /**
     * Stores the bridge helper's result as the channel's calibration (unit µε) and
     * switches the entry mode back to direct, showing the stored `a` and `b`.
     *
     * @param channel - Channel number.
     * @param index - Position of the channel in `channels_enabled`.
     */
    function applyBridge(channel: number, index: number) {
        const result = bridgeResult(channel);
        if (!result) return;
        setCalibration(channel, index, result);
        linearModes[String(channel)] = "direct";
    }

    /**
     * Formats a number for the calibration preview with up to six significant digits.
     *
     * @param value - Number to show.
     * @returns The text, or `—` for a non-finite value.
     */
    function formatValue(value: number): string {
        return Number.isFinite(value) ? String(Number(value.toPrecision(6))) : "—";
    }

    /**
     * Preview line for a calibration: what a raw reading of 1 V becomes.
     *
     * @param spec - Calibration.
     * @returns For example `1.000 V → 481.26 µε`.
     */
    function previewLine(spec: CalibrationSpec): string {
        const unit = spec.type === "identity" ? RAW_UNIT : (spec.unit ?? "(no unit)");
        return `raw 1.000 V → ${formatValue(applyCalibration(spec, 1))} ${unit}`;
    }
    
    /**
     * Returns a channel's noise filter settings (only the active ones).
     *
     * @param channel - Channel number.
     */
    function getFilters(channel: number): ChannelFilterSettings {
        return normalizeChannelFilters(formData.sensor_settings.filters?.[String(channel)]) ?? {};
    }

    /**
     * Turns one noise filter of a channel on or off, or sets a cutoff. A channel with
     * no filter on is removed from `filters`, and `filters` itself when it is empty, so
     * a config without filters keeps its shape.
     *
     * @param channel - Channel number.
     * @param field - Filter to change.
     * @param value - `true`/`false` for a switch; a number, or `undefined` for off, for a cutoff.
     */
    function setFilter(channel: number, field: keyof ChannelFilterSettings, value: boolean | number | undefined) {
        const key = String(channel);
        const next = normalizeChannelFilters({ ...getFilters(channel), [field]: value });
        const filters = { ...(formData.sensor_settings.filters ?? {}) };
        if (next) filters[key] = next;
        else delete filters[key];
        if (Object.keys(filters).length > 0) formData.sensor_settings.filters = filters;
        else delete formData.sensor_settings.filters;
    }

    /**
     * Reads a cutoff box: blank, zero or not a number turns the filter off.
     *
     * @param channel - Channel number.
     * @param field - `highpass_hz` or `lowpass_hz`.
     * @param input - The input element; rewritten with the stored value.
     */
    function commitCutoff(channel: number, field: "highpass_hz" | "lowpass_hz", input: HTMLInputElement) {
        const text = input.value.trim();
        const number = Number(text);
        setFilter(channel, field, text !== "" && Number.isFinite(number) && number > 0 ? number : undefined);
        input.value = String(getFilters(channel)[field] ?? "");
    }

    /**
     * Checks the whole form and replaces `errors` with the problems found.
     *
     * Requires a name and, when adding, a name and asset number not used by another
     * LabJack; asset number, rotate interval, scans per read, scan rate and gains above 0;
     * max channels from 1 to 16; non-empty NATS root and stream; at least one enabled
     * channel; one data format and one unit per enabled channel; and a unit for every
     * calibrated channel. Site, box and source are not checked.
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

        const missingUnit = channelsMissingUnit(formData.sensor_settings);
        if (missingUnit.length > 0) {
            errors.calibration_units = `Choose the unit of the calibration on channel ${missingUnit.join(", ")}.`;
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
     * Disabling removes the channel's sensor type, unit and calibration. Enabling adds
     * `voltage`, `V` and an identity calibration with unit `V`. `channels_enabled` is then sorted and
     * `data_formats` and `measurement_units` are rebuilt in the same order, so every
     * channel keeps its own labels whatever order channels are toggled in.
     *
     * @param channel - Channel number, 0 to `max_channels - 1`.
     */
    function handleChannelToggle(channel: number) {
        const settings = formData.sensor_settings;
        const calibrations = { ...(settings.calibrations ?? {}) };

        // Pair each enabled channel with its labels before changing anything.
        const labels = new Map<number, { format: string; unit: string }>();
        settings.channels_enabled.forEach((ch, i) => {
            labels.set(ch, {
                format: settings.data_formats[i] || "voltage",
                unit: settings.measurement_units[i] || "V",
            });
        });

        if (labels.has(channel)) {
            labels.delete(channel);
            delete calibrations[String(channel)];
            if (settings.filters?.[String(channel)]) {
                const filters = { ...settings.filters };
                delete filters[String(channel)];
                if (Object.keys(filters).length > 0) settings.filters = filters;
                else delete settings.filters;
            }
        } else {
            labels.set(channel, { format: "voltage", unit: "V" });
            calibrations[String(channel)] = { type: "identity", unit: RAW_UNIT };
        }

        const channels = [...labels.keys()].sort((a, b) => a - b);
        settings.channels_enabled = channels;
        settings.data_formats = channels.map((ch) => labels.get(ch)!.format);
        settings.measurement_units = channels.map((ch) => labels.get(ch)!.unit);
        settings.calibrations = calibrations;
    }
    
    
    /**
     * Returns the form as JSON for the unsaved-edits check. A missing `calibrations` is
     * treated as `{}`.
     */
    function serializeForm(data: LabJackConfig): string {
        const snapshot = $state.snapshot(data) as LabJackConfig;
        return JSON.stringify({
            ...snapshot,
            sensor_settings: { ...snapshot.sensor_settings, calibrations: snapshot.sensor_settings.calibrations ?? {} }
        });
    }

    /** True when the form differs from what it was when the modal opened. */
    function isDirty(): boolean {
        return serializeForm(formData) !== initialJson;
    }

    /**
     * Closes the modal without saving. When the form has unsaved edits it asks first and
     * does nothing if the user declines.
     */
    function requestClose() {
        if (saving) return;
        if (isDirty() && !confirm("Discard your unsaved changes to this LabJack configuration?")) {
            return;
        }
        onClose();
    }

    /**
     * Handles Escape inside the modal. Stops the event so the window handler does not
     * ask a second time.
     *
     * @param event - Keydown event from the backdrop or the dialog.
     */
    function handleModalKeydown(event: KeyboardEvent) {
        if (event.key === 'Escape') {
            event.stopPropagation();
            requestClose();
        }
    }

    /**
     * Closes the modal on Escape, from anywhere in the window.
     *
     * @param event - Window keydown event.
     */
    function handleKeyPress(event: KeyboardEvent) {
        if (event.key === 'Escape') {
            requestClose();
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
  enabled channel its sensor type (`data_formats`), its calibration (identity, linear
  `a`/`b`, or polynomial coefficients), the calibration's unit, and its noise filters
  (`filters`: spike removal, 10 Hz and 11.9 Hz removal, high-pass and low-pass
  cutoffs), which apply only where the data is read. Filters that cannot run at the
  scan rate are named under them.

Each channel has one calibration, edited in place and saved with the form; there are
no named presets. The calibration stores its unit (`calibrations[ch].unit`), which is
also copied to `measurement_units` for older readers. Identity means raw volts (`V`); a
linear or polynomial calibration must have a unit. A strain gauge channel's linear
calibration can be typed directly or computed with the bridge helper from the gauge
factor (µε per mV/V), bridge excitation, amplifier gain and zero reading. Each channel
shows its sensor type, unit, formula and what a raw 1 V reading becomes.

Saving runs full validation first; name and asset number duplicates are also flagged
live while adding. Escape, the close button, Cancel, or a click on the backdrop close
the modal without saving, asking first when the form has unsaved edits.

Props:
- `config: LabJackConfig`: document to edit, or the defaults for a new one. Deep-copied
  once at mount, so closing without saving discards every edit. Older configs whose
  calibrations have no `unit` open with the unit from `measurement_units` (unless it
  is `V`); an `id` from an older config is kept until the formula is changed.
- `isAddingNew: boolean`: new LabJack (enables duplicate checks, changes titles).
- `existingLabJacks: Map<string, LabJackConfig>`: all loaded configs by KV key, used
  for the duplicate name and asset number checks.
- `onSave: (config: LabJackConfig) => void`: called with the edited document after
  validation passes. Awaited, so it may return a promise.
- `onClose: () => void`: called to close the modal.

No props have defaults.
-->

<svelte:window on:keydown={handleKeyPress} />

<!-- Modal -->
<div class="modal modal-open" onclick={requestClose} role="button" tabindex="0" onkeydown={handleModalKeydown}>
    <div class="modal-box w-11/12 max-w-4xl h-[90vh] flex flex-col bg-base-100 shadow-2xl border border-base-200" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="0" onkeydown={handleModalKeydown}>
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
                onclick={requestClose}
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
                                {@const cal = getCalibration(channel)}
                                {@const key = String(channel)}
                                {@const sensorType = findSensorType(formData.sensor_settings.data_formats[index])}
                                {@const isStrain = formData.sensor_settings.data_formats[index] === "strain"}
                                {@const filters = getFilters(channel)}
                                {@const plan = planFilters(filters, formData.sensor_settings.scan_rate_hz)}
                                <div class="card bg-base-200 border border-base-300">
                                    <div class="card-body p-4">
                                        <div class="flex flex-wrap items-baseline justify-between gap-2">
                                            <h4 class="card-title text-md text-base-content">Channel {channel}</h4>
                                            <!-- Summary: sensor type, unit, formula and a 1 V preview. -->
                                            <div class="flex flex-wrap gap-2 text-xs" aria-label="Channel {channel} calibration summary">
                                                <span class="badge badge-outline badge-sm">{sensorType?.label ?? formData.sensor_settings.data_formats[index] ?? "—"}</span>
                                                <span class="badge badge-sm {cal.type !== 'identity' && !cal.unit ? 'badge-warning' : 'badge-secondary'}">
                                                    {cal.type === "identity" ? `${RAW_UNIT} (raw)` : (cal.unit ?? "unit not set")}
                                                </span>
                                                <span class="font-mono text-base-content/70">{formatCalibration(cal)}</span>
                                                <span class="font-mono text-base-content/70">{previewLine(cal)}</span>
                                            </div>
                                        </div>
                                        <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
                                            <!-- Sensor type (data_formats) -->
                                            <div class="form-control">
                                                <label class="label" for="data-format-{channel}">
                                                    <span class="label-text font-medium">Sensor type</span>
                                                </label>
                                                <select
                                                    id="data-format-{channel}"
                                                    value={formData.sensor_settings.data_formats[index]}
                                                    onchange={(event) => setSensorType(channel, index, (event.currentTarget as HTMLSelectElement).value)}
                                                    class="select select-bordered w-full focus:select-primary"
                                                >
                                                    {#each SENSOR_TYPES as type}
                                                        <option value={type.format}>{type.label}</option>
                                                    {/each}
                                                    {#if !sensorType}
                                                        <option value={formData.sensor_settings.data_formats[index]}>{formData.sensor_settings.data_formats[index]}</option>
                                                    {/if}
                                                </select>
                                            </div>

                                            <!-- Calibration type -->
                                            <div class="form-control">
                                                <label class="label" for="calibration-type-{channel}">
                                                    <span class="label-text font-medium">Calibration</span>
                                                </label>
                                                <select
                                                    id="calibration-type-{channel}"
                                                    value={cal.type}
                                                    onchange={(event) => setCalibrationType(channel, index, (event.currentTarget as HTMLSelectElement).value as CalibrationSpec["type"])}
                                                    class="select select-bordered w-full focus:select-primary"
                                                >
                                                    <option value="identity">None (raw volts)</option>
                                                    <option value="linear">Linear (a·x + b)</option>
                                                    <option value="polynomial">Polynomial</option>
                                                </select>
                                            </div>

                                            <!-- Unit of the calibrated values -->
                                            <div class="form-control">
                                                <label class="label" for="measurement-unit-{channel}">
                                                    <span class="label-text font-medium">Unit</span>
                                                </label>
                                                {#if cal.type === "identity"}
                                                    <select id="measurement-unit-{channel}" class="select select-bordered w-full" disabled>
                                                        <option>{RAW_UNIT}</option>
                                                    </select>
                                                {:else}
                                                    <select
                                                        id="measurement-unit-{channel}"
                                                        value={cal.unit ?? ""}
                                                        onchange={(event) => setUnit(channel, index, (event.currentTarget as HTMLSelectElement).value)}
                                                        class="select select-bordered w-full focus:select-primary"
                                                        class:select-warning={!cal.unit}
                                                        aria-describedby={!cal.unit ? `measurement-unit-warning-${channel}` : undefined}
                                                    >
                                                        {#if !cal.unit}
                                                            <option value="" disabled>Choose a unit</option>
                                                        {/if}
                                                        {#each unitOptions(channel, index) as unit}
                                                            <option value={unit}>{unit}</option>
                                                        {/each}
                                                    </select>
                                                    {#if !cal.unit}
                                                        <p id="measurement-unit-warning-{channel}" class="text-xs text-warning mt-1" role="status">
                                                            Choose the unit this calibration converts to. The live plot and
                                                            exports label the values with it.
                                                        </p>
                                                    {/if}
                                                {/if}
                                            </div>
                                        </div>

                                        {#if cal.type === "identity" && sensorType && sensorType.format !== "voltage"}
                                            <p class="text-xs text-base-content/60 mt-2">
                                                Without a calibration this channel shows raw volts. Choose Linear or
                                                Polynomial to convert to {sensorType.units.filter((u) => u !== RAW_UNIT).join(" / ")}{isStrain
                                                    ? "; Linear offers a bridge helper that works a and b out from the gauge certificate"
                                                    : ""}.
                                            </p>
                                        {/if}

                                        {#if cal.type === "linear"}
                                            {#if isStrain}
                                                <div class="join mt-4" role="group" aria-label="How to enter the strain calibration">
                                                    <button
                                                        type="button"
                                                        class="btn btn-sm join-item {linearModes[key] !== 'bridge' ? 'btn-primary' : ''}"
                                                        aria-pressed={linearModes[key] !== "bridge"}
                                                        onclick={() => setLinearMode(channel, "direct")}
                                                    >Enter a and b</button>
                                                    <button
                                                        type="button"
                                                        class="btn btn-sm join-item {linearModes[key] === 'bridge' ? 'btn-primary' : ''}"
                                                        aria-pressed={linearModes[key] === "bridge"}
                                                        onclick={() => setLinearMode(channel, "bridge")}
                                                    >Bridge helper</button>
                                                </div>
                                            {/if}

                                            {#if isStrain && linearModes[key] === "bridge" && bridgeInputs[key]}
                                                {@const result = bridgeResult(channel)}
                                                <div class="mt-4 space-y-3 rounded-box border border-base-300 p-4">
                                                    <p class="text-xs text-base-content/70">
                                                        µε = factor × 1000 × (raw − zero) / (excitation × gain). The result is
                                                        stored as a plain linear calibration in µε.
                                                    </p>
                                                    <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
                                                        <div class="form-control">
                                                            <label class="label" for="bridge-factor-{channel}">
                                                                <span class="label-text font-medium">Calibration factor (µε per mV/V)</span>
                                                            </label>
                                                            <input id="bridge-factor-{channel}" type="text" inputmode="decimal" placeholder="e.g. 481.26"
                                                                bind:value={bridgeInputs[key].factor} class="input input-bordered w-full focus:input-primary" />
                                                        </div>
                                                        <div class="form-control">
                                                            <label class="label" for="bridge-excitation-{channel}">
                                                                <span class="label-text font-medium">Bridge excitation (V)</span>
                                                            </label>
                                                            <input id="bridge-excitation-{channel}" type="text" inputmode="decimal" placeholder="e.g. 10"
                                                                bind:value={bridgeInputs[key].excitation} class="input input-bordered w-full focus:input-primary" />
                                                        </div>
                                                        <div class="form-control">
                                                            <label class="label" for="bridge-gain-{channel}">
                                                                <span class="label-text font-medium">Amplifier gain</span>
                                                            </label>
                                                            <input id="bridge-gain-{channel}" type="text" inputmode="decimal" placeholder="e.g. 100 or -100"
                                                                bind:value={bridgeInputs[key].gain} class="input input-bordered w-full focus:input-primary" />
                                                        </div>
                                                        <div class="form-control">
                                                            <label class="label" for="bridge-zero-{channel}">
                                                                <span class="label-text font-medium">Zero reading (V, optional)</span>
                                                            </label>
                                                            <input id="bridge-zero-{channel}" type="text" inputmode="decimal" placeholder="0"
                                                                bind:value={bridgeInputs[key].zero} class="input input-bordered w-full focus:input-primary" />
                                                        </div>
                                                    </div>
                                                    <div class="flex flex-wrap items-center justify-between gap-3">
                                                        <p class="text-sm font-mono" role="status">
                                                            {#if result && result.type === "linear"}
                                                                a = {formatValue(result.a)}, b = {formatValue(result.b)} → {previewLine(result)}
                                                            {:else}
                                                                Enter the factor, excitation and a non-zero gain.
                                                            {/if}
                                                        </p>
                                                        <button type="button" class="btn btn-sm btn-primary" disabled={!result}
                                                            onclick={() => applyBridge(channel, index)}>
                                                            Use this calibration
                                                        </button>
                                                    </div>
                                                </div>
                                            {:else}
                                                <div class="grid grid-cols-1 md:grid-cols-2 gap-4 mt-4">
                                                    <div class="form-control">
                                                        <label class="label" for="calibration-linear-a-{channel}">
                                                            <span class="label-text font-medium">Slope a ({cal.unit ?? "unit"} per V)</span>
                                                        </label>
                                                        <input
                                                            id="calibration-linear-a-{channel}"
                                                            type="number"
                                                            step="any"
                                                            value={cal.a}
                                                            oninput={(event) => updateLinearField(channel, index, "a", Number((event.currentTarget as HTMLInputElement).value))}
                                                            class="input input-bordered w-full focus:input-primary"
                                                        />
                                                    </div>
                                                    <div class="form-control">
                                                        <label class="label" for="calibration-linear-b-{channel}">
                                                            <span class="label-text font-medium">Offset b ({cal.unit ?? "unit"})</span>
                                                        </label>
                                                        <input
                                                            id="calibration-linear-b-{channel}"
                                                            type="number"
                                                            step="any"
                                                            value={cal.b}
                                                            oninput={(event) => updateLinearField(channel, index, "b", Number((event.currentTarget as HTMLInputElement).value))}
                                                            class="input input-bordered w-full focus:input-primary"
                                                        />
                                                    </div>
                                                </div>
                                            {/if}
                                        {:else if cal.type === "polynomial"}
                                            <div class="form-control mt-4">
                                                <label class="label" for="calibration-poly-{channel}">
                                                    <span class="label-text font-medium">Coefficients (c0, c1, c2...)</span>
                                                </label>
                                                <input
                                                    id="calibration-poly-{channel}"
                                                    type="text"
                                                    value={coeffInputs[key] ?? cal.coeffs.join(", ")}
                                                    oninput={(event) => updatePolynomialCoeffs(channel, index, (event.currentTarget as HTMLInputElement).value)}
                                                    class="input input-bordered w-full focus:input-primary"
                                                />
                                            </div>
                                        {/if}

                                        <!-- Noise filters: applied only when the data is read (live plot, exports). -->
                                        <div class="mt-4 rounded-box border border-base-300 p-4" aria-label="Channel {channel} noise filters">
                                            <div class="flex flex-wrap items-baseline justify-between gap-2 mb-2">
                                                <h5 class="font-medium text-sm text-base-content">Filters</h5>
                                                <span class="text-xs text-base-content/60">
                                                    {describeFilters(filters) || "None"} · live plot and exports only; the archive keeps the raw data
                                                </span>
                                            </div>
                                            <div class="grid grid-cols-1 sm:grid-cols-3 gap-2">
                                                <label class="label cursor-pointer justify-start gap-2">
                                                    <input id="filter-despike-{channel}" type="checkbox" class="toggle toggle-sm toggle-success"
                                                        checked={filters.despike === true}
                                                        onchange={(event) => setFilter(channel, "despike", (event.currentTarget as HTMLInputElement).checked)} />
                                                    <span class="label-text">Remove spikes</span>
                                                </label>
                                                <label class="label cursor-pointer justify-start gap-2">
                                                    <input id="filter-10hz-{channel}" type="checkbox" class="toggle toggle-sm toggle-success"
                                                        checked={filters.remove_10hz === true}
                                                        onchange={(event) => setFilter(channel, "remove_10hz", (event.currentTarget as HTMLInputElement).checked)} />
                                                    <span class="label-text">Remove 10 Hz</span>
                                                </label>
                                                <label class="label cursor-pointer justify-start gap-2">
                                                    <input id="filter-119hz-{channel}" type="checkbox" class="toggle toggle-sm toggle-success"
                                                        checked={filters.remove_11_9hz === true}
                                                        onchange={(event) => setFilter(channel, "remove_11_9hz", (event.currentTarget as HTMLInputElement).checked)} />
                                                    <span class="label-text">Remove 11.9 Hz</span>
                                                </label>
                                            </div>
                                            <div class="grid grid-cols-1 sm:grid-cols-2 gap-4 mt-2">
                                                <div class="form-control">
                                                    <label class="label" for="filter-highpass-{channel}">
                                                        <span class="label-text">High-pass (Hz, blank = off)</span>
                                                    </label>
                                                    <input id="filter-highpass-{channel}" type="text" inputmode="decimal" placeholder="e.g. 1"
                                                        value={filters.highpass_hz ?? ""}
                                                        onchange={(event) => commitCutoff(channel, "highpass_hz", event.currentTarget as HTMLInputElement)}
                                                        class="input input-bordered input-sm w-full focus:input-primary" />
                                                </div>
                                                <div class="form-control">
                                                    <label class="label" for="filter-lowpass-{channel}">
                                                        <span class="label-text">Low-pass (Hz, blank = off)</span>
                                                    </label>
                                                    <input id="filter-lowpass-{channel}" type="text" inputmode="decimal" placeholder="e.g. 100"
                                                        value={filters.lowpass_hz ?? ""}
                                                        onchange={(event) => commitCutoff(channel, "lowpass_hz", event.currentTarget as HTMLInputElement)}
                                                        class="input input-bordered input-sm w-full focus:input-primary" />
                                                </div>
                                            </div>
                                            {#if plan.skipped.length > 0}
                                                <p class="text-xs text-warning mt-2" role="status">
                                                    Not applied at {formData.sensor_settings.scan_rate_hz} Hz: {plan.skipped.join("; ")}.
                                                </p>
                                            {:else if plan.delaySamples > 0}
                                                <p class="text-xs text-base-content/60 mt-2">
                                                    The live plot shows filtered data {(plan.delaySamples * 1000 / plan.fs).toFixed(1)} ms late (despike window).
                                                </p>
                                            {/if}
                                        </div>
                                    </div>
                                </div>
                            {/each}
                        </div>
                        {#if errors.data_formats || errors.measurement_units || errors.calibration_units}
                            <div class="label">
                                <span class="label-text-alt text-error">
                                    {errors.data_formats || errors.measurement_units || errors.calibration_units}
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
                onclick={requestClose}
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
