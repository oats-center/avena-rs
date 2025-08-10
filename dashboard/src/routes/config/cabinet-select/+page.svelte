<script lang="ts">
    import { onMount } from "svelte";
    import {
        NatsService,
        connect,
        getKeys,
        getKeyValue,
    } from "$lib/nats.svelte";
    import { goto } from "$app/navigation";

    type Cabinet = {
        id: string;
        status: string;
    };

    let serverName: string | null;
    let nats: NatsService | null;
    let cabinetKeys = $state<string[]>([]);
    let cabinets = $state<Cabinet[]>([]);
    let loading = $state<boolean>(true);

    //initializes the nats connection and gets all cabinet vals
    async function initialize() {
        if (serverName) nats = await connect(serverName);
        if (nats) {
            let keys = await getKeys(nats, "all_cabinets");
            for (const key of keys) {
                let values = await getCabinet("all_cabinets", key);
                cabinets.push(values);
                console.log(values);
            }
            loading = false;
        } else {
            console.log("No Nats Connection");
        }
    }

    //gets the value of one cabinet
    async function getCabinet(bucket: string, key: string): Promise<Cabinet> {
        if (!nats) throw new Error("Nats connection is not initialized");
        let cabinet = {
            id: key,
            status: "",
        };
        let status = await getKeyValue(nats, bucket, key);
        cabinet.status = JSON.parse(status).status;
        return cabinet;
    }

    //once selected, sets the selectedCabinet to session storage and redirects to labjack config page
    function selectConfig(selectedCabinet: string) {
        sessionStorage.setItem("selectedCabinet", selectedCabinet);
        goto("/config/lj-config");
    }

    //gets the server name from session storage & initalizes
    onMount(() => {
        serverName = sessionStorage.getItem("serverName");
        initialize();
    });

    // Get status color and icon
    function getStatusInfo(status: string) {
        switch (status.toLowerCase()) {
            case 'online':
                return {
                    color: 'text-green-400',
                    bgColor: 'bg-green-500/20',
                    borderColor: 'border-green-500/30',
                    icon: 'M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z',
                    label: 'Online'
                };
            case 'offline':
                return {
                    color: 'text-red-400',
                    bgColor: 'bg-red-500/20',
                    borderColor: 'border-red-500/30',
                    icon: 'M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z',
                    label: 'Offline'
                };
            case 'maintenance':
                return {
                    color: 'text-yellow-400',
                    bgColor: 'bg-yellow-500/20',
                    borderColor: 'border-yellow-500/30',
                    icon: 'M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L3.732 16.5c-.77.833.192 2.5 1.732 2.5z',
                    label: 'Maintenance'
                };
            default:
                return {
                    color: 'text-gray-400',
                    bgColor: 'bg-gray-500/20',
                    borderColor: 'border-gray-500/30',
                    icon: 'M8.228 9c.549-1.165 2.03-2 3.772-2 2.21 0 4 1.343 4 3 0 1.4-1.278 2.575-3.006 2.907-.542.104-.994.54-.994 1.093m0 3h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z',
                    label: 'Unknown'
                };
        }
    }

    // Get cabinet display name
    function getDisplayName(id: string) {
        return id.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase());
    }
</script>

