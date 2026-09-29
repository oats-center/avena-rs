/**
 * State of one channel on the live plot page: its settings, trigger state, the live
 * buffer and the snapshot the plot draws.
 *
 * The page keeps one {@link ChannelView} per enabled channel. Settings and trigger
 * state are reactive; the live buffer is not (it changes on every message) and is
 * copied into {@link ChannelView.liveData} at most about 30 times a second.
 *
 * @module
 */
import { applyAxisLimitInput, parseFiniteInput, type AxisLimits } from './axis';
import {
    defaultAxisSettings,
    defaultTriggerSettings,
    isTriggerMode,
    type AxisSettings,
    type ChannelPlotMode,
    type ChannelStreamStatus
} from './channel';
import type { CalibrationSpec } from '../calibration';
import { describeFilters, hasActiveFilters, type ChannelFilterSettings } from '../filter-settings';
import { planFilters, type FilterPlan } from '../filters';
import { LiveChannelFilter } from './live-filter';
import { createLiveChannel, type DataPoint, type LiveChannel } from './stream';
import { hasRequiredPreBuffer, type TriggerCapture, type TriggerSettings } from './trigger';

/** One channel of the plot page. */
export class ChannelView {
    /** LabJack channel number. */
    readonly channel: number;
    /** Plot mode, chosen in the Plot Mode select. */
    mode = $state<ChannelPlotMode>('free_run');
    /** Axis settings. Replaced, never changed in place. */
    axis = $state.raw<AxisSettings>(defaultAxisSettings(1));
    /** Why the last typed axis limit was rejected, or `""`. */
    axisError = $state('');
    /** Trigger settings. Replaced, never changed in place. */
    trigger = $state.raw<TriggerSettings>(defaultTriggerSettings(1));
    /** True while a trigger capture is held. */
    triggered = $state(false);
    /** Time of the last trigger in Unix ms, or 0 when not triggered. */
    triggerTime = $state(0);
    /** True when the buffer spans the pre-trigger window. Set true in free run. */
    prebufferReady = $state(false);
    /**
     * The current trigger capture, if any. `advanceTrigger` extends it in place; the
     * plot reads its data again on each snapshot.
     */
    capture = $state.raw<TriggerCapture | null>(null);
    /** Copy of the newest part of the live buffer, drawn by the plot. */
    liveData = $state.raw<DataPoint[]>([]);
    /** Stream counters shown in Data Statistics; `null` until the first snapshot. */
    streamStatus = $state.raw<ChannelStreamStatus | null>(null);
    /**
     * Rolling live buffer and stream state. Not reactive: changed in place on every
     * message. Sorted by sample time with no duplicates (see `stream.ts`).
     */
    live: LiveChannel = createLiveChannel();
    /**
     * The channel's noise filters, or `null` when none runs at the configured rate.
     * Not reactive; see `live-filter.ts`.
     */
    filter: LiveChannelFilter | null = null;
    /** What runs at the configured rate, for the filter notes; `null` without filters. */
    filterPlan = $state.raw<FilterPlan | null>(null);
    /** Short description of the configured filters, e.g. `despike, −10 Hz`; `""` for none. */
    filterDescription = $state('');
    /** Filtered / Raw switch of the plot. Not saved. */
    showFiltered = $state(true);

    /**
     * @param channel - LabJack channel number.
     * @param autoTimeWindowSec - Automatic X window; the default X window and post
     *   trigger window.
     */
    constructor(channel: number, autoTimeWindowSec: number) {
        this.channel = channel;
        this.axis = defaultAxisSettings(autoTimeWindowSec);
        this.trigger = defaultTriggerSettings(autoTimeWindowSec);
    }

    /**
     * Empties the buffer, stream state, counters, snapshot and trigger capture, so the
     * next data starts a fresh trace.
     */
    resetStream() {
        this.live = createLiveChannel();
        this.filter?.reset();
        this.clearTrigger();
        this.prebufferReady = false;
        this.liveData = [];
        this.streamStatus = null;
    }

