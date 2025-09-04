<script lang="ts">
  import { onMount } from 'svelte';
  
  let deferredPrompt: any = null;
  let showInstallPrompt = $state(false);
  let isInstalled = $state(false);
  let isFirefox = $state(false);
  let isSafari = $state(false);
  let isMacOS = $state(false);

  onMount(() => {
    // Check if already installed
    if (window.matchMedia('(display-mode: standalone)').matches) {
      isInstalled = true;
      return;
    }

    // Detect browser type and OS
    const userAgent = navigator.userAgent.toLowerCase();
    isFirefox = userAgent.includes('firefox');
    const isChrome = userAgent.includes('chrome');
    isSafari = userAgent.includes('safari') && !isChrome;
    isMacOS = userAgent.includes('mac os x') || userAgent.includes('macintosh');

    // Listen for beforeinstallprompt event (Chrome/Edge only)
    window.addEventListener('beforeinstallprompt', (e) => {
      e.preventDefault();
      deferredPrompt = e;
      showInstallPrompt = true;
    });

    // Listen for app installed event
    window.addEventListener('appinstalled', () => {
      isInstalled = true;
      showInstallPrompt = false;
      deferredPrompt = null;
    });

    // For Firefox and Safari, show manual install instructions after a delay
    if (isFirefox || (isSafari && isMacOS)) {
      setTimeout(() => {
        if (!isInstalled && !showInstallPrompt) {
          showInstallPrompt = true;
        }
      }, 10000); // Show after 10 seconds
    }
  });

  async function installApp() {
    if (!deferredPrompt) return;
    
    deferredPrompt.prompt();
    const { outcome } = await deferredPrompt.userChoice;
    
    if (outcome === 'accepted') {
      showInstallPrompt = false;
    }
    
    deferredPrompt = null;
  }

  function dismissPrompt() {
    showInstallPrompt = false;
  }
</script>

{#if showInstallPrompt && !isInstalled}
  <div class="fixed bottom-4 right-4 z-50 max-w-sm">
    <div class="bg-gradient-to-r from-yellow-500 to-yellow-600 rounded-lg shadow-lg p-4 text-white">
      <div class="flex items-start space-x-3">
        <div class="flex-shrink-0">
          <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z"/>
          </svg>
        </div>
        <div class="flex-1">
          <h3 class="text-sm font-semibold">
            {#if isFirefox}
              Bookmark Avena Dashboard
            {:else if isSafari && isMacOS}
              Add to Dock - Avena Dashboard
            {:else}
              Install Avena Dashboard
            {/if}
          </h3>
          <p class="text-xs opacity-90 mt-1">
            {#if isFirefox}
              Bookmark this page or add to Firefox shortcuts for quick access
            {:else if isSafari && isMacOS}
              Add to Dock from Safari's File menu for app-like experience
            {:else}
              Add to your home screen for quick access
            {/if}
          </p>
          <div class="flex space-x-2 mt-3">
            {#if !isFirefox && !isSafari && deferredPrompt}
              <button 
                onclick={installApp}
                class="px-3 py-1 bg-white/20 hover:bg-white/30 rounded text-xs font-medium transition-colors"
              >
                Install
              </button>
            {:else if isFirefox}
              <button 
                onclick={() => window.open('https://support.mozilla.org/en-US/kb/how-do-i-create-desktop-shortcut-website', '_blank')}
                class="px-3 py-1 bg-white/20 hover:bg-white/30 rounded text-xs font-medium transition-colors"
              >
                Learn How
              </button>
            {:else if isSafari && isMacOS}
              <button 
                onclick={() => window.open('https://support.apple.com/guide/safari/add-a-website-to-the-dock-sfri40734/mac', '_blank')}
                class="px-3 py-1 bg-white/20 hover:bg-white/30 rounded text-xs font-medium transition-colors"
              >
                Show Steps
              </button>
            {/if}
            <button 
              onclick={dismissPrompt}
              class="px-3 py-1 bg-white/10 hover:bg-white/20 rounded text-xs font-medium transition-colors"
            >
              {#if isFirefox}Got it{:else}Later{/if}
            </button>
          </div>
        </div>
        <button 
          onclick={dismissPrompt}
          class="flex-shrink-0 p-1 hover:bg-white/20 rounded"
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>
    </div>
  </div>
{/if}