<div class="min-h-screen bg-gradient-to-br from-gray-900 via-gray-800 to-black">
    <!-- Header -->
    <div class="bg-white/5 backdrop-blur-lg border-b border-white/10">
        <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
            <div class="flex items-center justify-between h-16">
                <!-- Logo and Title -->
                <div class="flex items-center space-x-4">
                    <div class="w-8 h-8 bg-gradient-to-br from-yellow-400 to-yellow-600 rounded-lg flex items-center justify-center">
                        <svg class="w-5 h-5 text-white" fill="currentColor" viewBox="0 0 20 20">
                            <path d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
                        </svg>
                    </div>
                    <h1 class="text-xl font-semibold text-white">Avena-OTR Dashboard</h1>
                </div>
                
                <!-- Connection Status -->
                <div class="flex items-center space-x-3">
                    <div class="flex items-center space-x-2">
                        <div class="w-2 h-2 bg-green-400 rounded-full animate-pulse"></div>
                        <span class="text-sm text-gray-300">Connected to NATS</span>
                    </div>
                    <button 
                        onclick={() => goto('/')}
                        class="px-3 py-1.5 text-sm text-gray-300 hover:text-white hover:bg-white/10 rounded-lg transition-colors duration-200"
                    >
                        Disconnect
                    </button>
                </div>
            </div>
        </div>
    </div>

    <!-- Main Content -->
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <!-- Page Header -->
        <div class="text-center mb-8">
            <h1 class="text-4xl font-bold text-white mb-4">Select Avena Box</h1>
            <p class="text-xl text-gray-300 max-w-2xl mx-auto">
                Choose an Avena box to configure its LabJack devices and monitor sensor data
            </p>
        </div>

        <!-- Navigation and Actions Bar -->
        <div class="flex flex-col sm:flex-row items-center justify-center mb-8 p-4 bg-white/5 backdrop-blur-lg rounded-xl border border-white/10">
            <button 
                onclick={() => goto("/config/cabinet-status")}
                class="flex items-center space-x-2 px-6 py-3 bg-gradient-to-r from-blue-500 to-blue-600 hover:from-blue-600 hover:to-blue-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl"
            >
                <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>
                </svg>
                <span>Manage Cabinet Statuses</span>
            </button>
        </div>

        {#if loading}
            <!-- Loading State -->
            <div class="flex items-center justify-center py-20">
                <div class="text-center">
                    <div class="inline-flex items-center justify-center w-16 h-16 bg-gradient-to-br from-yellow-400 to-yellow-600 rounded-full mb-6 animate-pulse">
                        <svg class="w-8 h-8 text-white animate-spin" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                        </svg>
                    </div>
                    <p class="text-gray-400 text-lg">Loading Avena boxes...</p>
                </div>
            </div>
        {:else if cabinets && cabinets.length > 0}
            <!-- Cabinet Grid -->
            <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6 max-w-6xl mx-auto">
                {#each cabinets as cabinet}
                    {@const statusInfo = getStatusInfo(cabinet.status)}
                    {@const displayName = getDisplayName(cabinet.id)}
                    
                    <div class="group relative bg-white/5 backdrop-blur-lg rounded-2xl p-6 border border-white/10 hover:border-yellow-500/30 transition-all duration-300 hover:transform hover:scale-[1.02] hover:shadow-2xl hover:shadow-yellow-500/10">
                        <!-- Status Badge -->
                        <div class="absolute top-4 right-4">
                            <div class="flex items-center space-x-2 px-3 py-1.5 rounded-full {statusInfo.bgColor} border {statusInfo.borderColor}">
                                <svg class="w-4 h-4 {statusInfo.color}" fill="currentColor" viewBox="0 0 20 20">
                                    <path fill-rule="evenodd" d="{statusInfo.icon}" clip-rule="evenodd"/>
                                </svg>
                                <span class="text-xs font-medium {statusInfo.color}">{statusInfo.label}</span>
                            </div>
                        </div>

                        <!-- Cabinet Icon -->
                        <div class="flex items-center justify-center w-16 h-16 bg-gradient-to-br from-blue-500/20 to-purple-500/20 rounded-2xl mb-6 border border-blue-500/30">
                            <svg class="w-8 h-8 text-blue-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4"/>
                            </svg>
                        </div>

                        <!-- Cabinet Info -->
                        <div class="text-center mb-6">
                            <h3 class="text-xl font-semibold text-white mb-2">{displayName}</h3>
                            <p class="text-gray-400 text-sm">Roadside Infrastructure Unit</p>
                        </div>

                        <!-- Status Details -->
                        <div class="mb-6 p-4 bg-gray-800/30 rounded-lg border border-gray-700/50">
                            <div class="flex items-center justify-between">
                                <span class="text-sm text-gray-400">Current Status:</span>
                                <span class="text-sm font-medium {statusInfo.color}">{statusInfo.label}</span>
                            </div>
                            <div class="flex items-center justify-between mt-2">
                                <span class="text-sm text-gray-400">Device Type:</span>
                                <span class="text-sm text-gray-300">Avena Box</span>
                            </div>
                        </div>

                        <!-- Action Button -->
                        <button
                            onclick={() => selectConfig(cabinet.id)}
                            class="w-full py-3 px-4 bg-gradient-to-r from-yellow-500 to-yellow-600 hover:from-yellow-600 hover:to-yellow-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl hover:shadow-yellow-500/25"
                        >
                            <div class="flex items-center justify-center space-x-2">
                                <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"/>
                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
                                </svg>
                                <span>Configure LabJack Devices</span>
                            </div>
                        </button>
                    </div>
                {/each}
            </div>

            <!-- Help Text -->
            <div class="mt-12 text-center">
                <div class="inline-flex items-center space-x-2 px-4 py-2 bg-blue-500/20 border border-blue-500/30 rounded-lg">
                    <svg class="w-5 h-5 text-blue-400" fill="currentColor" viewBox="0 0 20 20">
                        <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z" clip-rule="evenodd"/>
                    </svg>
                    <span class="text-blue-300 text-sm">
                        Select an Avena box to configure its LabJack devices and sensor settings
                    </span>
                </div>
            </div>
        {:else}
            <!-- No Cabinets State -->
            <div class="text-center py-20">
                <div class="inline-flex items-center justify-center w-20 h-20 bg-gray-500/20 rounded-full mb-6">
                    <svg class="w-10 h-10 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4"/>
                    </svg>
                </div>
                <h3 class="text-xl font-semibold text-white mb-2">No Avena Boxes Found</h3>
                <p class="text-gray-400 mb-6">No Avena boxes are currently available in the system.</p>
                <button 
                    onclick={() => window.location.reload()}
                    class="px-4 py-2 bg-yellow-500 hover:bg-yellow-600 text-white rounded-lg transition-colors duration-200"
                >
                    Refresh Page
                </button>
            </div>
        {/if}
    </div>
</div>

<style>
    /* Custom scrollbar */
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
