<script lang="ts">
  import { onMount } from "svelte";
  import AlertsList from "../components/AlertsList.svelte";
  import { darkMode } from "../lib/stores";
  import { api, type Alert } from "../lib/api";
  import { notifications } from "../lib/stores";
  import { features } from "../lib/features.svelte";

  let alerts: Alert[] = [];
  let loading = true;
  let error: string | null = null;

  async function fetchAlerts() {
    try {
      const response = await api.getRecentAlerts();
      alerts = response.alerts || [];
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to fetch alerts";
      console.error("Alerts fetch error:", e);
    } finally {
      loading = false;
    }
  }

  async function acknowledgeAllAlerts() {
    if (!features.admin) {
      notifications.add({
        type: "error",
        message: "Admin feature is not enabled on this server",
      });
      return;
    }
    try {
      await api.acknowledgeAllAlerts();
      alerts = alerts.map((a) => ({ ...a, acknowledged: true }));
      notifications.add({
        type: "success",
        message: "All alerts acknowledged",
      });
    } catch (e) {
      notifications.add({
        type: "error",
        message:
          e instanceof Error ? e.message : "Failed to acknowledge alerts",
      });
    }
  }

  async function acknowledgeAlert(id: string) {
    if (!features.admin) {
      notifications.add({
        type: "error",
        message: "Admin feature is not enabled on this server",
      });
      return;
    }
    try {
      await api.acknowledgeAlert(id);
      alerts = alerts.map((a) =>
        a.id === id ? { ...a, acknowledged: true } : a,
      );
      notifications.add({
        type: "success",
        message: "Alert acknowledged",
      });
    } catch (e) {
      notifications.add({
        type: "error",
        message: e instanceof Error ? e.message : "Failed to acknowledge alert",
      });
    }
  }

  async function clearAllAlerts() {
    if (!features.admin) {
      notifications.add({
        type: "error",
        message: "Admin feature is not enabled on this server",
      });
      return;
    }
    try {
      await api.clearAlerts();
      alerts = [];
      notifications.add({
        type: "success",
        message: "All alerts cleared",
      });
    } catch (e) {
      notifications.add({
        type: "error",
        message: e instanceof Error ? e.message : "Failed to clear alerts",
      });
    }
  }

  onMount(() => {
    fetchAlerts();
    // Refresh every 5 seconds
    const interval = setInterval(fetchAlerts, 5000);
    return () => clearInterval(interval);
  });
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-2xl font-bold {$darkMode ? 'text-white' : 'text-gray-900'}">
      All Alerts
    </h1>
    <p class="{$darkMode ? 'text-gray-400' : 'text-gray-700'} mt-1">
      View all security and system alerts
    </p>
  </div>

  {#if error}
    <div class="card bg-red-500/10 border border-red-500/50 p-4 text-red-500">
      <p>Error: {error}</p>
    </div>
  {:else if loading}
    <div class="card p-8 text-center">
      <div class="inline-block">
        <div
          class="w-8 h-8 border-4 {$darkMode
            ? 'border-gray-700 border-t-primary-500'
            : 'border-gray-300 border-t-primary-600'} rounded-full animate-spin"
        ></div>
      </div>
    </div>
  {:else}
    <div class="card">
      <div
        class="card-header flex items-center justify-between bg-white {$darkMode
          ? 'dark:bg-gray-800'
          : 'bg-gray-50'}"
      >
        <h3
          class="font-semibold {$darkMode
            ? 'text-white'
            : 'text-gray-900'} flex items-center gap-2"
        >
          <svg
            class="w-5 h-5 {$darkMode ? 'text-gray-400' : 'text-gray-500'}"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
            />
          </svg>
          All Alerts ({alerts.length})
        </h3>
        <div class="flex gap-2">
          <button
            on:click={clearAllAlerts}
            disabled={!features.admin}
            class="px-3 py-1.5 text-xs font-medium rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed {$darkMode
              ? 'bg-red-900/30 hover:bg-red-900/50 text-red-400'
              : 'bg-red-100 hover:bg-red-200 text-red-700'}"
          >
            Clear All
          </button>
          <button
            on:click={acknowledgeAllAlerts}
            disabled={!features.admin}
            class="px-3 py-1.5 text-xs font-medium rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed {$darkMode
              ? 'bg-blue-900/30 hover:bg-blue-900/50 text-blue-400'
              : 'bg-blue-100 hover:bg-blue-200 text-blue-700'}"
          >
            Acknowledge All
          </button>
        </div>
      </div>
      <div class="p-4">
        <AlertsList
          {alerts}
          limit={alerts.length}
          showAcknowledgeButton={true}
          adminEnabled={features.admin}
          showViewAllLink={false}
          on:acknowledge={(e) => acknowledgeAlert(e.detail)}
        />
      </div>
    </div>
  {/if}
</div>
