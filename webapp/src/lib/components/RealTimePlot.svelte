<script lang="ts">
    import { onMount } from "svelte";
    import type { DataPoint } from "$lib/plot/stream";
    import {
        computeValueRange,
        downsampleMinMax,
        formatTimeTick,
        latestFinitePoint,
        selectTimeWindow,
        timeAxisTicks
    } from "$lib/plot/render";
    
    /** Component props. See the `@component` block below for each one. */
    interface Props {
        /** Live buffer of samples for this channel. */
        data: DataPoint[];
        /** Unit of the plotted values, for the y axis, badges and threshold. */
        unit: string;
        /**
         * True when the values are calibrated from raw volts; the raw volts of the
         * latest sample are then shown next to Latest.
         */
        calibrated?: boolean;
        /** Width of the continuous time axis, in seconds. */
        timeWindow: number;
        /** Whether a trigger has fired. */
        isTriggered: boolean;
        /** Time the trigger fired, Unix epoch ms. `0` means none. */
        triggerTime: number;
        /** `continuous` scrolls with the live buffer; `frozen` shows the captured trigger window. */
        mode: 'continuous' | 'frozen';
        /** Samples captured around the trigger, shown instead of `data` in frozen mode. */
        frozenData?: DataPoint[];
        /** Seconds shown before the trigger in frozen mode. */
        frozenPreWindowSec?: number;
        /** Seconds shown after the trigger in frozen mode. */
        frozenPostWindowSec?: number;
        /** True while post-trigger samples are still being collected. */
        frozenCollecting?: boolean;
        /** Draws the trigger threshold line and LEVEL badge. */
        showTriggerThreshold?: boolean;
        /** Trigger level, in `unit`. */
        triggerThreshold?: number;
        /** Shows the PREBUFFERING badge. */
        prebuffering?: boolean;
        /** Fits the y axis to the data when true; otherwise uses `yMin`/`yMax`. */
        yAutoScale?: boolean;
        /** Lower y limit when `yAutoScale` is false. */
        yMin?: number;
        /** Upper y limit when `yAutoScale` is false. */
        yMax?: number;
        /** Mirrors the time axis. */
        invertX?: boolean;
        /** Mirrors the value axis. */
        invertY?: boolean;
    }
    
    let {
        data,
        unit,
        calibrated = false,
        timeWindow,
        isTriggered,
        triggerTime,
        mode,
        frozenData,
        frozenPreWindowSec = timeWindow,
        frozenPostWindowSec = timeWindow,
        frozenCollecting = false,
        showTriggerThreshold = false,
        triggerThreshold,
        prebuffering = false,
        yAutoScale = true,
        yMin = -1,
        yMax = 1,
        invertX = false,
        invertY = false
    }: Props = $props();
    
    let canvas: HTMLCanvasElement;
    /** 2D context, scaled by `devicePixelRatio` so drawing uses CSS pixels. Plain `let`, not reactive. */
    let ctx: CanvasRenderingContext2D;
    /** Id of the pending `requestAnimationFrame`, cancelled on unmount. */
    let animationFrame = 0;
    /** True while a render is queued, so many prop changes in one frame draw once. */
    let renderQueued = false;
    /** Canvas size in CSS pixels, set by {@link resizeCanvas}. */
    let plotWidth = 0;
    let plotHeight = 0;
    /**
     * Space around the plot area for tick labels and axis titles, in CSS pixels.
     * `left` is recomputed every frame by {@link layoutLeftMargin} from the widest
     * value label, so the labels and the y-axis title never overlap.
     */
    let margin = { top: 30, right: 40, bottom: 50, left: 80 };
    /** Font of the tick labels. */
    const TICK_FONT = '13px Inter, system-ui, sans-serif';
    /** Font of the axis titles. */
    const TITLE_FONT = '15px Inter, system-ui, sans-serif';
    /** Gap between the canvas edge and the rotated y-axis title, CSS pixels. */
    const EDGE_PAD = 6;
    /** Height of a line of {@link TITLE_FONT}; the width the rotated title takes up. */
    const TITLE_BAND = 18;
    /** Gap between the y-axis title and the value labels, and between labels and axis. */
    const LABEL_GAP = 8;
    /** Smallest space kept between two time labels, CSS pixels. */
    const MIN_TIME_TICK_SPACING = 70;
    /** Running min/max of every value seen in continuous autoscale. Only grows until reset. */
    let stickyAutoExtrema: { min: number; max: number } | null = null;
    /** Number of horizontal grid intervals on the y axis. */
    const Y_GRID_DIVISIONS = 8;
    /** Smallest autoscale grid step, in `unit`. Keeps a flat signal from collapsing the axis. */
    const MIN_AUTO_Y_INTERVAL = 0.01;
    
    /** Trace color (one trace per plot). */
    const TRACE_COLOR = '#3B82F6';
    
    /**
     * Matches the canvas backing store to its on-screen size and the device pixel ratio.
     *
     * Sets `plotWidth`/`plotHeight` in CSS pixels and scales the context by
     * `devicePixelRatio`, so all later drawing uses CSS pixels and stays sharp on
     * high-DPI screens. Does nothing before the canvas is bound.
     */
    function resizeCanvas() {
        if (!canvas) return;
        
        const rect = canvas.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        
        canvas.width = rect.width * dpr;
        canvas.height = rect.height * dpr;
        
        plotWidth = rect.width;
        plotHeight = rect.height;
        
        ctx = canvas.getContext('2d')!;
        ctx.scale(dpr, dpr);
        
        // Set canvas size in CSS
        canvas.style.width = rect.width + 'px';
        canvas.style.height = rect.height + 'px';
    }
    
    /**
     * Draws the background grid: a vertical line at each time tick and 9 horizontal
     * lines (8 value intervals) across the plot area.
     *
     * @param timeTicks - Time ticks of this frame, from {@link getTimeTicks}.
     */
    function drawGrid(timeTicks: { x: number; label: string }[]) {
        if (!ctx) return;
        
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.1)';
        ctx.lineWidth = 1;
        
        // Vertical grid lines (time)
        for (const { x } of timeTicks) {
            ctx.beginPath();
            ctx.moveTo(x, margin.top);
            ctx.lineTo(x, plotHeight - margin.bottom);
            ctx.stroke();
        }
        
        // Horizontal grid lines (value)
        for (let i = 0; i <= 8; i++) {
            const y = margin.top + (i / 8) * (plotHeight - margin.top - margin.bottom);
            ctx.beginPath();
            ctx.moveTo(margin.left, y);
            ctx.lineTo(plotWidth - margin.right, y);
            ctx.stroke();
        }
    }
    
    /** Draws the x axis along the bottom and the y axis along the left of the plot area. */
    function drawAxes() {
        if (!ctx) return;
        
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.3)';
        ctx.lineWidth = 2;
        
        // X-axis (time)
        ctx.beginPath();
        ctx.moveTo(margin.left, plotHeight - margin.bottom);
        ctx.lineTo(plotWidth - margin.right, plotHeight - margin.bottom);
        ctx.stroke();
        
        // Y-axis (value)
        ctx.beginPath();
        ctx.moveTo(margin.left, margin.top);
        ctx.lineTo(margin.left, plotHeight - margin.bottom);
        ctx.stroke();
    }
    
    /**
     * Rounds a raw grid step up to 1, 2, 5 or 10 times a power of ten.
     *
     * @param value - Raw step, in `unit`.
     * @returns The rounded step, never below `MIN_AUTO_Y_INTERVAL`. Non-finite or
     *   non-positive input returns `MIN_AUTO_Y_INTERVAL`.
     */
    function niceStep(value: number): number {
        if (!Number.isFinite(value) || value <= 0) return MIN_AUTO_Y_INTERVAL;
        const exponent = Math.floor(Math.log10(value));
        const fraction = value / Math.pow(10, exponent);

        let niceFraction = 10;
        if (fraction <= 1) niceFraction = 1;
        else if (fraction <= 2) niceFraction = 2;
        else if (fraction <= 5) niceFraction = 5;

        return Math.max(MIN_AUTO_Y_INTERVAL, niceFraction * Math.pow(10, exponent));
    }

    /**
     * Turns data extrema into a y range with round grid lines.
     *
     * Pads the observed span by 20% (at least `MIN_AUTO_Y_INTERVAL` per division),
     * picks a {@link niceStep} for `Y_GRID_DIVISIONS` divisions, and snaps the lower
     * edge to a multiple of that step around the midpoint. Values are rounded to six
     * decimals to hide floating point noise in the labels.
     *
     * @param extrema - Data min and max, in `unit`.
     * @returns `low` and `high` edges of the axis and the grid `step`.
     */
    function normalizeAutoDisplayRange(extrema: { min: number; max: number }): { low: number; high: number; step: number } {
        // Fixed number of divisions: choose a round step that covers the padded span, then
        // center that span on the data and snap its lower edge to the step so ticks land
        // on round values.
        const midpoint = (extrema.min + extrema.max) / 2;
        const observedSpan = Math.max(extrema.max - extrema.min, MIN_AUTO_Y_INTERVAL * Y_GRID_DIVISIONS);
        const paddedSpan = Math.max(
            MIN_AUTO_Y_INTERVAL * Y_GRID_DIVISIONS,
            observedSpan * 1.2
        );
        const step = niceStep(paddedSpan / Y_GRID_DIVISIONS);
        const totalSpan = step * Y_GRID_DIVISIONS;
        const snappedLow = Math.floor((midpoint - (totalSpan / 2)) / step) * step;
        const low = Number(snappedLow.toFixed(6));
        const high = Number((low + totalSpan).toFixed(6));
        return { low, high, step };
    }

    /**
     * Formats a y tick label with enough decimals to tell neighboring ticks apart.
     *
     * @param value - Tick value, in `unit`.
     * @param step - Distance between ticks. Steps of 1 or more get 2 decimals; smaller
     *   steps get between 2 and 6.
     * @returns The formatted number, without unit.
     */
    function formatAxisValue(value: number, step: number): string {
        const decimals = step >= 1
            ? 2
            : Math.min(6, Math.max(2, Math.ceil(-Math.log10(step))));
        return value.toFixed(decimals);
    }

    /**
     * Returns the y range to draw.
     *
     * - `yAutoScale` off: `yMin`/`yMax` (falling back to -1 and 1), with `high` forced
     *   above `low`.
     * - Frozen mode: the extrema of `points`, the whole capture so far. While the capture
     *   is collecting the range can widen as samples arrive, so every captured sample is
     *   inside the axis.
     * - Continuous mode: the union of `points` and every earlier call, kept in
     *   `stickyAutoExtrema`, so the axis grows but does not shrink or jitter.
     *
     * Autoscaled ranges go through {@link normalizeAutoDisplayRange}.
     *
     * @param points - Samples the range should cover.
     * @returns `{ low, high }` in `unit`, or `null` if autoscaling has no data yet.
     *
     * @remarks Side effect: in continuous autoscale this widens `stickyAutoExtrema`.
     */
    function getDisplayRange(points: DataPoint[]): { low: number; high: number } | null {
        if (!yAutoScale) {
            const low = Number.isFinite(yMin) ? yMin : -1;
            let high = Number.isFinite(yMax) ? yMax : 1;
            if (high <= low) high = low + 0.001;
            return { low, high };
        }

        if (mode === 'frozen') {
            const frozenExtrema = computeValueRange(points);
            if (!frozenExtrema) return null;
            const { low, high } = normalizeAutoDisplayRange(frozenExtrema);
            return { low, high };
        }

        const currentExtrema = computeValueRange(points);
        if (!currentExtrema) {
            if (!stickyAutoExtrema) return null;
            const { low, high } = normalizeAutoDisplayRange(stickyAutoExtrema);
            return { low, high };
        }

        // Merge into the running extrema so the axis only widens; otherwise every new
        // batch would rescale the axis and the trace would jump.
        stickyAutoExtrema = stickyAutoExtrema
            ? {
                min: Math.min(stickyAutoExtrema.min, currentExtrema.min),
                max: Math.max(stickyAutoExtrema.max, currentExtrema.max)
            }
            : currentExtrema;

        const { low, high } = normalizeAutoDisplayRange(stickyAutoExtrema);
        return { low, high };
    }

    /**
     * Converts a value to a canvas y coordinate inside the plot area.
     *
     * @param value - Value in `unit`.
     * @param range - Axis edges from {@link getDisplayRange}.
     * @returns Y in CSS pixels. `range.high` maps to the top unless `invertY`. Values
     *   outside the range map outside the plot area and are not clamped.
     */
    function mapValueToY(value: number, range: { low: number; high: number }): number {
        const span = range.high - range.low;
        if (span <= 0) return margin.top;
        const normalized = (value - range.low) / span;
        const vertical = invertY ? normalized : (1 - normalized);
        return margin.top + vertical * (plotHeight - margin.top - margin.bottom);
    }

    /**
     * Returns the frozen-mode window around the trigger, in seconds.
     *
     * @returns `pre` and `post` seconds, each at least 0.01. Missing or zero props
     *   become 0.01.
     */
    function getFrozenWindow() {
        const pre = Math.max(0.01, frozenPreWindowSec || 0.01);
        const post = Math.max(0.01, frozenPostWindowSec || 0.01);
        return { pre, post };
    }

    /**
     * Converts a time offset to a canvas x coordinate inside the plot area.
     *
     * The meaning of `timeSincePoint` depends on the mode:
     * - Frozen and triggered: seconds relative to the trigger (negative before it).
     *   `-pre` maps to the left edge and `+post` to the right, or mirrored with `invertX`.
     * - Otherwise: seconds before the reference time (the sample's age). Age 0 maps to
     *   the right edge and `timeWindow` to the left, or mirrored with `invertX`.
     *
     * @param timeSincePoint - Offset in seconds, as described above.
     * @returns X in CSS pixels. Not clamped to the plot area.
     */
    function mapTimeToX(timeSincePoint: number): number {
        const width = plotWidth - margin.left - margin.right;
        if (mode === 'frozen' && isTriggered) {
            const { pre, post } = getFrozenWindow();
            // Shift [-pre, +post] to [0, pre + post], then scale to [0, 1].
            const normalizedTime = (timeSincePoint + pre) / (pre + post);
            return invertX
                ? (plotWidth - margin.right) - normalizedTime * width
                : margin.left + normalizedTime * width;
        }

        // Age 0 (newest) sits at the right edge by default; older samples move left.
        const normalizedTime = timeSincePoint / timeWindow;
        return invertX
            ? margin.left + normalizedTime * width
            : (plotWidth - margin.right) - normalizedTime * width;
    }

    /**
     * Returns the value tick labels for a y range, top to bottom: 9 labels for 8 grid
     * intervals. With no data (`range` null) they show a fixed -10 to 10 scale.
     *
     * @param range - Y range of this frame, from {@link getDisplayRange}.
     * @returns The labels, top grid line first.
     */
    function getValueTickLabels(range: { low: number; high: number } | null): string[] {
        const labels: string[] = [];
        for (let i = 0; i <= Y_GRID_DIVISIONS; i++) {
            const ratio = i / Y_GRID_DIVISIONS;
            if (range) {
                const span = range.high - range.low;
                const value = invertY ? range.low + ratio * span : range.high - ratio * span;
                labels.push(formatAxisValue(value, span / Y_GRID_DIVISIONS));
            } else {
                labels.push((10 - ratio * 20).toFixed(1));
            }
        }
        return labels;
    }

    /**
     * Sets `margin.left` so that, from the left edge, there is room for the rotated
     * y-axis title, a gap, the widest value label and another gap before the axis.
     *
     * @param valueLabels - Labels from {@link getValueTickLabels}.
     */
    function layoutLeftMargin(valueLabels: string[]) {
        if (!ctx) return;
        ctx.font = TICK_FONT;
        let widest = 0;
        for (const label of valueLabels) {
            widest = Math.max(widest, ctx.measureText(label).width);
        }
        margin.left = Math.ceil(EDGE_PAD + TITLE_BAND + LABEL_GAP + widest + LABEL_GAP);
    }

    /**
     * Returns the time range the x axis shows, in seconds on the axis: `[-timeWindow, 0]`
     * in continuous mode (0 is the newest sample), `[-pre, +post]` in frozen mode (0 is
     * the trigger).
     */
    function getTimeAxisRange(): { start: number; end: number } {
        if (mode === 'frozen' && isTriggered) {
            const { pre, post } = getFrozenWindow();
            return { start: -pre, end: post };
        }
        return { start: -timeWindow, end: 0 };
    }

    /**
     * Returns the time ticks of this frame: round steps (1, 2 or 5 times a power of
     * ten) anchored at 0, spaced at least {@link MIN_TIME_TICK_SPACING} px apart. All
     * labels use one unit, given in the axis title.
     *
     * @returns Each tick's x in CSS pixels and its label, plus the label unit.
     */
    function getTimeTicks(): { ticks: { x: number; label: string }[]; unit: 'ms' | 's' } {
        const { start, end } = getTimeAxisRange();
        const width = Math.max(0, plotWidth - margin.left - margin.right);
        const maxTicks = Math.max(2, Math.floor(width / MIN_TIME_TICK_SPACING) + 1);
        const axis = timeAxisTicks(start, end, maxTicks);
        const frozen = mode === 'frozen' && isTriggered;
        const ticks = axis.values.map((value) => ({
            // Continuous mode maps a sample's age; an axis value of -2 s is age 2 s.
            x: mapTimeToX(frozen ? value : -value),
            label: formatTimeTick(value, axis.step, axis.unit)
        }));
        return { ticks, unit: axis.unit };
    }

    /**
     * Draws the time tick labels, value tick labels and both axis titles.
     *
     * Time labels sit under the grid lines from {@link getTimeTicks}. Value labels are
     * right-aligned {@link LABEL_GAP} px left of the axis, and the rotated y-axis title
     * sits in its own band at the left edge (see {@link layoutLeftMargin}).
     *
     * @param timeTicks - Time ticks of this frame.
     * @param timeUnit - Unit of the time labels.
     * @param valueLabels - Value labels of this frame, top to bottom.
     */
    function drawLabels(
        timeTicks: { x: number; label: string }[],
        timeUnit: 'ms' | 's',
        valueLabels: string[]
    ) {
        if (!ctx) return;
        
        ctx.fillStyle = 'rgba(255, 255, 255, 0.7)';
        ctx.font = TICK_FONT;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'top';
        
        for (const { x, label } of timeTicks) {
            ctx.fillText(label, x, plotHeight - margin.bottom + 5);
        }
        
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        valueLabels.forEach((label, i) => {
            const y = margin.top + (i / Y_GRID_DIVISIONS) * (plotHeight - margin.top - margin.bottom);
            ctx.fillText(label, margin.left - LABEL_GAP, y);
        });
        
        // Axis titles
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.font = TITLE_FONT;
        const timeTitle = mode === 'frozen' && isTriggered ? 'Time from trigger' : 'Time';
        ctx.fillText(`${timeTitle} (${timeUnit})`, margin.left + (plotWidth - margin.left - margin.right) / 2, plotHeight - 5);
        
        ctx.save();
        ctx.translate(EDGE_PAD + TITLE_BAND / 2, margin.top + (plotHeight - margin.top - margin.bottom) / 2);
        ctx.rotate(-Math.PI / 2);
        ctx.textBaseline = 'middle';
        ctx.fillText(`Value (${unit})`, 0, 0);
        ctx.restore();
    }
    
    /**
     * Draws a dashed red vertical line at the trigger time.
     *
     * In frozen mode the trigger is always at offset 0. In continuous mode its x
     * position moves left as the reference time advances. Skipped when not triggered,
     * when `triggerTime` is 0, or when the line falls outside the plot area.
     */
    function drawTriggerLine() {
        if (!ctx || !isTriggered || triggerTime === 0) return;
        
        let x: number;
        
        if (mode === 'frozen') {
            x = mapTimeToX(0);
        } else {
            const referenceTime = getContinuousReferenceTime(getDisplayData());
            const timeSinceTrigger = (referenceTime - triggerTime) / 1000;
            x = mapTimeToX(timeSinceTrigger);
        }
        
        if (x >= margin.left && x <= plotWidth - margin.right) {
            ctx.strokeStyle = 'rgba(239, 68, 68, 0.8)';
            ctx.lineWidth = 2;
            ctx.setLineDash([5, 5]);
            ctx.beginPath();
            ctx.moveTo(x, margin.top);
            ctx.lineTo(x, plotHeight - margin.bottom);
            ctx.stroke();
            ctx.setLineDash([]);
        }
    }

    /**
     * Draws a dashed amber horizontal line at `triggerThreshold` with a `Trig` label.
     *
     * Skipped unless `showTriggerThreshold` is set and the threshold is a number inside
     * the current y range.
     *
     * @param range - Y range of this frame, from {@link getDisplayRange}.
     */
    function drawThresholdLine(range: { low: number; high: number } | null) {
        if (!ctx || !showTriggerThreshold || typeof triggerThreshold !== 'number' || Number.isNaN(triggerThreshold)) {
            return;
        }

        if (!range) return;
        if (triggerThreshold < range.low || triggerThreshold > range.high) return;

        const y = mapValueToY(triggerThreshold, range);
        ctx.strokeStyle = 'rgba(255, 193, 7, 0.9)';
        ctx.lineWidth = 1.5;
        ctx.setLineDash([4, 4]);
        ctx.beginPath();
        ctx.moveTo(margin.left, y);
        ctx.lineTo(plotWidth - margin.right, y);
        ctx.stroke();
        ctx.setLineDash([]);

        ctx.fillStyle = 'rgba(255, 193, 7, 0.9)';
        ctx.font = '11px Inter, system-ui, sans-serif';
        ctx.textAlign = 'left';
        ctx.textBaseline = 'bottom';
        ctx.fillText(`Trig ${triggerThreshold.toFixed(3)} ${unit}`, margin.left + 6, y - 4);
    }

    /**
     * Draws a small rounded label, right-aligned to `x`.
     *
     * @param text - Label text.
     * @param x - Right edge of the badge, CSS pixels.
     * @param y - Top edge of the badge, CSS pixels. The badge is 18 px tall.
     * @param fill - CSS fill color.
     * @param stroke - CSS border color.
     */
    function drawBadge(text: string, x: number, y: number, fill: string, stroke: string) {
        if (!ctx) return;
        ctx.save();
        ctx.font = '11px Inter, system-ui, sans-serif';
        const width = ctx.measureText(text).width + 12;
        const height = 18;
        ctx.fillStyle = fill;
        ctx.strokeStyle = stroke;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.roundRect(x - width, y, width, height, 6);
        ctx.fill();
        ctx.stroke();
        ctx.fillStyle = 'rgba(255, 255, 255, 0.95)';
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        ctx.fillText(text, x - 6, y + height / 2);
        ctx.restore();
    }

    /**
     * Stacks status badges in the top right corner of the plot area: LEVEL (threshold
     * shown), PREBUFFERING, and FROZEN or COLLECTING (frozen mode after a trigger).
     */
    function drawCanvasBadges() {
        if (!ctx) return;

        let top = margin.top + 6;
        const right = plotWidth - margin.right - 6;

        if (showTriggerThreshold && typeof triggerThreshold === 'number' && Number.isFinite(triggerThreshold)) {
            drawBadge(`LEVEL ${triggerThreshold.toFixed(3)} ${unit}`, right, top, 'rgba(234, 179, 8, 0.18)', 'rgba(234, 179, 8, 0.8)');
            top += 22;
        }

        if (prebuffering) {
            drawBadge('PREBUFFERING', right, top, 'rgba(59, 130, 246, 0.18)', 'rgba(59, 130, 246, 0.8)');
            top += 22;
        }

        if (mode === 'frozen' && isTriggered) {
            drawBadge(frozenCollecting ? 'COLLECTING' : 'FROZEN', right, top, 'rgba(255, 193, 7, 0.18)', 'rgba(255, 193, 7, 0.8)');
        }
    }

    /**
     * Returns the time, in Unix epoch ms, that sits at the "0 s" edge in continuous mode.
     *
     * This is the newest sample's own time, so the time axis is sample time from the
     * box and does not depend on the browser clock or on network delay (the lag badge
     * shows that). The plot is redrawn when new samples arrive, so the trace moves in
     * steps of one message.
     *
     * @param dataToPlot - Samples sorted by time; the last one is the newest.
     * @returns Reference time in Unix epoch ms, or `Date.now()` with no samples.
     */
    function getContinuousReferenceTime(dataToPlot: DataPoint[]): number {
        const latestTimestamp = dataToPlot[dataToPlot.length - 1]?.timestamp;
        if (typeof latestTimestamp !== 'number' || !Number.isFinite(latestTimestamp)) {
            return Date.now();
        }
        return latestTimestamp;
    }

    /**
     * Returns the samples being shown: `frozenData` in frozen mode when set, else `data`.
     */
    function getDisplayData(): DataPoint[] {
        return mode === 'frozen' && frozenData ? frozenData : data;
    }

    /**
     * Returns the sample at the plot's "t = 0" position, used for the source clock and
     * lag badges under the plot.
     *
     * @param points - Samples being shown.
     * @returns In frozen mode after a trigger, the sample closest to `triggerTime`;
     *   otherwise the last sample. `null` for an empty list.
     */
    function getZeroTimePoint(points: DataPoint[]): DataPoint | null {
        if (!points || points.length === 0) return null;
        if (!(mode === 'frozen' && isTriggered && triggerTime > 0)) {
            return points[points.length - 1];
        }

        let nearest = points[0];
        let nearestDelta = Math.abs(points[0].timestamp - triggerTime);
        for (let i = 1; i < points.length; i++) {
            const candidate = points[i];
            const delta = Math.abs(candidate.timestamp - triggerTime);
            if (delta < nearestDelta) {
                nearest = candidate;
                nearestDelta = delta;
            }
        }
        return nearest;
    }

    /**
     * Formats a Unix epoch ms time as a local wall-clock time string.
     *
     * @param timestampMs - Unix epoch milliseconds.
     * @returns The browser's `toLocaleTimeString()` output.
     */
    function formatSourceClock(timestampMs: number): string {
        return new Date(timestampMs).toLocaleTimeString();
    }

    /**
     * Formats a delay for the lag badge, picking ms, s, m or h by size.
     *
     * @param ms - Delay in milliseconds.
     * @returns The formatted delay, or `--` for negative or non-finite input.
     */
    function formatLag(ms: number): string {
        if (!Number.isFinite(ms) || ms < 0) return '--';
        if (ms < 1000) return `${Math.round(ms)}ms`;
        if (ms < 60000) return `${(ms / 1000).toFixed(2)}s`;
        if (ms < 3600000) return `${(ms / 60000).toFixed(1)}m`;
        return `${(ms / 3600000).toFixed(2)}h`;
    }
    
    
    /**
     * Returns the samples that fall inside the visible time window.
     *
     * Frozen mode after a trigger keeps `[triggerTime - pre, triggerTime + post]`, which
     * is exactly the capture. Otherwise keeps the `timeWindow` seconds ending at
     * `referenceTime`, plus the nearest sample outside each edge so the line runs to the
     * border (where the canvas clips it).
     *
     * @param dataToPlot - Samples sorted by time.
     * @param referenceTime - Right edge of the continuous window, Unix epoch ms.
     * @returns A new array.
     */
    function getVisiblePoints(dataToPlot: DataPoint[], referenceTime: number): DataPoint[] {
        if (dataToPlot.length === 0) return dataToPlot;

        if (mode === 'frozen' && isTriggered) {
            const { pre, post } = getFrozenWindow();
            return selectTimeWindow(dataToPlot, triggerTime - pre * 1000, triggerTime + post * 1000);
        }

        return selectTimeWindow(dataToPlot, referenceTime - timeWindow * 1000, referenceTime, true);
    }

    /**
     * Counts the samples with a value inside the visible time window (the same window
     * {@link getVisiblePoints} uses, without the neighbors past each edge and without
     * gap markers).
     *
     * @param dataToPlot - Samples being shown, sorted by time.
     * @returns Number of samples on screen.
     */
    function countVisibleSamples(dataToPlot: DataPoint[]): number {
        if (dataToPlot.length === 0) return 0;
        let points: DataPoint[];
        if (mode === 'frozen' && isTriggered) {
            const { pre, post } = getFrozenWindow();
            points = selectTimeWindow(dataToPlot, triggerTime - pre * 1000, triggerTime + post * 1000);
        } else {
            const referenceTime = getContinuousReferenceTime(dataToPlot);
            points = selectTimeWindow(dataToPlot, referenceTime - timeWindow * 1000, referenceTime);
        }
        let count = 0;
        for (const point of points) {
            if (Number.isFinite(point.value)) count++;
        }
        return count;
    }

    /**
     * Draws the trace for the visible samples, clipped to the plot area.
     *
     * Downsamples to a min/max pair per pixel column (spikes stay visible, gaps are
     * kept) and draws one path. The path is broken at every `NaN` value (a missing
     * sample or a gap in the stream) and wherever time goes backward, so gaps are never
     * bridged. Everything is clipped to the plot rectangle, so values outside a fixed y
     * range, or points just past the time window, cannot draw over the axes or labels.
     *
     * @param visibleData - Samples to draw, sorted by time (see {@link getVisiblePoints}).
     * @param range - Y range of this frame.
     * @param referenceTime - Right edge of the continuous window, Unix epoch ms.
     * @param color - CSS stroke color.
     */
    function drawDataLine(
        visibleData: DataPoint[],
        range: { low: number; high: number },
        referenceTime: number,
        color: string
    ) {
        if (!ctx || visibleData.length < 1) return;

        // Buffers are kept sorted by the page; sort defensively if they are not.
        let isMonotonic = true;
        for (let i = 1; i < visibleData.length; i++) {
            if (visibleData[i].timestamp < visibleData[i - 1].timestamp) {
                isMonotonic = false;
                break;
            }
        }
        const orderedData = isMonotonic
            ? visibleData
            : [...visibleData].sort((a, b) => a.timestamp - b.timestamp);
        // About two points (min and max) per pixel column, so spikes stay visible at any
        // sample rate; NaN gaps are kept.
        const sampledData = downsampleMinMax(
            orderedData,
            Math.max(16, Math.floor(plotWidth - margin.left - margin.right))
        );

        ctx.save();
        ctx.beginPath();
        ctx.rect(
            margin.left,
            margin.top,
            Math.max(0, plotWidth - margin.left - margin.right),
            Math.max(0, plotHeight - margin.top - margin.bottom)
        );
        ctx.clip();

        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;
        ctx.lineCap = 'round';
        ctx.lineJoin = 'round';
        ctx.beginPath();

        let hasActiveSegment = false;
        let previousTimestamp = Number.NaN;

        for (const point of sampledData) {
            // A NaN value is a missing sample or a gap in the stream: end the line here so
            // the gap is not bridged.
            if (!point || !Number.isFinite(point.timestamp) || !Number.isFinite(point.value)) {
                hasActiveSegment = false;
                continue;
            }

            const timeSincePoint = mode === 'frozen' && isTriggered
                ? (point.timestamp - triggerTime) / 1000
                : (referenceTime - point.timestamp) / 1000;
            const x = mapTimeToX(timeSincePoint);
            const y = mapValueToY(point.value, range);

            const nonMonotonicTime = Number.isFinite(previousTimestamp) && point.timestamp <= previousTimestamp;
            if (!hasActiveSegment || nonMonotonicTime) {
                ctx.moveTo(x, y);
                hasActiveSegment = true;
            } else {
                ctx.lineTo(x, y);
            }
            previousTimestamp = point.timestamp;
        }

        ctx.stroke();
        ctx.restore();
    }
    
    /**
     * Redraws the whole canvas: background, grid, axes, trace, threshold line, trigger
     * line, labels and badges, or "No Data Available" when there are no samples.
     * Does nothing until the canvas and context exist.
     */
    function render() {
        if (!ctx || !canvas) {
            return;
        }
        
        // Clear canvas
        ctx.clearRect(0, 0, plotWidth, plotHeight);
        
        // Draw background
        ctx.fillStyle = 'rgba(0, 0, 0, 0.1)';
        ctx.fillRect(0, 0, plotWidth, plotHeight);
        
        const dataToPlot = getDisplayData();

        // One y range per frame, from the samples actually drawn, shared by the trace,
        // the threshold line and the labels so they always agree.
        const referenceTime = getContinuousReferenceTime(dataToPlot);
        const visibleData = getVisiblePoints(dataToPlot, referenceTime);
        const range = getDisplayRange(visibleData);

        // The left margin depends on the value labels, and everything else on the margin.
        const valueLabels = getValueTickLabels(range);
        layoutLeftMargin(valueLabels);
        const { ticks: timeTicks, unit: timeUnit } = getTimeTicks();

        drawGrid(timeTicks);
        drawAxes();

        // Draw data
        if (range && visibleData.length > 0) {
            drawDataLine(visibleData, range, referenceTime, TRACE_COLOR);
        }

        drawThresholdLine(range);
        
        // Draw trigger line
        drawTriggerLine();
        
        // Draw labels
        drawLabels(timeTicks, timeUnit, valueLabels);

        drawCanvasBadges();
        
        // Draw "No Data" message if no data
        if (dataToPlot.length === 0) {
            ctx.fillStyle = 'rgba(255, 255, 255, 0.5)';
            ctx.font = '16px Inter, system-ui, sans-serif';
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText('No Data Available', plotWidth / 2, plotHeight / 2);
        }
    }

    /**
     * Queues one {@link render} on the next animation frame. Further calls before that
     * frame are ignored, so a burst of prop changes costs one draw.
     */
    function scheduleRender() {
        if (renderQueued) return;
        renderQueued = true;
        animationFrame = requestAnimationFrame(() => {
            renderQueued = false;
            render();
        });
    }
    
    /**
     * Sizes the canvas, draws the first frame, and redraws on window resize. The
     * cleanup removes the listener and cancels any queued frame.
     */
    onMount(() => {
        resizeCanvas();
        scheduleRender();
        
        const handleResize = () => {
            resizeCanvas();
            scheduleRender();
        };
        
        window.addEventListener('resize', handleResize);
        
        return () => {
            window.removeEventListener('resize', handleResize);
            if (animationFrame) cancelAnimationFrame(animationFrame);
        };
    });
    
    /**
     * Schedules a redraw when `data`, `mode` or `frozenData` change. In frozen mode it
     * waits until `frozenData` is set.
     */
    $effect(() => {
        if (ctx && data) {
            if (mode === 'frozen' && !frozenData) return;
            scheduleRender();
        }
    });

    /**
     * Resets the continuous autoscale extrema when autoscale is turned off or the live
     * buffer is cleared, so the axis can shrink again, then schedules a redraw.
     */
    $effect(() => {
        if (!yAutoScale) {
            stickyAutoExtrema = null;
        } else if (mode !== 'frozen' && data.length === 0) {
            stickyAutoExtrema = null;
        }
        scheduleRender();
    });

    /**
     * Redraws when any display setting changes, so axis, window and badge changes show
     * at once rather than with the next batch of data.
     */
    $effect(() => {
        // Bare reads register these props as dependencies.
        timeWindow; unit; isTriggered; triggerTime; frozenPreWindowSec; frozenPostWindowSec;
        frozenCollecting; showTriggerThreshold; triggerThreshold; prebuffering;
        yMin; yMax; invertX; invertY;
        scheduleRender();
    });
