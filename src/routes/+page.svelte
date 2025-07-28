<script lang="ts">
    import "../app.css";
    import { goto } from "$app/navigation";
    import { wsconnect } from "@nats-io/nats-core";
    //use session storage to save the connection ID, which will be used when in the config and map pages
    let serverName = $state<string>("");
    let password = $state<string>("");
    let loading = $state<boolean>(false);
    let alert = $state<string>("");

    async function connect() {
        loading = true;
        try {
            const server = await wsconnect({ servers: serverName });
            if (server) {
                server.close();
                sessionStorage.setItem("serverName", serverName);
                goto("/config/cabinet-select");
            }
            loading = false;
        } catch (error) {
            loading = false;
            alert = "Incorrect Server Name or Password";
            console.error("Error Initializing NATS Connection");
        }
    }
</script>

<div class="flex items-center justify-center h-screen">
    <div class="flex flex-col justify-center text-center items-center">
        <h1 class="text-4xl">Welcome to AvenaOTR!</h1>
        <h2 class="text-2xl pt-5 pb-10">Enter Server Credentials Below:</h2>
        <h2 class="text-red-600 pb-5">{alert}</h2>
        <input
            type="text"
            placeholder="Server"
            bind:value={serverName}
            class="input input-bordered input-primary w-72 bg-secondary text-accent placeholder-accent mb-3"
        />
        <input
            type="text"
            placeholder="Password"
            bind:value={password}
            class="input input-bordered input-primary w-72 bg-secondary text-accent placeholder-accent mb-3"
        />
        <button class="btn btn-secondary w-28 mb-20" onclick={connect}>
            {#if loading}
                <div class="loading-overlay">
                    <span class="loading loading-spinner loading-lg"></span>
                </div>
            {:else}
                Connect
            {/if}
        </button>
    </div>
</div>
