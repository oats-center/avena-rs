<script lang="ts">
    import type { LiveConnectionState } from "$lib/plot/connection";

    /** Component props. See the `@component` block below. */
    interface Props {
        /** State of the page's NATS connection. */
        state: LiveConnectionState;
        /** Why the connection closed, if known. */
        reason: string;
        /** Called by the Reconnect button. */
        onreconnect: () => void;
    }

    let { state, reason, onreconnect }: Props = $props();
</script>

<!--
@component
Banner of the plot page's live connection, shown once a config is loaded: a yellow
"Reconnecting..." banner while the NATS client tries to get the server back, and a
red "Disconnected" banner with a Reconnect button once it has given up or was closed.
Nothing is shown while connected or connecting.
-->
{#if state === "reconnecting"}
    <div class="alert alert-warning mb-6" role="status">
        <span class="loading loading-spinner loading-sm"></span>
        <span>Connection to NATS lost. Reconnecting... The plots resume by themselves; the missing time shows as a gap.</span>
    </div>
{:else if state === "disconnected"}
    <div class="alert alert-error mb-6" role="alert">
        <span>Disconnected from NATS{reason ? `: ${reason}` : ""}. Live data has stopped.</span>
        <div>
            <button onclick={onreconnect} class="btn btn-sm">Reconnect</button>
        </div>
    </div>
{/if}
