/**
 * Per-channel settings and derived values of the live plot page.
 *
 * Pure TypeScript with no Svelte dependency, so it is unit tested directly. The page
 * keeps one {@link "plot/channel-view.svelte"!ChannelView} per enabled channel; these
 * helpers compute what it shows from that state.
 *
 * @module
 */
import type { DataPoint } from './stream';
import { getTriggerWindows, type TriggerCapture, type TriggerSettings } from './trigger';

/**
 * Plot mode of one channel.
 *
 * `free_run` scrolls continuously. `trigger_normal` freezes a capture on each threshold
 * crossing and re-arms after the post-trigger window. `trigger_single` keeps the first
 * capture until the user presses Re-arm.
 */
export type ChannelPlotMode = 'free_run' | 'trigger_normal' | 'trigger_single';

/** Axis settings of one channel, edited in the Mode & Axis panel. */
export interface AxisSettings {
    /** When true the plot scales Y to the data and ignores `yMin` and `yMax`. */
    autoY: boolean;
    /** Lower Y limit when `autoY` is false. */
    yMin: number;
    /** Upper Y limit when `autoY` is false. Kept above `yMin` by `applyAxisLimitInput`. */
    yMax: number;
    /** Width of the X axis in seconds. Above 0. */
    xWindowSec: number;
    /** Mirrors the X axis. */
    invertX: boolean;
    /** Mirrors the Y axis. */
    invertY: boolean;
}

/**
 * Stream counters of one channel shown in Data Statistics: messages dropped because
 * the queue overflowed, gaps (missing time between messages), timeline resets, samples
 * skipped as duplicates, and undecodable payloads.
 */
export interface ChannelStreamStatus {
    dropped: number;
    gaps: number;
    resets: number;
    skipped: number;
    decodeErrors: number;
}

/**
 * Samples the automatic X window aims to show; the window is this count divided by
 * the scan rate.
 */
export const TARGET_SAMPLES_PER_WINDOW = 1000;

/**
 * Live snapshots cover this multiple of the channel's X window, so the plot edge is
 * never empty.
 */
export const SNAPSHOT_OVERSCAN_FACTOR = 1.25;

/** Most channels plotted at once. */
export const MAX_PLOT_CHANNELS = 2;

/**
 * Returns the automatic X window for a scan rate.
 *
 * @param scanRateHz - Scans per second per channel.
 * @returns {@link TARGET_SAMPLES_PER_WINDOW} divided by the rate, in seconds, at least
 *   0.05. Returns 1 when the rate is not a positive finite number.
 *
 * @example
 * ```ts
 * deriveAutoTimeWindowSec(1000);  // 1
 * deriveAutoTimeWindowSec(50000); // 0.05 (0.02 raised to the minimum)
 * ```
 */
export function deriveAutoTimeWindowSec(scanRateHz: number): number {
    if (!Number.isFinite(scanRateHz) || scanRateHz <= 0) {
        return 1;
    }
    return Math.max(0.05, TARGET_SAMPLES_PER_WINDOW / scanRateHz);
}

/**
 * Axis settings a channel starts with: auto Y with limits -1 to 1, the automatic X
 * window, no inversion.
 *
 * @param autoTimeWindowSec - Automatic X window, seconds.
 */
export function defaultAxisSettings(autoTimeWindowSec: number): AxisSettings {
    return {
        autoY: true,
        yMin: -1,
        yMax: 1,
        xWindowSec: autoTimeWindowSec,
        invertX: false,
        invertY: false
    };
}

/**
 * Trigger settings a channel starts with: a rising edge at 0, 40 % pre-trigger and a
 * post-trigger window equal to the automatic X window.
 *
 * @param autoTimeWindowSec - Automatic X window, seconds.
 */
export function defaultTriggerSettings(autoTimeWindowSec: number): TriggerSettings {
    return {
        type: 'rising',
        threshold: 0,
        preTriggerPercent: 40,
        postTriggerWindowSec: autoTimeWindowSec
    };
}