    /**
     * Sets up the channel's noise filters from its config.
     *
     * @param settings - The channel's `sensor_settings.filters` entry, if any.
     * @param calibration - The channel's calibration.
     * @param scanRateHz - Configured scan rate, used for the notes; the filter itself
     *   follows the rate of the data.
     */
    configureFilter(settings: ChannelFilterSettings | undefined, calibration: CalibrationSpec, scanRateHz: number) {
        this.filter = null;
        this.filterPlan = null;
        this.filterDescription = '';
        if (!settings || !hasActiveFilters(settings)) return;
        const plan = planFilters(settings, scanRateHz);
        this.filterPlan = plan;
        this.filterDescription = describeFilters(settings);
        if (plan.active) {
            this.filter = new LiveChannelFilter(settings, calibration);
            this.filter.showFiltered = this.showFiltered;
        }
    }

    /**
     * Switches the plot between filtered and raw (calibrated) values. The live buffer
     * is switched in place and the trigger is re-armed, since its level was crossed by
     * the other trace.
     *
     * @param show - `true` for filtered.
     */
    setShowFiltered(show: boolean) {
        if (this.showFiltered === show) return;
        this.showFiltered = show;
        this.filter?.setShowFiltered(show, this.live.buffer);
        this.clearTrigger();
    }

    /** Arms the channel again: clears the trigger flag, time and capture. */
    clearTrigger() {
        this.triggered = false;
        this.triggerTime = 0;
        this.capture = null;
    }

    /**
     * Stores a new or extended capture and the trigger flags derived from it.
     *
     * @param capture - The capture.
     */
    setCapture(capture: TriggerCapture) {
        this.capture = capture;
        this.triggered = true;
        this.triggerTime = capture.triggerTime;
    }

    /**
     * Changes the plot mode.
     *
     * Switching between the two trigger modes keeps the current capture; any other
     * change clears it. The pre-buffer flag is recomputed from the live buffer.
     *
     * @param next - Mode chosen in the Plot Mode select.
     * @returns `false` when the mode did not change.
     */
    setMode(next: ChannelPlotMode): boolean {
        const current = this.mode;
        if (current === next) return false;
        if (!isTriggerMode(next) || !isTriggerMode(current)) {
            this.clearTrigger();
        }
        this.prebufferReady = isTriggerMode(next) ? hasRequiredPreBuffer(this.live.buffer, this.trigger) : true;
        this.mode = next;
        return true;
    }

    /**
     * Merges changes into the axis settings and clears the axis input message, which
     * described an earlier rejected value.
     *
     * @param updates - Fields to change.
     */
    updateAxis(updates: Partial<AxisSettings>) {
        this.axisError = '';
        this.axis = { ...this.axis, ...updates };
    }

    /**
     * Applies a typed Y Min, Y Max or X Window.
     *
     * Any finite number is accepted, including 0, as long as Y Min stays below Y Max
     * and the X window is above 0 (see `applyAxisLimitInput`). Otherwise the reason is
     * kept in {@link axisError}.
     *
     * @param field - Limit being edited.
     * @param text - Text typed in the input.
     * @returns `null` when applied, else the value to put back into the input.
     */
    commitAxisLimit(field: keyof AxisLimits, text: string): string | null {
        const current = this.axis;
        const result = applyAxisLimitInput(current, field, text);
        if (result.ok) {
            this.updateAxis(result.limits);
            return null;
        }
        this.axisError = result.error;
        return String(current[field]);
    }

    /**
     * Applies a typed trigger threshold or post-trigger window. Any finite threshold
     * is accepted, including 0; the post window must be at least 0.01 s.
     *
     * @param field - Setting being edited.
     * @param text - Text typed in the input.
     * @returns `null` when applied, else the value to put back into the input.
     */
    commitTriggerNumber(field: 'threshold' | 'postTriggerWindowSec', text: string): string | null {
        const value = parseFiniteInput(text);
        if (value === null || (field === 'postTriggerWindowSec' && value < 0.01)) {
            return String(this.trigger[field]);
        }
        this.trigger = { ...this.trigger, [field]: value };
        return null;
    }

    /**
     * Sets the trigger edge.
     *
     * @param type - `rising` or `falling`.
     */
    setTriggerEdge(type: TriggerSettings['type']) {
        this.trigger = { ...this.trigger, type };
    }

    /**
     * Sets the pre-trigger share from typed text: an integer clamped to 0 to 95, and 0
     * for anything that is not a number.
     *
     * @param text - Text typed in the input.
     */
    setPreTriggerPercent(text: string) {
        const raw = parseInt(text, 10) || 0;
        this.trigger = { ...this.trigger, preTriggerPercent: Math.min(95, Math.max(0, raw)) };
    }
}
