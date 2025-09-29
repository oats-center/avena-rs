<script lang="ts">
    import { onMount } from "svelte";
    
    interface DataPoint {
        timestamp: number;
        value: number;
    }
    
    interface Props {
        data: DataPoint[];
        unit: string;
        timeWindow: number;
        isTriggered: boolean;
        triggerTime: number;
        mode: 'continuous' | 'frozen';
        frozenData?: DataPoint[];
    }
    
    let { data, unit, timeWindow, isTriggered, triggerTime, mode, frozenData }: Props = $props();
    
    let canvas: HTMLCanvasElement;
    let ctx: CanvasRenderingContext2D;
    let animationFrame: number;
    let lastRenderTime = 0;
    let plotWidth = 0;
    let plotHeight = 0;
    let margin = { top: 30, right: 40, bottom: 50, left: 80 };
    
    // Color palette for different channels
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
    
    function getChannelColor(channelIndex: number): string {
        return colors[channelIndex % colors.length];
    }
    
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
                // For frozen mode, show time relative to trigger (negative to positive values)
                timeValue = -timeWindow + (i * (2 * timeWindow / 10));
            } else {
                // For continuous mode, show time relative to now (negative values)
                timeValue = -timeWindow + (i * timeStep);
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
        
        if (data.length > 0) {
            const values = data.map(d => d.value);
            const minValue = Math.min(...values);
            const maxValue = Math.max(...values);
            const valueRange = maxValue - minValue;
            const padding = valueRange > 0 ? valueRange * 0.1 : 1;
            
            for (let i = 0; i <= 8; i++) {
                const y = margin.top + (i / 8) * (plotHeight - margin.top - margin.bottom);
                const value = maxValue + padding - (i / 8) * (valueRange + 2 * padding);
                ctx.fillText(value.toFixed(2), margin.left - 20, y);
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
    
    function drawTriggerLine() {
        if (!ctx || !isTriggered || triggerTime === 0) return;
        
        let x: number;
        
        if (mode === 'frozen') {
            // For frozen mode, the trigger line should be at the center (time = 0)
            x = margin.left + (plotWidth - margin.left - margin.right) / 2;
        } else {
            // For continuous mode, show where the trigger occurred relative to current time
            const now = Date.now();
            const timeSinceTrigger = (now - triggerTime) / 1000;
            x = margin.left + (timeSinceTrigger / timeWindow) * (plotWidth - margin.left - margin.right);
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
    
    
    function applyLightSmoothing(data: DataPoint[]): DataPoint[] {
        if (data.length < 3) return data;
        
        // For high-frequency data (7000+ samples/sec), use more aggressive smoothing
        const isHighFrequency = data.length > 1000;
        const smoothingWindow = isHighFrequency ? 7 : 5;
        
        const smoothed: DataPoint[] = [];
        
        // Keep first few points as is
        const initialCount = Math.min(Math.floor(smoothingWindow / 2), data.length);
        for (let i = 0; i < initialCount; i++) {
            const point = data[i];
            if (point && typeof point.timestamp === 'number' && typeof point.value === 'number') {
                smoothed.push(point);
            }
        }
        
        // Apply moving average smoothing
        const halfWindow = Math.floor(smoothingWindow / 2);
        for (let i = halfWindow; i < data.length - halfWindow; i++) {
            let weightedSum = 0;
            let weightSum = 0;
            
            // Apply weighted moving average
            for (let j = -halfWindow; j <= halfWindow; j++) {
                const dataPoint = data[i + j];
                if (!dataPoint || typeof dataPoint.value !== 'number') {
                    continue;
                }
                
                const weight = j === 0 ? 0.4 : 0.6 / (smoothingWindow - 1); // Center point gets more weight
                weightedSum += dataPoint.value * weight;
                weightSum += weight;
            }
            
            const currentPoint = data[i];
            if (!currentPoint || typeof currentPoint.timestamp !== 'number') {
                continue;
            }
            
            smoothed.push({
                timestamp: currentPoint.timestamp,
                value: weightedSum / weightSum
            });
        }
        
        // Keep last few points as is
        const startIndex = data.length - Math.min(Math.floor(smoothingWindow / 2), data.length);
        for (let i = startIndex; i < data.length; i++) {
            const point = data[i];
            if (point && typeof point.timestamp === 'number' && typeof point.value === 'number') {
                smoothed.push(point);
            }
        }
        
        return smoothed;
    }
    
    
    function drawDataLine(data: DataPoint[], color: string) {
        if (!ctx || data.length < 1) return;
        
        // For frozen mode, use the frozen data if available, otherwise use regular data
        const dataToPlot = mode === 'frozen' && frozenData ? frozenData : data;
        if (dataToPlot.length < 1) return;
        
        
        const now = Date.now();
        const values = dataToPlot.map(d => d.value);
        const minValue = Math.min(...values);
        const maxValue = Math.max(...values);
        const valueRange = maxValue - minValue;
        const padding = valueRange > 0 ? valueRange * 0.1 : 1; // Avoid division by zero
        
        // Enable anti-aliasing for smooth lines
        ctx.imageSmoothingEnabled = true;
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5; // Slightly thinner for smoother appearance
        ctx.lineCap = 'round';
        ctx.lineJoin = 'round';
        
        // Apply light smoothing to the data for better visualization
        const smoothedData = applyLightSmoothing(dataToPlot);
        
        ctx.beginPath();
        
        let firstPoint = true;
        let lastValidX = 0;
        let lastValidY = 0;
        
        for (const point of smoothedData) {
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
                timeSincePoint = (now - point.timestamp) / 1000;
            }
            
            // Adjust positioning for frozen mode
            let x: number;
            if (mode === 'frozen' && isTriggered) {
                // For frozen mode, map time values to the plot area
                // timeSincePoint can be negative (before trigger) or positive (after trigger)
                // We want to map [-timeWindow, +timeWindow] to [0, 1] for positioning
                const normalizedTime = (timeSincePoint + timeWindow) / (2 * timeWindow);
                x = margin.left + normalizedTime * (plotWidth - margin.left - margin.right);
            } else {
                // For continuous mode, use the original logic
                x = margin.left + (timeSincePoint / timeWindow) * (plotWidth - margin.left - margin.right);
            }
            
            const y = margin.top + ((maxValue + padding - point.value) / (valueRange + 2 * padding)) * (plotHeight - margin.top - margin.bottom);
            
            
            if (x >= margin.left && x <= plotWidth - margin.right) {
                if (firstPoint) {
                    ctx.moveTo(x, y);
                    firstPoint = false;
                } else {
                    // Always draw lines between consecutive points for smooth curves
                    ctx.lineTo(x, y);
                }
                lastValidX = x;
                lastValidY = y;
            }
        }
        
        ctx.stroke();
        
        // Skip drawing individual data points for smoother appearance
        // With 7000 samples/sec, the connected line should be smooth enough
    }
    
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
        
        // Draw data
        if (data.length > 0) {
            drawDataLine(data, getChannelColor(0));
        }
        
        // Draw trigger line
        drawTriggerLine();
        
        // Draw labels
        drawLabels();
        
        // Draw "No Data" message if no data
        if (data.length === 0) {
            ctx.fillStyle = 'rgba(255, 255, 255, 0.5)';
            ctx.font = '16px Inter, system-ui, sans-serif';
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText('No Data Available', plotWidth / 2, plotHeight / 2);
        }
        
        // Draw frozen indicator for frozen mode
        if (mode === 'frozen' && isTriggered) {
            ctx.fillStyle = 'rgba(255, 193, 7, 0.8)';
            ctx.font = '12px Inter, system-ui, sans-serif';
            ctx.textAlign = 'right';
            ctx.textBaseline = 'top';
            
            // Check if we're still collecting data after trigger
            const now = Date.now();
            const timeSinceTrigger = (now - triggerTime) / 1000;
            if (timeSinceTrigger <= timeWindow) {
                ctx.fillText('COLLECTING...', plotWidth - margin.right - 5, margin.top + 5);
            } else {
                ctx.fillText('FROZEN', plotWidth - margin.right - 5, margin.top + 5);
            }
        }
    }
    
    function animate() {
        const now = performance.now();
        if (now - lastRenderTime >= 16) { // ~60 FPS
            render();
            lastRenderTime = now;
        }
        
        // Only continue animation for continuous mode
        // For frozen mode, we only animate when data changes
        if (mode === 'continuous') {
            animationFrame = requestAnimationFrame(animate);
        }
    }
    
    onMount(() => {
        resizeCanvas();
        
        // Only start continuous animation for continuous mode
        if (mode === 'continuous') {
            animate();
        } else {
            // For frozen mode, just render once initially
            render();
        }
        
        const handleResize = () => {
            resizeCanvas();
        };
        
        window.addEventListener('resize', handleResize);
        
        return () => {
            window.removeEventListener('resize', handleResize);
            if (animationFrame) {
                cancelAnimationFrame(animationFrame);
            }
        };
    });
    
    // Re-render when data changes using effect
    $effect(() => {
        if (ctx && data) {
            if (mode === 'frozen') {
                // For frozen mode, only render when frozen data changes
                if (frozenData) {
                    render();
                }
            } else {
                // For continuous mode, render on any data change
                render();
            }
        }
    });
</script>

<div class="w-full h-80 bg-base-200 rounded-lg overflow-hidden flex-shrink-0">
    <canvas
        bind:this={canvas}
        class="w-full h-full"
        style="display: block;"
    ></canvas>
</div>

<div class="mt-3 text-sm text-base-content/70 flex-shrink-0">
    {#if (mode === 'continuous' && data.length > 0) || (mode === 'frozen' && frozenData && frozenData.length > 0)}
        <div class="flex justify-between items-center">
            <span class="badge badge-outline badge-sm">
                Data Points: {mode === 'frozen' && frozenData ? frozenData.length : data.length}
            </span>
            <span class="badge badge-primary badge-sm">
                Latest: {(mode === 'frozen' && frozenData ? frozenData[frozenData.length - 1] : data[data.length - 1])?.value.toFixed(3)} {unit}
            </span>
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
