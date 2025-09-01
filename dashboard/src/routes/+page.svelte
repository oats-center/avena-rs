<script lang="ts">
    import "../app.css";
    import { goto } from "$app/navigation";
    import { wsconnect } from "@nats-io/nats-core";
    
    let serverName = $state<string>("");
    let password = $state<string>("");
    let loading = $state<boolean>(false);
    let alert = $state<string>("");

    async function connect() {
        if (!serverName.trim()) {
            alert = "Please enter a server URL";
            return;
        }
        
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
            alert = "Connection failed. Please check your server URL.";
            console.error("Error Initializing NATS Connection:", error);
        }
    }

    function handleKeyPress(event: KeyboardEvent) {
        if (event.key === 'Enter') {
            connect();
        }
    }
</script>

<div class="min-h-screen bg-gradient-to-br from-gray-900 via-gray-800 to-black flex items-center justify-center p-4">
    <!-- Background Pattern -->
    <div class="absolute inset-0 opacity-5">
        <div class="absolute inset-0" style="background-image: radial-gradient(circle at 25% 25%, #CEB888 2px, transparent 2px); background-size: 50px 50px;"></div>
    </div>
    
    <!-- Main Login Card -->
    <div class="relative w-full max-w-md">
        <!-- Logo/Brand Section -->
        <div class="text-center mb-8">
            <div class="inline-flex items-center justify-center w-20 h-20 bg-gradient-to-br from-yellow-400 to-yellow-600 rounded-full mb-6 shadow-2xl">
                <svg class="w-10 h-10 text-white" fill="currentColor" viewBox="0 0 20 20">
                    <path d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
                </svg>
            </div>
            <h1 class="text-4xl font-bold text-white mb-2">Avena-OTR</h1>
            <p class="text-gray-300 text-lg">Roadside Infrastructure Dashboard</p>
        </div>

        <!-- Login Form Card -->
        <div class="bg-white/10 backdrop-blur-lg rounded-2xl p-8 shadow-2xl border border-white/20">
            <div class="text-center mb-6">
                <h2 class="text-2xl font-semibold text-white mb-2">Connect to NATS Server</h2>
                <p class="text-gray-300 text-sm">Enter your server details to access the dashboard</p>
            </div>

            <!-- Alert Message -->
            {#if alert}
                <div class="mb-6 p-4 bg-red-500/20 border border-red-500/30 rounded-lg">
                    <div class="flex items-center">
                        <svg class="w-5 h-5 text-red-400 mr-2" fill="currentColor" viewBox="0 0 20 20">
                            <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd"/>
                        </svg>
                        <span class="text-red-300 text-sm">{alert}</span>
                    </div>
                </div>
            {/if}

            <!-- Form Fields -->
            <form on:submit|preventDefault={connect} class="space-y-6">
                <!-- Server URL Field -->
                <div>
                    <label for="server" class="block text-sm font-medium text-gray-300 mb-2">
                        Server URL
                    </label>
                    <div class="relative">
                        <div class="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                            <svg class="h-5 w-5 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9"/>
                            </svg>
                        </div>
                        <input
                            id="server"
                            type="text"
                            placeholder="ws://localhost:4443"
                            bind:value={serverName}
                            on:keypress={handleKeyPress}
                            class="w-full pl-10 pr-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                            required
                        />
                    </div>
                    <p class="mt-1 text-xs text-gray-400">Example: ws://localhost:4443</p>
                </div>

                <!-- Password Field -->
                <div>
                    <label for="password" class="block text-sm font-medium text-gray-300 mb-2">
                        Password
                    </label>
                    <div class="relative">
                        <div class="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                            <svg class="h-5 w-5 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/>
                            </svg>
                        </div>
                        <input
                            id="password"
                            type="password"
                            placeholder="Leave empty for local development"
                            bind:value={password}
                            on:keypress={handleKeyPress}
                            class="w-full pl-10 pr-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-yellow-500/50 focus:border-yellow-500/50 transition-all duration-200"
                        />
                    </div>
                    <p class="mt-1 text-xs text-gray-400">Optional for local development</p>
                </div>

                <!-- Connect Button -->
                <button
                    type="submit"
                    disabled={loading}
                    class="w-full py-3 px-4 bg-gradient-to-r from-yellow-500 to-yellow-600 hover:from-yellow-600 hover:to-yellow-700 disabled:from-gray-600 disabled:to-gray-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] disabled:transform-none disabled:cursor-not-allowed shadow-lg hover:shadow-xl"
                >
                    {#if loading}
                        <div class="flex items-center justify-center">
                            <svg class="animate-spin -ml-1 mr-3 h-5 w-5 text-white" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"></path>
                            </svg>
                            Connecting...
                        </div>
                    {:else}
                        <div class="flex items-center justify-center">
                            <svg class="w-5 h-5 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
                            </svg>
                            Connect to Dashboard
                        </div>
                    {/if}
                </button>
            </form>

            <!-- Help Text -->
            <div class="mt-6 text-center">
                <p class="text-xs text-gray-400">
                    Need help? Check the 
                    <a href="#" class="text-yellow-400 hover:text-yellow-300 underline">documentation</a>
                </p>
            </div>
        </div>

        <!-- Footer -->
        <div class="text-center mt-8">
            <p class="text-gray-500 text-sm">
                Advanced Vehicle Network Architecture - Off-The-Road Monitoring System
            </p>
        </div>
    </div>
</div>

<style>
    /* Custom scrollbar for webkit browsers */
    ::-webkit-scrollbar {
        width: 8px;
    }
    
    ::-webkit-scrollbar-track {
        background: rgba(255, 255, 255, 0.1);
        border-radius: 4px;
    }
    
    ::-webkit-scrollbar-thumb {
        background: rgba(206, 184, 136, 0.5);
        border-radius: 4px;
    }
    
    ::-webkit-scrollbar-thumb:hover {
        background: rgba(206, 184, 136, 0.7);
    }
    
    /* Smooth transitions */
    * {
        transition-property: color, background-color, border-color, text-decoration-color, fill, stroke, opacity, box-shadow, transform, filter, backdrop-filter;
        transition-timing-function: cubic-bezier(0.4, 0, 0.2, 1);
        transition-duration: 150ms;
    }
</style>