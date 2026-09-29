<script lang="ts">
    import "../app.css";
    import type { Snippet } from "svelte";

    let { children }: { children: Snippet } = $props();
</script>

<!--
@component
Root layout for every route. It imports the global stylesheet `app.css` (Tailwind and
daisyUI) and renders the current page through `{@render children()}`. It has no state,
reads no sessionStorage and makes no NATS calls; each page opens its own connection.
-->
{@render children()}
