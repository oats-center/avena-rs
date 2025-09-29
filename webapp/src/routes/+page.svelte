<script lang="ts">
    import { goto } from "$app/navigation";
    import { connect } from "$lib/nats.svelte";
    
    let serverName = $state<string>("");
    let credentialsFile = $state<File | null>(null);
    let credentialsContent = $state<string>("");
    let loading = $state<boolean>(false);
    let alert = $state<string>("");
    
    // Debug log
    console.log("Login page loaded");

    async function handleFileUpload(event: Event) {
        const target = event.target as HTMLInputElement;
        const file = target.files?.[0];
        
        if (file) {
            credentialsFile = file;
            try {
                credentialsContent = await file.text();
                alert = ""; // Clear any previous alerts
            } catch (error) {
                alert = "Failed to read credentials file. Please try again.";
                console.error("Error reading file:", error);
            }
        }
    }

    async function connectToNats() {
        if (!serverName.trim()) {
            alert = "Please enter a server URL";
            return;
        }
        
        if (!credentialsContent) {
            alert = "Please upload a credentials file";
            return;
        }
        
        loading = true;
        try {
            const natsService = await connect(serverName, credentialsContent);
            if (natsService) {
                natsService.connection.close();
                sessionStorage.setItem("serverName", serverName);
                sessionStorage.setItem("credentialsContent", credentialsContent);
                goto("/labjacks");
            } else {
                alert = "Connection failed. Please check your server URL and credentials file.";
            }
            loading = false;
        } catch (error) {
            loading = false;
            alert = "Connection failed. Please check your server URL and credentials file.";
            console.error("Error Initializing NATS Connection:", error);
        }
    }

    function handleKeyPress(event: KeyboardEvent) {
        if (event.key === 'Enter') {
            connectToNats();
        }
    }
</script>

<svelte:head>
    <title>Login - Avena-OTR LabJack Management</title>
</svelte:head>

<div class="min-h-screen bg-base-300 flex items-center justify-center p-4" data-theme="dark">
    <!-- Main Login Card -->
    <div class="w-full max-w-md">
        <!-- Logo/Brand Section -->
        <div class="text-center mb-8">
            <div class="avatar placeholder mb-6">
                <div class="flex items-center justify-center h-12 w-12">
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      fill="none"
                      viewBox="2 2 24 24"
                      stroke-width="1.5"
                      stroke="currentColor"
                      class="w-12 h-12"
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
                      />
                    </svg>
                  </div>
                  
            </div>
            <h1 class="text-4xl font-bold text-base-content mb-2">Avena-OTR</h1>
            <p class="text-base-content/70 text-lg">Roadside Infrastructure Dashboard</p>
        </div>

        <!-- Login Form Card -->
        <div class="card bg-base-100 shadow-2xl">
            <div class="card-body">
                <div class="text-center mb-6">
                    <h2 class="card-title text-2xl justify-center mb-2">Connect to NATS Server</h2>
                    <p class="text-base-content/70 text-sm">Enter server details and upload credentials file to access the dashboard</p>
                </div>

                <!-- Alert Message -->
                {#if alert}
                    <div class="alert alert-error mb-6">
                        <svg class="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                            <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd"/>
                        </svg>
                        <span>{alert}</span>
                    </div>
                {/if}

                <!-- Form Fields -->
                <form onsubmit={(e) => { e.preventDefault(); connectToNats(); }} class="space-y-6">
                    <!-- Server URL Field -->
                    <div class="form-control">
                        <label class="label" for="server">
                            <span class="label-text">Server URL</span>
                        </label>
                        <div class="relative">
                            <div class="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                                <svg class="h-5 w-5 text-base-content/50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
                                </svg>
                            </div>
                            <input
                                id="server"
                                type="text"
                                placeholder="ws://nats1.oats:8080"
                                bind:value={serverName}
                                onkeypress={handleKeyPress}
                                class="input input-bordered w-full pl-10"
                                required
                            />
                        </div>
                        <div class="label">
                            <span class="label-text-alt">Example: ws://nats1.oats:8080</span>
                        </div>
                    </div>

                    <!-- Credentials File Upload Field -->
                    <div class="form-control">
                        <label class="label" for="credentials">
                            <span class="label-text">Credentials File</span>
                        </label>
                        <div class="relative">
                            <div class="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                                <svg class="h-5 w-5 text-base-content/50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
                                </svg>
                            </div>
                            <input
                                id="credentials"
                                type="file"
                                accept=".creds,.txt"
                                onchange={handleFileUpload}
                                class="file-input file-input-bordered w-full pl-10"
                            />
                        </div>
                        <label class="label">
                            <span class="label-text-alt">
                                {#if credentialsFile}
                                    ✅ {credentialsFile.name} uploaded
                                {:else}
                                    Upload your NATS credentials file (.creds)
                                {/if}
                            </span>
                        </label>
                        
                    </div>

                    <!-- Connect Button -->
                    <button
                        type="submit"
                        disabled={loading}
                        class="btn btn-warning w-full"
                    >
                        {#if loading}
                            <span class="loading loading-spinner loading-sm"></span>
                            Connecting...
                        {:else}
                            <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                            </svg>
                            Connect with Credentials
                        {/if}
                    </button>
                </form>

            </div>
        </div>
    </div>
</div>