/**
 * Tells whether a mode uses the trigger.
 *
 * @param mode - Plot mode.
 * @returns `true` for `trigger_normal` and `trigger_single`.
 */
export function isTriggerMode(mode: ChannelPlotMode): boolean {
    return mode === 'trigger_normal' || mode === 'trigger_single';
}

/**
 * Chooses up to two channels to plot after a config load.
 *
 * Keeps channels from the previous selection that are still enabled, then fills up to
 * two from `enabledChannels` in config order.
 *
 * @param enabledChannels - `channels_enabled` from the config.
 * @param existingSelection - Selection before the reload, if any.
 * @returns A new set with at most two channels.
 */
export function pickInitialPlotChannels(enabledChannels: number[], existingSelection?: Set<number>): Set<number> {
    const selected = new Set<number>();

    if (existingSelection) {
        for (const channel of enabledChannels) {
            if (existingSelection.has(channel)) {
                selected.add(channel);
                if (selected.size >= MAX_PLOT_CHANNELS) return selected;
            }
        }
    }

    for (const channel of enabledChannels) {
        selected.add(channel);
        if (selected.size >= MAX_PLOT_CHANNELS) break;
    }

    return selected;
}

/**
 * Returns the selection after ticking or unticking a channel.
 *
 * @param selection - Current selection.
 * @param channel - LabJack channel number.
 * @param checked - New checkbox state.
 * @returns A new set, or `null` when the change is refused (adding a third channel).
 */
export function togglePlotSelection(selection: Set<number>, channel: number, checked: boolean): Set<number> | null {
    const next = new Set(selection);
    if (checked) {
        if (next.size >= MAX_PLOT_CHANNELS) return null;
        next.add(channel);
    } else {
        next.delete(channel);
    }
    return next;
}

/**
 * Returns the number of samples each live buffer keeps.
 *
 * The window is the largest of: twice the automatic X window, twice each channel's X
 * window (at least 0.1 s), and each channel's pre plus post trigger window. It is
 * multiplied by the scan rate, plus 10 % and one read of headroom: the actual scan
 * rate can be slightly above the configured one, and gap markers take a slot each.
 * Without it a trigger capture could find the start of its pre-trigger window already
 * trimmed.
 *
 * @param scanRateHz - Configured scan rate.
 * @param scansPerRead - Configured scans per read; values below 1 count as 1.
 * @param autoTimeWindowSec - Automatic X window, seconds.
 * @param channels - Axis and trigger settings of every channel.
 */
export function requiredBufferPoints(
    scanRateHz: number,
    scansPerRead: number,
    autoTimeWindowSec: number,
    channels: Iterable<{ axis: AxisSettings; trigger: TriggerSettings }>
): number {
    let requiredSeconds = autoTimeWindowSec * 2;
    for (const { axis, trigger } of channels) {
        requiredSeconds = Math.max(requiredSeconds, Math.max(0.1, axis.xWindowSec) * 2);
        const windows = getTriggerWindows(trigger);
        requiredSeconds = Math.max(requiredSeconds, windows.preWindowSec + windows.postWindowSec);
    }
    const perRead = Math.max(1, scansPerRead || 1);
    return Math.ceil(scanRateHz * requiredSeconds * 1.1) + perRead;
}

/**
 * Returns how much of a channel's live buffer the plot snapshot keeps, in ms.
 *
 * The X window times {@link SNAPSHOT_OVERSCAN_FACTOR}, at least 0.25 s. In the trigger
 * modes at least the pre-trigger window plus 0.25 s.
 *
 * @param xWindowSec - The channel's X window, seconds.
 * @param mode - The channel's plot mode.
 * @param trigger - The channel's trigger settings.
 */
export function snapshotKeepMs(xWindowSec: number, mode: ChannelPlotMode, trigger: TriggerSettings | undefined): number {
    let keepSeconds = Math.max(0.25, xWindowSec * SNAPSHOT_OVERSCAN_FACTOR);
    if (isTriggerMode(mode) && trigger) {
        const windows = getTriggerWindows(trigger);
        keepSeconds = Math.max(keepSeconds, windows.preWindowSec + 0.25);
    }
    return keepSeconds * 1000;
}

