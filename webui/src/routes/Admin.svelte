<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, type ConfigDumpResponse } from "../lib/api";
  import { darkMode } from "../lib/stores";
  import ManagementTab from "../components/ManagementTab.svelte";
  import ConfigTab from "../components/ConfigTab.svelte";

  // Active tab: "admin" (server management) or "config" (configuration view)
  type TabType = "admin" | "config";
  let activeTab: TabType = "admin";

  // Configuration view state (owned here, passed down to ConfigTab).
  let configDump: ConfigDumpResponse | null = null;
  let configError: string | null = null;
  // Error from the last reload attempt (persistent banner on Configuration tab).
  let reloadError: string | null = null;

  let managementTab: ManagementTab;

  async function fetchConfigDump() {
    try {
      configDump = await api.getConfigDump();
      configError = null;
    } catch (e) {
      configError = e instanceof Error ? e.message : "Failed to load configuration";
      console.error("Config fetch error:", e);
    }
  }

  // ManagementTab reports reload outcomes so the Configuration tab can refresh
  // (success) or surface a persistent error banner (failure).
  async function onReloadSuccess() {
    await fetchConfigDump();
    reloadError = null;
  }

  function onReloadError(e: CustomEvent<{ message: string }>) {
    reloadError = e.detail.message;
  }

  let refreshInterval: ReturnType<typeof setInterval>;
  onMount(() => {
    fetchConfigDump();
    // Periodically refresh the config dump in case it is reloaded out of band.
    refreshInterval = setInterval(fetchConfigDump, 30000);
  });

  onDestroy(() => {
    if (refreshInterval) clearInterval(refreshInterval);
  });
</script>

<div class="space-y-6">
  <!-- Page Header -->
  <div class="flex items-center justify-between">
    <div>
      <h1
        class="text-2xl font-bold {$darkMode ? 'text-white' : 'text-gray-900'}"
      >
        Admin
      </h1>
      <p class="{$darkMode ? 'text-gray-400' : 'text-gray-700'} mt-1">
        Server management and configuration
      </p>
    </div>
  </div>

  <!-- Tabs -->
  <div
    class="flex items-center gap-1 border-b {$darkMode
      ? 'border-gray-700'
      : 'border-gray-200'}"
  >
    <button
      on:click={() => (activeTab = "admin")}
      class="px-4 py-3 text-sm font-medium transition-colors border-b-2 -mb-px"
      class:border-primary-500={activeTab === "admin"}
      class:text-primary-500={activeTab === "admin"}
      class:border-transparent={activeTab !== "admin"}
      class:text-gray-400={activeTab !== "admin" && $darkMode}
      class:text-gray-600={activeTab !== "admin" && !$darkMode}
    >
      <span class="flex items-center gap-2">
        <svg
          class="w-4 h-4"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
          />
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
          />
        </svg>
        Management
      </span>
    </button>
    <button
      on:click={() => (activeTab = "config")}
      class="px-4 py-3 text-sm font-medium transition-colors border-b-2 -mb-px"
      class:border-primary-500={activeTab === "config"}
      class:text-primary-500={activeTab === "config"}
      class:border-transparent={activeTab !== "config"}
      class:text-gray-400={activeTab !== "config" && $darkMode}
      class:text-gray-600={activeTab !== "config" && !$darkMode}
    >
      <span class="flex items-center gap-2">
        <svg
          class="w-4 h-4"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
          />
        </svg>
        Configuration
        {#if configDump}
          <span
            class="px-2 py-0.5 rounded-full text-xs {$darkMode
              ? 'bg-gray-700'
              : 'bg-gray-200 text-gray-700'}"
          >
            {configDump.plugin_count}
          </span>
        {/if}
      </span>
    </button>
  </div>

  {#if activeTab === "admin"}
    <ManagementTab
      bind:this={managementTab}
      on:reloadsuccess={onReloadSuccess}
      on:reloaderror={onReloadError}
    />
  {:else}
    <ConfigTab
      {configDump}
      {configError}
      {reloadError}
      on:dismissreloaderror={() => (reloadError = null)}
    />
  {/if}
</div>