</script>

<!--
@component
Live line plot of one LabJack channel, drawn on a `<canvas>`. Used by the plot page
`/labjacks/plots/[asset_number]`, which subscribes to the channel, calibrates the
samples and passes them in. This component does no NATS I/O.

Modes:
- `continuous`: the x axis shows the last `timeWindow` seconds, newest at the right
  (left with `invertX`). The right edge is the newest sample's own time (sample time,
  not the browser clock).
- `frozen`: after a trigger, shows `frozenData` from `frozenPreWindowSec` before to
  `frozenPostWindowSec` after `triggerTime`, with the trigger at 0 s. With autoscale,
  the y range covers every captured sample, so it can widen while the capture is
  collecting. Badges read COLLECTING while post-trigger samples are still arriving,
  then FROZEN.

Rendering: redraws are batched to one per animation frame. Each frame keeps only
samples inside the window, sorts them if needed, and downsamples to a min/max pair per
pixel column so spikes stay visible at any sample rate. The path breaks at every `NaN`
value (missing sample or stream gap) and is clipped to the plot area. With autoscale the y axis snaps to round 1/2/5 grid
steps and only widens in continuous mode until autoscale is turned off or `data` is
emptied. Below the canvas: sample count, source clock and lag at t = 0 (the newest
sample, or the one nearest the trigger when frozen; lag is hidden while a capture is
held), the number of samples inside the visible window, and the latest value.