/** Data and trigger props of one channel's `RealTimePlot`, from {@link plotConfigFor}. */
export interface PlotConfig {
    mode: 'frozen' | 'continuous';
    data: DataPoint[];
    frozenData: DataPoint[] | undefined;
    isTriggered: boolean;
    /** Unix ms, 0 when not triggered. */
    triggerTime: number;
    frozenPreWindowSec: number;
    frozenPostWindowSec: number;
    frozenCollecting: boolean;
}

/**
 * Builds the data and trigger props for one channel's `RealTimePlot`.
 *
 * In a trigger mode with a capture it returns mode `"frozen"` with the capture as
 * `frozenData` and its own pre and post windows; `frozenCollecting` is true while the
 * capture is still filling. Otherwise mode `"continuous"` with the windows of the
 * current trigger settings.
 *
 * @param mode - The channel's plot mode.
 * @param liveData - The channel's live snapshot.
 * @param capture - The channel's trigger capture, if any.
 * @param trigger - The channel's trigger settings.
 */
export function plotConfigFor(
    mode: ChannelPlotMode,
    liveData: DataPoint[],
    capture: TriggerCapture | null | undefined,
    trigger: TriggerSettings | undefined
): PlotConfig {
    if (isTriggerMode(mode) && capture) {
        return {
            mode: 'frozen',
            data: liveData,
            frozenData: capture.data,
            isTriggered: true,
            triggerTime: capture.triggerTime,
            frozenPreWindowSec: capture.preWindowSec,
            frozenPostWindowSec: capture.postWindowSec,
            frozenCollecting: !capture.complete
        };
    }

    const { preWindowSec, postWindowSec } = getTriggerWindows(trigger);
    return {
        mode: 'continuous',
        data: liveData,
        frozenData: undefined,
        isTriggered: false,
        triggerTime: 0,
        frozenPreWindowSec: preWindowSec,
        frozenPostWindowSec: postWindowSec,
        frozenCollecting: false
    };
}

/**
 * Text of a channel's status badge.
 *
 * @param mode - The channel's plot mode.
 * @param prebufferReady - The buffer spans the pre-trigger window.
 * @param triggered - A capture is held.
 * @returns `Running`, `Pre-buffering`, `Triggered` or `Armed`.
 */
export function channelStatusLabel(mode: ChannelPlotMode, prebufferReady: boolean, triggered: boolean): string {
    if (mode === 'free_run') return 'Running';
    if (!prebufferReady) return 'Pre-buffering';
    if (triggered) return 'Triggered';
    return 'Armed';
}

/**
 * Average sample rate of a snapshot: points divided by its time span.
 *
 * @param data - Snapshot, sorted by time.
 * @returns Rounded Hz, or 0 for fewer than two points.
 */
export function snapshotRateHz(data: DataPoint[]): number {
    if (data.length <= 1) return 0;
    const latest = data[data.length - 1];
    return Math.round(1000 * (data.length - 1) / (latest?.timestamp - data[0]?.timestamp));
}

/**
 * Text of the stream-problem badge of one channel, or `null` when there is nothing to
 * report.
 *
 * @param channel - LabJack channel number.
 * @param status - The channel's counters.
 * @returns E.g. `Ch3: 2 gaps, 1 resets, 5 dup samples, 1 bad msgs`.
 */
export function streamProblemText(channel: number, status: ChannelStreamStatus | null | undefined): string | null {
    if (!status || !(status.gaps > 0 || status.resets > 0 || status.skipped > 0 || status.decodeErrors > 0)) {
        return null;
    }
    return `Ch${channel}: ${status.gaps} gaps${status.resets > 0 ? `, ${status.resets} resets` : ''}${status.skipped > 0 ? `, ${status.skipped} dup samples` : ''}${status.decodeErrors > 0 ? `, ${status.decodeErrors} bad msgs` : ''}`;
}
