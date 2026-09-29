<script lang="ts">
    import { page } from "$app/state";

    /** Short heading for the status code. */
    const heading = $derived(
        page.status === 404 ? "Page not found" : page.status >= 500 ? "Something went wrong" : "Request failed"
    );
</script>

<!--
@component
Error page for every route, shown by SvelteKit for unknown URLs (404) and for errors
thrown while loading a page. Shows the status code, a short message and links back to
the login page and the LabJack list. It makes no NATS calls.
-->
<svelte:head>
    <title>{page.status} {heading} - Avena-RS</title>
</svelte:head>

<div class="min-h-screen bg-base-300 flex items-center justify-center p-4">
    <div class="card bg-base-100 shadow-2xl w-full max-w-md">
        <div class="card-body items-center text-center">
            <p class="text-6xl font-bold text-warning">{page.status}</p>
            <h1 class="card-title text-2xl mt-2">{heading}</h1>
            {#if page.error?.message && page.error.message !== "Not Found"}
                <p class="text-base-content/70 break-words">{page.error.message}</p>
            {:else if page.status === 404}
                <p class="text-base-content/70">
                    There is no page at <span class="font-mono">{page.url.pathname}</span>.
                </p>
            {/if}
            <div class="card-actions mt-6">
                <a href="/labjacks" class="btn btn-outline">LabJacks</a>
                <a href="/" class="btn btn-warning">Go to login</a>
            </div>
        </div>
    </div>
</div>
