<script lang="ts">
    import type { LabJackConfig } from "$lib/labjack-config";
    import { deriveAutoTimeWindowSec, snapshotRateHz, streamProblemText } from "$lib/plot/channel";
    import type { ChannelView } from "$lib/plot/channel-view.svelte";
    import { connectionBadgeClass, connectionLabel, type LiveConnectionState } from "$lib/plot/connection";
    import { liveLabJackChannelPattern } from "$lib/subjects";

    /** Component props. See the `@component` block below. */
    interface Props {
        /** Loaded config. */
        config: LabJackConfig;
        /** Asset number from the URL. */
        assetNumber: number;
        /** Plotted channels, in config order. */
        channels: number[];
        /** State of every enabled channel. */
        views: Map<number, ChannelView>;
        /** State of the page's NATS connection. */
        connectionState: LiveConnectionState;
    }

    let { config, assetNumber, channels, views, connectionState }: Props = $props();
</script>

<!--
@component
Data Statistics card of the plot page: the config's scan settings, the live subject
pattern, per plotted channel the snapshot size, its average rate and any stream
problems, and the connection state.
-->
<div class="card bg-base-100 shadow-xl mb-6">
    <div class="card-body">
        <h4 class="card-title text-base-content">Data Statistics</h4>
        <div class="text-sm text-base-content/70 space-y-1">
            <div class="flex justify-between">
                <span>Asset Number:</span>
                <span class="badge badge-primary badge-sm">{assetNumber}</span>
            </div>
            <div class="flex justify-between">
                <span>Scans / Read:</span>
                <span class="badge badge-info badge-sm">{config.sensor_settings.scans_per_read}</span>
            </div>
            <div class="flex justify-between">
                <span>Scan Rate:</span>
                <span class="badge badge-info badge-sm">{config.sensor_settings.scan_rate_hz} Hz</span>
            </div>
            <div class="flex justify-between">
                <span>Auto X Window:</span>
                <span class="badge badge-info badge-sm">{deriveAutoTimeWindowSec(config.sensor_settings.scan_rate_hz).toFixed(3)} s</span>
            </div>
            <div class="flex justify-between">
                <span>Enabled Channels:</span>
                <span class="badge badge-secondary badge-sm">{config.sensor_settings.channels_enabled.join(', ')}</span>
            </div>
            <div class="flex justify-between">
                <span>NATS Subject Pattern:</span>
                <span class="badge badge-accent badge-sm font-mono h-auto break-all">{liveLabJackChannelPattern(config)}</span>
            </div>
            <div class="flex justify-between">
                <span>Channel Data Status:</span>
                <div class="flex flex-wrap gap-1">
                    <!--
                        Rate is the average sample rate of the current
                        snapshot: points divided by its time span.
                    -->
                    {#each channels as ch (ch)}
                        {@const data = views.get(ch)?.liveData ?? []}
                        {@const rate = snapshotRateHz(data)}
                        {@const status = views.get(ch)?.streamStatus}
                        {@const problems = streamProblemText(ch, status)}
                        <span class="badge badge-outline badge-xs">Ch{ch}: {data.length} pts ({rate} Hz)</span>
                        <!--
                            Stream problems since the page loaded: messages
                            dropped because decoding fell behind, gaps in the
                            received data, timeline resets, samples skipped as
                            duplicates, undecodable messages.
                        -->
                        {#if status && status.dropped > 0}
                            <span class="badge badge-error badge-xs">Ch{ch}: {status.dropped} msgs dropped (overload)</span>
                        {/if}
                        {#if problems}
                            <span class="badge badge-warning badge-xs">{problems}</span>
                        {/if}
                    {/each}
                </div>
            </div>
            <div class="flex justify-between">
                <span>Connection Status:</span>
                <span class="badge {connectionBadgeClass(connectionState)} badge-sm">
                    {connectionLabel(connectionState)}
                </span>
            </div>
        </div>
    </div>
</div>
