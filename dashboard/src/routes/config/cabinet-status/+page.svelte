<script lang="ts">
    import { onMount } from "svelte";
    import { goto } from "$app/navigation";
    import { NatsService, connect, putKeyValue, getKeyValue, getKeys } from "$lib/nats.svelte";
    import Alert from "$lib/components/Alert.svelte";
  
    type CabinetStatus = {
      id: string;
      status: 'online' | 'offline' | 'maintenance';
      lastUpdated: string;
      deviceCount: number;
    }
  
    let serverName: string | null = null;
    let nats: NatsService | null = null;
    let cabinets = $state<CabinetStatus[]>([]);
    let loading = $state<boolean>(true);
    let alert = $state<string | null>(null);
    let updatingStatus = $state<string | null>(null);
  
    // Initialize connection and load cabinet data
    async function initialize() {
      try {
        if (serverName) nats = await connect(serverName);
        if (nats) {
          await loadCabinetStatuses();
          loading = false;
        } else {
          console.log('No NATS Connection');
          loading = false;
        }
      } catch (error) {
        console.error("Initialization failed:", error);
        loading = false;
      }
    }
  
    // Load all cabinet statuses
    async function loadCabinetStatuses() {
      if (!nats) return;
  
      try {
        const cabinetKeys = await getKeys(nats, "all_cabinets", "*");
        const loadedCabinets: CabinetStatus[] = [];
  
        for (const key of cabinetKeys) {
          try {
            const statusData = await getKeyValue(nats, "all_cabinets", key);
            const cabinetData = JSON.parse(statusData);
            
            // Get device count for this cabinet
            let deviceCount = 0;
            try {
              const deviceKeys = await getKeys(nats, key, "labjackd.config.*");
              deviceCount = deviceKeys.length;
            } catch (error) {
              console.log(`No devices found for ${key}`);
            }
  
            loadedCabinets.push({
              id: key,
              status: cabinetData.status || 'offline',
              lastUpdated: new Date().toISOString(),
              deviceCount
            });
          } catch (error) {
            console.error(`Failed to load status for ${key}:`, error);
          }
        }
  
        cabinets = loadedCabinets;
      } catch (error) {
        console.error("Failed to load cabinet statuses:", error);
      }
    }
  
    // Change cabinet status
    async function changeCabinetStatus(cabinetId: string, newStatus: 'online' | 'offline' | 'maintenance') {
      if (!nats) return;
  
      try {
        updatingStatus = cabinetId;
        
        // Update the status in NATS
        const statusData = { status: newStatus };
        await putKeyValue(nats, "all_cabinets", cabinetId, JSON.stringify(statusData));
        
        // Update local state
        const cabinetIndex = cabinets.findIndex(cab => cab.id === cabinetId);
        if (cabinetIndex !== -1) {
          cabinets[cabinetIndex].status = newStatus;
          cabinets[cabinetIndex].lastUpdated = new Date().toISOString();
          cabinets = [...cabinets]; // Trigger reactivity
        }
  
        alert = `Cabinet ${cabinetId} status changed to ${newStatus}`;
        
        // Clear alert after 3 seconds
        setTimeout(() => {
          alert = null;
        }, 3000);
  
      } catch (error) {
        console.error(`Failed to change status for ${cabinetId}:`, error);
        alert = `Failed to change status for ${cabinetId}`;
      } finally {
        updatingStatus = null;
      }
    }
  
    // Get status color and icon
    function getStatusInfo(status: string) {
      switch (status) {
        case 'online':
          return {
            color: 'text-green-400',
            bgColor: 'bg-green-500/20',
            borderColor: 'border-green-500/30',
            icon: '🟢',
            label: 'Online'
          };
        case 'offline':
          return {
            color: 'text-red-400',
            bgColor: 'bg-red-500/20',
            borderColor: 'border-red-500/30',
            icon: '🔴',
            label: 'Offline'
          };
        case 'maintenance':
          return {
            color: 'text-yellow-400',
            bgColor: 'bg-yellow-500/20',
            borderColor: 'border-yellow-500/30',
            icon: '🟡',
            label: 'Maintenance'
          };
        default:
          return {
            color: 'text-gray-400',
            bgColor: 'bg-gray-500/20',
            borderColor: 'border-gray-500/30',
            icon: '⚪',
            label: 'Unknown'
          };
      }
    }
  
    // Get display name for cabinet
    function getDisplayName(id: string) {
      return id.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase());
    }
  
    // Refresh cabinet data
    async function refreshData() {
      loading = true;
      await loadCabinetStatuses();
      loading = false;
    }
  
    onMount(() => {
      serverName = sessionStorage.getItem("serverName");
      if (!serverName) goto("/");
      initialize();
    });
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
            <div class="flex items-center space-x2">
              <div class="w-2 h-2 bg-green-400 rounded-full animate-pulse"></div>
              <span class="text-sm text-gray-300">Connected to NATS</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  
    <!-- Main Content -->
    <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
      <!-- Page Header -->
      <div class="text-center mb-8">
        <h1 class="text-4xl font-bold text-white mb-4">
          Cabinet Status Management
        </h1>
        <p class="text-xl text-gray-300 max-w-3xl mx-auto">
          Monitor and control the operational status of all Avena cabinets. Change statuses to manage access levels and maintenance schedules.
        </p>
      </div>
  
      <!-- Navigation and Actions Bar -->
      <div class="flex flex-col sm:flex-row items-center justify-between mb-8 p-4 bg-white/5 backdrop-blur-lg rounded-xl border border-white/10">
        <div class="flex items-center space-x-4 mb-4 sm:mb-0">
          <button 
            onclick={() => goto("/config/cabinet-select")}
            class="flex items-center space-x-2 px-4 py-2 bg-gray-700 hover:bg-gray-600 text-white rounded-lg transition-colors duration-200"
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
            </svg>
            <span>Back to Cabinet Selection</span>
          </button>
          
          <button 
            onclick={() => goto("/config/lj-config")}
            class="flex items-center space-x-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors duration-200"
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 20l-5.447-2.724A1 1 0 013 16.382V5.618a1 1 0 011.447-.894L9 7m0 13l6-3m-6 3V7m6 10l4.553 2.276A1 1 0 0021 18.382V7.618a1 1 0 00-1.447-.894L15 4m0 13V4m0 0L9 7"/>
            </svg>
            <span>LabJack Configuration</span>
          </button>
        </div>
  
        <button 
          onclick={refreshData}
          disabled={loading}
          class="flex items-center space-x-2 px-6 py-3 bg-gradient-to-r from-green-500 to-green-600 hover:from-green-600 hover:to-green-700 text-white font-semibold rounded-lg transition-all duration-200 transform hover:scale-[1.02] shadow-lg hover:shadow-xl disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
          </svg>
          <span>Refresh Data</span>
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
            <p class="text-gray-400 text-lg">Loading cabinet statuses...</p>
          </div>
        </div>
      {:else if cabinets.length === 0}
        <!-- No Cabinets State -->
        <div class="text-center py-20">
          <div class="inline-flex items-center justify-center w-20 h-20 bg-gray-500/20 rounded-full mb-6">
            <svg class="w-10 h-10 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4"/>
            </svg>
          </div>
          <h3 class="text-xl font-semibold text-white mb-2">No Cabinets Found</h3>
          <p class="text-gray-400 mb-6">No cabinets are currently configured in the system.</p>
        </div>
      {:else}
        <!-- Cabinet Status Grid -->
        <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {#each cabinets as cabinet}
            {@const statusInfo = getStatusInfo(cabinet.status)}
            
            <div class="group relative bg-white/5 backdrop-blur-lg rounded-2xl p-6 border border-white/10 hover:border-yellow-500/30 transition-all duration-300 hover:transform hover:scale-[1.02] hover:shadow-2xl hover:shadow-yellow-500/10">
              <!-- Status Badge -->
              <div class="absolute top-4 right-4">
                <div class="flex items-center space-x-2 px-3 py-1.5 rounded-full {statusInfo.bgColor} border {statusInfo.borderColor}">
                  <span class="text-lg">{statusInfo.icon}</span>
                  <span class="text-xs font-medium {statusInfo.color}">
                    {statusInfo.label}
                  </span>
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
                <h3 class="text-xl font-semibold text-white mb-2">{getDisplayName(cabinet.id)}</h3>
                <p class="text-gray-400 text-sm">ID: {cabinet.id}</p>
              </div>
  
              <!-- Device Count -->
              <div class="mb-6 p-4 bg-gray-800/30 rounded-lg border border-gray-700/50">
                <div class="flex items-center justify-between">
                  <span class="text-sm text-gray-400">LabJack Devices:</span>
                  <span class="text-sm font-medium text-white">{cabinet.deviceCount}</span>
                </div>
              </div>
  
              <!-- Status Change Dropdown -->
              <div class="mb-6">
                <label for={`status-${cabinet.id}`} class="block text-sm font-medium text-gray-300 mb-2">
                  Change Status:
                </label>
                <select
                  id={`status-${cabinet.id}`}
                  value={cabinet.status}
                  onchange={(e) => {
                    const target = e.target as HTMLSelectElement;
                    if (target) {
                      changeCabinetStatus(cabinet.id, target.value as 'online' | 'offline' | 'maintenance');
                    }
                  }}
                  disabled={updatingStatus === cabinet.id}
                  class="w-full px-4 py-3 bg-gray-800/50 border border-gray-600/50 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  <option value="online" class="bg-gray-800 text-white">🟢 Online</option>
                  <option value="offline" class="bg-gray-800 text-white">🔴 Offline</option>
                  <option value="maintenance" class="bg-gray-800 text-white">🟡 Maintenance</option>
                </select>
              </div>
  
              <!-- Last Updated -->
              <div class="text-center">
                <p class="text-xs text-gray-500">
                  Last updated: {new Date(cabinet.lastUpdated).toLocaleString()}
                </p>
              </div>
  
              <!-- Loading Indicator -->
              {#if updatingStatus === cabinet.id}
                <div class="absolute inset-0 bg-black/50 rounded-2xl flex items-center justify-center">
                  <div class="text-center">
                    <div class="inline-flex items-center justify-center w-8 h-8 bg-blue-500 rounded-full mb-2 animate-spin">
                      <svg class="w-4 h-4 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                      </svg>
                    </div>
                    <p class="text-xs text-white">Updating...</p>
                  </div>
                </div>
              {/if}
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
              Use the dropdown menus to change cabinet statuses. Changes take effect immediately and affect access levels across the dashboard.
            </span>
          </div>
        </div>
      {/if}
    </div>
  </div>
  
  <Alert bind:alert={alert}/>
  
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