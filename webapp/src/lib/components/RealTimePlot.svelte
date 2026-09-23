<script lang="ts">
    import { onMount } from "svelte";
    
    /** One sample of one channel, as the plot page hands it over. */
    interface DataPoint {
        /** Sample time, Unix epoch in milliseconds. Positions the point on the time axis. */
        timestamp: number;
        /** Sample value in `unit` (already calibrated by the caller). */
        value: number;
        /** Source clock time of the sample, Unix epoch ms, or `null` if unknown. */
        sourceTimestamp?: number | null;
        /** Browser time the containing message arrived, Unix epoch ms. Used for the lag badge. */
        receivedAt?: number;
    }
    
    /** Component props. See the `@component` block below for each one. */
    interface Props {
        /** Live buffer of samples for this channel. */
        data: DataPoint[];
        /** Unit label for the y axis, badges and threshold. */
        unit: string;
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
    /** Space around the plot area for tick labels and axis titles, in CSS pixels. */
    let margin = { top: 30, right: 40, bottom: 50, left: 80 };
    /** Value extrema captured once when a trigger freezes the plot, so the y axis holds still. */
    let frozenRange: { min: number; max: number } | null = null;
    /** Trigger time that `frozenRange` was captured for; a new trigger recaptures it. */
    let frozenRangeTriggerTime = 0;
    /** Running min/max of every value seen in continuous autoscale. Only grows until reset. */
    let stickyAutoExtrema: { min: number; max: number } | null = null;
    /** Number of horizontal grid intervals on the y axis. */
    const Y_GRID_DIVISIONS = 8;
    /** Smallest autoscale grid step, in `unit`. Keeps a flat signal from collapsing the axis. */
    const MIN_AUTO_Y_INTERVAL = 0.01;
    /** Fraction of `timeWindow` the newest sample may lag `Date.now()` before the axis anchors to it. */
    const MAX_VISIBLE_LIVE_LAG_FRACTION = 0.1;
    /** Lower bound on that allowed lag, in milliseconds. */
    const MAX_VISIBLE_LIVE_LAG_MS = 75;
    
    // Color palette for different channels. Only index 0 is used today (one trace per plot).
    const colors = [
        '#3B82F6', // Blue
        '#EF4444', // Red
        '#10B981', // Green
        '#F59E0B', // Yellow
        '#8B5CF6', // Purple
        '#06B6D4', // Cyan
        '#F97316', // Orange
        '#84CC16'  // Lime
    ];
    
    /**
     * Returns the trace color for a channel index, cycling through the palette.
     *
     * @param channelIndex - Zero-based index; wraps modulo the palette length.
     * @returns A CSS hex color.
     */
    function getChannelColor(channelIndex: number): string {
        return colors[channelIndex % colors.length];
    }
    
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
     * Draws the background grid: 11 vertical lines (10 time intervals) and 9 horizontal
     * lines (8 value intervals) across the plot area.
     */
    function drawGrid() {
        if (!ctx) return;
        
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.1)';
        ctx.lineWidth = 1;
        
        // Vertical grid lines (time)
        const timeStep = timeWindow / 10;
        for (let i = 0; i <= 10; i++) {
            const x = margin.left + (i / 10) * (plotWidth - margin.left - margin.right);
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
     * Returns the smallest and largest finite `value` in a list of points.
     *
     * @param points - Samples to scan. Non-finite values are skipped.
     * @returns `{ min, max }`, or `null` if there are no finite values.
     */
    function computeValueRange(points: DataPoint[]): { min: number; max: number } | null {
        if (!points || points.length === 0) return null;
        let minValue = Number.POSITIVE_INFINITY;
        let maxValue = Number.NEGATIVE_INFINITY;
        for (const point of points) {
            if (!Number.isFinite(point.value)) continue;
            minValue = Math.min(minValue, point.value);
            maxValue = Math.max(maxValue, point.value);
        }
        if (!Number.isFinite(minValue) || !Number.isFinite(maxValue)) return null;
        return { min: minValue, max: maxValue };
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
     * - Frozen mode: `frozenRange` if captured, otherwise the extrema of `points`.
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
            const frozenExtrema = frozenRange ?? computeValueRange(points);
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
     * Draws the time tick labels, value tick labels and both axis titles.
     *
     * Time labels match {@link mapTimeToX}: in continuous mode they run from
     * `-timeWindow` to 0 s, in frozen mode from `-pre` to `+post` around the trigger.
     * Offsets under 0.1 s are shown in milliseconds. Value labels use
     * {@link getDisplayRange}; with no data they show a fixed -10 to 10 scale.
     */
    function drawLabels() {
        if (!ctx) return;
        
        ctx.fillStyle = 'rgba(255, 255, 255, 0.7)';
        ctx.font = '13px Inter, system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'top';
        
        // X-axis labels (time)
        const timeStep = timeWindow / 10;
        for (let i = 0; i <= 10; i++) {
            const x = margin.left + (i / 10) * (plotWidth - margin.left - margin.right);
            let timeValue: number;
            
            if (mode === 'frozen' && isTriggered) {
                const { pre, post } = getFrozenWindow();
                const start = invertX ? post : -pre;
                const step = ((pre + post) / 10) * (invertX ? -1 : 1);
                timeValue = start + (i * step);
            } else {
                const start = invertX ? 0 : -timeWindow;
                const step = invertX ? -timeStep : timeStep;
                timeValue = start + (i * step);
            }
            
            // Format time labels with better precision for high-frequency data
            const timeLabel = Math.abs(timeValue) < 0.1 ? 
                (timeValue * 1000).toFixed(0) + 'ms' : 
                timeValue.toFixed(1) + 's';
            ctx.fillText(timeLabel, x, plotHeight - margin.bottom + 5);
        }
        
        // Y-axis labels (value)
        ctx.textAlign = 'right';
        ctx.textBaseline = 'middle';
        
        const labelData = mode === 'frozen' && frozenData ? frozenData : data;
        const range = getDisplayRange(labelData);

        if (range) {
            const span = range.high - range.low;
            const step = span / Y_GRID_DIVISIONS;
            for (let i = 0; i <= 8; i++) {
                const y = margin.top + (i / 8) * (plotHeight - margin.top - margin.bottom);
                const ratio = i / 8;
                const value = invertY
                    ? range.low + ratio * span
                    : range.high - ratio * span;
                ctx.fillText(formatAxisValue(value, step), margin.left - 20, y);
            }
        } else {
            // Show default scale when no data
            for (let i = 0; i <= 8; i++) {
                const y = margin.top + (i / 8) * (plotHeight - margin.top - margin.bottom);
                const value = 10 - (i / 8) * 20; // Default scale from -10 to 10
                ctx.fillText(value.toFixed(1), margin.left - 20, y);
            }
        }
        
        // Axis titles
        ctx.textAlign = 'center';
        ctx.textBaseline = 'bottom';
        ctx.font = '15px Inter, system-ui, sans-serif';
        ctx.fillText('Time (s)', plotWidth / 2, plotHeight - 5);
        
        ctx.save();
        ctx.translate(25, plotHeight / 2);
        ctx.rotate(-Math.PI / 2);
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
            const referenceTime = getContinuousReferenceTime(data);
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
     */
    function drawThresholdLine() {
        if (!ctx || !showTriggerThreshold || typeof triggerThreshold !== 'number' || Number.isNaN(triggerThreshold)) {
            return;
        }

        const source = mode === 'frozen' && frozenData ? frozenData : data;
        const range = getDisplayRange(source);
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
        ctx.fillText(`Trig ${triggerThreshold.toFixed(3)}`, margin.left + 6, y - 4);
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
     * Uses `Date.now()` so the trace scrolls smoothly. If the newest sample is further
     * from now than the larger of `MAX_VISIBLE_LIVE_LAG_MS` and
     * `MAX_VISIBLE_LIVE_LAG_FRACTION * timeWindow`, uses the newest sample's timestamp
     * instead, so a lagging or clock-skewed source still fills the window rather than
     * sliding off the left edge.
     *
     * @param dataToPlot - Samples in arrival order; the last one is taken as newest.
     * @returns Reference time in Unix epoch ms.
     */
    function getContinuousReferenceTime(dataToPlot: DataPoint[]): number {
        const latestPoint = dataToPlot[dataToPlot.length - 1];
        const latestTimestamp = latestPoint?.timestamp;
        const now = Date.now();
        if (typeof latestTimestamp !== 'number' || Number.isNaN(latestTimestamp)) {
            return now;
        }

        // Keep the live trace filled even when transport/render lag is noticeable.
        const skew = Math.abs(now - latestTimestamp);
        const maxVisibleLag = Math.max(
            MAX_VISIBLE_LIVE_LAG_MS,
            timeWindow * 1000 * MAX_VISIBLE_LIVE_LAG_FRACTION
        );
        if (skew > maxVisibleLag) {
            return latestTimestamp;
        }

        return now;
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
     * Reduces a sorted series to about two points per horizontal pixel, keeping peaks.
     *
     * Splits the series into one bucket per pixel of plot width (at least 16) and keeps
     * each bucket's minimum and maximum sample, in time order. Unlike taking every Nth
     * sample, this never hides a spike, and it bounds the work per frame regardless of
     * sample rate. Series that already fit (two points per bucket or fewer) are returned
     * unchanged.
     *
     * @param data - Samples sorted by `timestamp`.
     * @returns The reduced series, or `data` itself if no reduction was needed or
     *   possible.
     */
    function downsampleMinMax(data: DataPoint[]): DataPoint[] {
        if (data.length <= 2 || plotWidth <= 0) return data;

        // One bucket per CSS pixel of plot width. Two points per bucket is as much detail
        // as the screen can show.
        const bucketCount = Math.max(16, Math.floor(plotWidth - margin.left - margin.right));
        if (data.length <= bucketCount * 2) return data;

        const bucketSize = Math.ceil(data.length / bucketCount);
        const reduced: DataPoint[] = [];

        for (let start = 0; start < data.length; start += bucketSize) {
            const end = Math.min(data.length, start + bucketSize);
            let minPoint: DataPoint | null = null;
            let maxPoint: DataPoint | null = null;

            for (let i = start; i < end; i++) {
                const point = data[i];
                if (!minPoint || point.value < minPoint.value) minPoint = point;
                if (!maxPoint || point.value > maxPoint.value) maxPoint = point;
            }

            if (!minPoint || !maxPoint) continue;

            // Emit min and max in time order so the path does not zigzag backward in x.
            if (minPoint.timestamp <= maxPoint.timestamp) {
                reduced.push(minPoint);
                if (maxPoint !== minPoint) reduced.push(maxPoint);
            } else {
                reduced.push(maxPoint);
                if (maxPoint !== minPoint) reduced.push(minPoint);
            }
        }

        return reduced.length > 1 ? reduced : data;
    }

    /**
     * Returns the samples that fall inside the visible time window.
     *
     * Frozen mode after a trigger keeps `[triggerTime - pre, triggerTime + post]`;
     * otherwise keeps the `timeWindow` seconds ending at `referenceTime`. Both bounds
     * are inclusive.
     *
     * @param dataToPlot - Candidate samples.
     * @param referenceTime - Right edge of the continuous window, Unix epoch ms.
     * @returns A new filtered array (or the input if it is empty).
     */
    function getVisiblePoints(dataToPlot: DataPoint[], referenceTime: number): DataPoint[] {
        if (dataToPlot.length === 0) return dataToPlot;

        if (mode === 'frozen' && isTriggered) {
            const { pre, post } = getFrozenWindow();
            const start = triggerTime - (pre * 1000);
            const end = triggerTime + (post * 1000);
            return dataToPlot.filter((point) => point.timestamp >= start && point.timestamp <= end);
        }

        const end = referenceTime;
        const start = end - (timeWindow * 1000);
        return dataToPlot.filter((point) => point.timestamp >= start && point.timestamp <= end);
    }
    
    
    /**
     * Draws the trace for the visible samples.
     *
     * Steps: pick frozen or live data, keep the visible window, sort by time if needed,
     * downsample with {@link downsampleMinMax}, then draw one path. The path is broken
     * (a new `moveTo`) where time goes backward, where two neighboring points are more
     * than a quarter of the plot width apart (a gap in the data), and around points
     * outside the plot area.
     *
     * @param data - Live samples. Shadows the `data` prop; in frozen mode `frozenData` is
     *   used instead when set.
     * @param color - CSS stroke color.
     */
    function drawDataLine(data: DataPoint[], color: string) {
        if (!ctx || data.length < 1) return;
        
        // For frozen mode, use the frozen data if available, otherwise use regular data
        const dataToPlot = mode === 'frozen' && frozenData ? frozenData : data;
        if (dataToPlot.length < 1) return;
        
        
        const referenceTime = getContinuousReferenceTime(dataToPlot);
        const visibleData = getVisiblePoints(dataToPlot, referenceTime);
        if (visibleData.length < 1) return;

        const range = getDisplayRange(visibleData);
        if (!range) return;
        
        // Enable anti-aliasing for smooth lines
        ctx.imageSmoothingEnabled = true;
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5; // Slightly thinner for smoother appearance
        ctx.lineCap = 'round';
        ctx.lineJoin = 'round';
        
        // Fast path: avoid sorting every frame when data is already monotonic.
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
        const sampledData = downsampleMinMax(orderedData);
        
        ctx.beginPath();
        
        let hasActiveSegment = false;
        let previousTimestamp = Number.NaN;
        let previousX = Number.NaN;
        // A jump wider than this between neighboring points is treated as a data gap and
        // left undrawn instead of bridged with a straight line.
        const reconnectThreshold = (plotWidth - margin.left - margin.right) * 0.25;
        
        for (const point of sampledData) {
            // Validate point before accessing properties
            if (!point || typeof point.timestamp !== 'number' || typeof point.value !== 'number') {
                continue;
            }
            
            let timeSincePoint: number;
            
            if (mode === 'frozen' && isTriggered) {
                // For frozen mode, calculate time relative to trigger time
                // This can be negative (before trigger) or positive (after trigger)
                timeSincePoint = (point.timestamp - triggerTime) / 1000;
            } else {
                // For continuous mode, calculate time relative to now
                timeSincePoint = (referenceTime - point.timestamp) / 1000;
            }
            
            const x = mapTimeToX(timeSincePoint);
            const y = mapValueToY(point.value, range);
            
            
            if (x >= margin.left && x <= plotWidth - margin.right) {
                // Start a new subpath after an off-screen point, a repeated or backward timestamp,
                // or a gap; otherwise extend the current one.
                const nonMonotonicTime = Number.isFinite(previousTimestamp) && point.timestamp <= previousTimestamp;
                const largeJump = Number.isFinite(previousX) && Math.abs(x - previousX) > reconnectThreshold;

                if (!hasActiveSegment || nonMonotonicTime || largeJump) {
                    ctx.moveTo(x, y);
                    hasActiveSegment = true;
                } else {
                    // Draw lines between consecutive reduced points.
                    ctx.lineTo(x, y);
                }
                previousTimestamp = point.timestamp;
                previousX = x;
            } else {
                hasActiveSegment = false;
            }
        }
        
        ctx.stroke();
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
        
        // Draw grid
        drawGrid();
        
        // Draw axes
        drawAxes();
        
        const dataToPlot = mode === 'frozen' && frozenData ? frozenData : data;

        // Draw data
        if (dataToPlot.length > 0) {
            drawDataLine(data, getChannelColor(0));
        }

        drawThresholdLine();
        
        // Draw trigger line
        drawTriggerLine();
        
        // Draw labels
        drawLabels();

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
     * Captures `frozenRange` once per trigger (from `frozenData` if it has samples, else
     * from `data`) and clears it when the plot leaves frozen mode. Later samples added
     * while COLLECTING do not widen it.
     */
    $effect(() => {
        if (mode === 'frozen' && isTriggered && triggerTime > 0) {
            const shouldInitialize =
                triggerTime !== frozenRangeTriggerTime || frozenRange === null;
            if (shouldInitialize) {
                const source =
                    frozenData && frozenData.length > 0 ? frozenData : data;
                frozenRange = computeValueRange(source);
                frozenRangeTriggerTime = triggerTime;
            }
        } else {
            frozenRange = null;
            frozenRangeTriggerTime = 0;
        }
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
  (left with `invertX`). The right edge is `Date.now()`, or the newest sample's time
  when that lags by more than max(75 ms, 10% of the window).
- `frozen`: after a trigger, shows `frozenData` from `frozenPreWindowSec` before to
  `frozenPostWindowSec` after `triggerTime`, with the trigger at 0 s. With autoscale,
  the y range is captured once per trigger so the plot holds still. Badges read COLLECTING while
  post-trigger samples are still arriving, then FROZEN.

Rendering: redraws are batched to one per animation frame. Each frame keeps only
samples inside the window, sorts them if needed, and downsamples to a min/max pair per
pixel column so spikes stay visible at any sample rate. The path breaks at data gaps
wider than a quarter of the plot. With autoscale the y axis snaps to round 1/2/5 grid
steps and only widens in continuous mode until autoscale is turned off or `data` is
emptied. Below the canvas: sample count, source clock and lag at t = 0 (the newest
sample, or the one nearest the trigger when frozen), and the latest value.

Props:
- `data: DataPoint[]`: live samples. `timestamp` is Unix epoch ms; `sourceTimestamp`
  and `receivedAt` (both epoch ms, optional) feed the source clock and lag badges.
- `unit: string`: unit label for the value axis, threshold and badges.
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
        {@const latestPoint = plotData[plotData.length - 1]}
        {@const zeroSourceTimestamp = (typeof zeroPoint?.sourceTimestamp === 'number' && Number.isFinite(zeroPoint.sourceTimestamp)) ? zeroPoint.sourceTimestamp : null}
        {@const lagReferenceTimestamp = (typeof zeroPoint?.receivedAt === 'number' && Number.isFinite(zeroPoint.receivedAt)) ? zeroPoint.receivedAt : zeroPoint?.timestamp}
        {@const zeroLagMs = zeroSourceTimestamp !== null && typeof lagReferenceTimestamp === 'number' ? Math.max(0, lagReferenceTimestamp - zeroSourceTimestamp) : null}
        <div class="flex justify-between items-center gap-2 flex-wrap">
            <div class="flex items-center gap-2 flex-wrap">
                <span class="badge badge-outline badge-sm">
                    Data Points: {plotData.length}
                </span>
                {#if zeroSourceTimestamp !== null}
                    <span class="badge badge-secondary badge-sm">
                        t=0 Src: {formatSourceClock(zeroSourceTimestamp)}
                    </span>
                {/if}
            </div>
            <div class="flex items-center gap-2 flex-wrap justify-end">
                {#if zeroLagMs !== null}
                    <span class="badge badge-accent badge-sm">
                        Lag: {formatLag(zeroLagMs)}
                    </span>
                {/if}
                <span class="badge badge-primary badge-sm">
                    Latest: {latestPoint?.value.toFixed(3)} {unit}
                </span>
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