Props:
- `data: DataPoint[]`: live samples. `timestamp` is Unix epoch ms; `sourceTimestamp`
  and `receivedAt` (both epoch ms, optional) feed the source clock and lag badges.
- `unit: string`: unit of the plotted values, for the value axis, threshold and badges.
- `calibrated?: boolean`: the values are calibrated; shows the latest raw volts next
  to Latest. Default `false`.
- `timeWindow: number`: continuous window width, seconds.
- `isTriggered: boolean`: a trigger has fired.
- `triggerTime: number`: trigger time, Unix epoch ms; `0` means none.
- `mode: 'continuous' | 'frozen'`: see above.
- `frozenData?: DataPoint[]`: samples around the trigger, shown in frozen mode.
- `frozenPreWindowSec?: number`: seconds before the trigger. Default `timeWindow`.
- `frozenPostWindowSec?: number`: seconds after the trigger. Default `timeWindow`.
- `frozenCollecting?: boolean`: shows COLLECTING instead of FROZEN. Default `false`.
- `showTriggerThreshold?: boolean`: draws the threshold line and LEVEL badge.
  Default `false`.
- `triggerThreshold?: number`: trigger level, in `unit`. No default.
- `prebuffering?: boolean`: shows the PREBUFFERING badge. Default `false`.
- `yAutoScale?: boolean`: fit the y axis to the data. Default `true`.
- `yMin?: number`, `yMax?: number`: fixed y limits when autoscale is off. Defaults
  `-1` and `1`.
- `invertX?: boolean`, `invertY?: boolean`: mirror an axis. Default `false`.

Events: none. The component only reads its props.
-->

<div class="w-full h-80 bg-base-200 rounded-lg overflow-hidden flex-shrink-0">
    <canvas
        bind:this={canvas}
        class="w-full h-full"
        style="display: block;"
    ></canvas>
</div>

<div class="mt-3 text-sm text-base-content/70 flex-shrink-0">
    {#if (mode === 'continuous' && data.length > 0) || (mode === 'frozen' && frozenData && frozenData.length > 0)}
        {@const plotData = getDisplayData()}
        {@const zeroPoint = getZeroTimePoint(plotData)}
        {@const latestPoint = latestFinitePoint(plotData)}
        {@const zeroSourceTimestamp = (typeof zeroPoint?.sourceTimestamp === 'number' && Number.isFinite(zeroPoint.sourceTimestamp)) ? zeroPoint.sourceTimestamp : null}
        {@const lagReferenceTimestamp = (typeof zeroPoint?.receivedAt === 'number' && Number.isFinite(zeroPoint.receivedAt)) ? zeroPoint.receivedAt : zeroPoint?.timestamp}
        {@const zeroLagMs = zeroSourceTimestamp !== null && typeof lagReferenceTimestamp === 'number' ? Math.max(0, lagReferenceTimestamp - zeroSourceTimestamp) : null}
        <div class="flex justify-between items-center gap-2 flex-wrap">
            <div class="flex items-center gap-2 flex-wrap">
                <span class="badge badge-outline badge-sm" title="Samples inside the visible time window">
                    Data Points: {countVisibleSamples(plotData)}
                </span>
                {#if zeroSourceTimestamp !== null}
                    <span class="badge badge-secondary badge-sm">
                        t=0 Src: {formatSourceClock(zeroSourceTimestamp)}
                    </span>
                {/if}
            </div>
            <div class="flex items-center gap-2 flex-wrap justify-end">
                <!-- Lag is meaningful only for live data, not a held capture. -->
                {#if zeroLagMs !== null && !(mode === 'frozen' && isTriggered)}
                    <span class="badge badge-accent badge-sm">
                        Lag: {formatLag(zeroLagMs)}
                    </span>
                {/if}
                <span class="badge badge-primary badge-sm">
                    Latest: {latestPoint ? latestPoint.value.toFixed(3) : '--'} {unit}
                </span>
                {#if calibrated}
                    <span class="badge badge-ghost badge-sm" title="Raw reading of the latest sample, before calibration">
                        Raw: {latestPoint && typeof latestPoint.raw === 'number' && Number.isFinite(latestPoint.raw) ? latestPoint.raw.toFixed(4) : '--'} V
                    </span>
                {/if}
            </div>
        </div>
        {#if mode === 'frozen' && isTriggered}
            <div class="text-xs text-warning mt-2 flex items-center">
                <svg class="w-3 h-3 mr-1" fill="currentColor" viewBox="0 0 20 20">
                    <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm1-12a1 1 0 10-2 0v4a1 1 0 00.293.707l2.828 2.829a1 1 0 101.415-1.415L11 9.586V6z" clip-rule="evenodd"/>
                </svg>
                Frozen at: {new Date(triggerTime).toLocaleTimeString()}
            </div>
        {:else}
            <!-- Empty space to maintain consistent height -->
            <div class="h-5"></div>
        {/if}
    {:else}
        <!-- Empty space to maintain consistent height when no data -->
        <div class="flex justify-between items-center">
            <span class="badge badge-outline badge-sm">
                Data Points: 0
            </span>
            <span class="badge badge-primary badge-sm">
                Latest: -- {unit}
            </span>
        </div>
        <div class="h-5"></div>
    {/if}
</div>
