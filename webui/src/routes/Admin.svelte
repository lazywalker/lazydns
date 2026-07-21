<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { formatNumber, formatBytes, formatUptime } from "../lib/utils";
  import { api, type ConfigDumpResponse, type ConfigPluginSummary } from "../lib/api";
  import { notifications, darkMode } from "../lib/stores";
  import { features } from "../lib/features.svelte";

  // Active tab: "admin" (server management) or "config" (configuration view)
  type TabType = "admin" | "config";
  let activeTab: TabType = "admin";

  let configPath = "/etc/lazydns/config.yaml";
  let isReloading = false;
  let isClearingCache = false;
  let isAcknowledgingAlerts = false;

  // Configuration view state
  let configDump: ConfigDumpResponse | null = null;
  let configError: string | null = null;
  // Expandable sequences: keyed by plugin tag (true = expanded).
  let expandedSequences: Record<string, boolean> = {};

  // Real data
  let serverInfo = {
    version: "0.3.1",
    status: "loading",
    uptime_secs: 0,
    total_queries: 0,
    cache_size: 0,
    rss_bytes: 0,
  };
  let cacheStats = {
    size: 0,
    hit_rate: 0,
    hits: 0,
    misses: 0,
    evictions: 0,
    expirations: 0,
  };
  let latencyStats = {
    p50_ms: 0,
    p95_ms: 0,
    p99_ms: 0,
    max_ms: 0,
    avg_ms: 0,
  };

  async function fetchData() {
    try {
      const [overviewRes, latencyRes, cacheStatsRes, serverInfoRes] =
        await Promise.all([
          api.getDashboardOverview(),
          api.getLatencyDistribution().catch(() => ({
            distribution: {
              buckets: [],
              total: 0,
              p50_ms: 0,
              p95_ms: 0,
              p99_ms: 0,
              max_ms: 0,
              avg_ms: 0,
            },
          })),
          api.getCacheStats().catch(() => ({
            size: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            expirations: 0,
            hit_rate: 0,
          })),
          api.getServerInfo().catch((err) => {
            console.warn("Failed to fetch server info:", err);
            return {
              version: "0.3.1",
              uptime_secs: 0,
              rss_bytes: 0,
            };
          }),
        ]);

      serverInfo = {
        version: serverInfoRes.version || "0.3.1",
        status: overviewRes.status,
        uptime_secs: overviewRes.uptime_secs,
        total_queries: overviewRes.metrics.total_queries,
        cache_size: cacheStatsRes.size,
        rss_bytes: serverInfoRes.rss_bytes || 0,
      };

      // Use actual cache stats from admin API
      cacheStats = {
        size: cacheStatsRes.size,
        hit_rate: cacheStatsRes.hit_rate,
        hits: cacheStatsRes.hits,
        misses: cacheStatsRes.misses,
        evictions: cacheStatsRes.evictions,
        expirations: cacheStatsRes.expirations,
      };

      // Latency stats
      latencyStats = {
        p50_ms: latencyRes.distribution.p50_ms ?? 0,
        p95_ms: latencyRes.distribution.p95_ms ?? 0,
        p99_ms: latencyRes.distribution.p99_ms ?? 0,
        max_ms: latencyRes.distribution.max_ms ?? 0,
        avg_ms: latencyRes.distribution.avg_ms ?? 0,
      };
    } catch (e) {
      console.error("Admin fetch error:", e);
    }
  }

  // Fetch the read-only configuration dump for the "Configuration" tab.
  async function fetchConfigDump() {
    try {
      configDump = await api.getConfigDump();
      configError = null;
    } catch (e) {
      configError = e instanceof Error ? e.message : "Failed to load configuration";
      console.error("Config fetch error:", e);
    }
  }

  function toggleSequence(tag: string) {
    expandedSequences[tag] = !expandedSequences[tag];
  }

  // Render a short one-line summary of a plugin's args based on common keys.
  function argsPreview(p: ConfigPluginSummary): string {
    // args_summary may be null when the plugin has no args (e.g. blackhole).
    const a = (p.args_summary ?? {}) as Record<string, unknown>;
    const parts: string[] = [];
    if (typeof a.size === "number") parts.push(`size=${a.size}`);
    if (typeof a.concurrent === "number") parts.push(`concurrent=${a.concurrent}`);
    const ups = a.upstreams;
    if (Array.isArray(ups)) parts.push(`${ups.length} upstreams`);
    if (Array.isArray(a.files)) parts.push(`${a.files.length} files`);
    if (typeof a.auto_reload === "boolean" && a.auto_reload) parts.push("auto-reload");
    if (typeof a.enabled === "boolean" && !a.enabled) parts.push("disabled");
    if (typeof a.entry === "string") parts.push(`entry=${a.entry}`);
    if (typeof a.listen === "string") parts.push(`listen=${a.listen}`);
    if (parts.length === 0 && Object.keys(a).length > 0) {
      parts.push(`${Object.keys(a).length} fields`);
    }
    return parts.join(" · ");
  }

  // Separated plugin groups for display.
  $: configServers = configDump
    ? configDump.plugins.filter((p) => isServer(p))
    : [];
  $: configSequences = configDump
    ? configDump.plugins.filter((p) => p.is_sequence)
    : [];
  $: configOtherPlugins = configDump
    ? configDump.plugins.filter((p) => !isServer(p) && !p.is_sequence)
    : [];

  function isServer(p: ConfigPluginSummary): boolean {
    return p.plugin_type.endsWith("_server") || p.plugin_type === "server";
  }

  async function reloadConfig() {
    isReloading = true;

    try {
      const result = await api.reloadConfig(configPath || undefined);
      notifications.add({
        type: "success",
        message: result.message,
      });
      // Refresh data after reload
      await fetchData();
      await fetchConfigDump();
    } catch (e) {
      notifications.add({
        type: "error",
        message:
          e instanceof Error ? e.message : "Failed to reload configuration",
      });
    } finally {
      isReloading = false;
    }
  }

  async function clearCache() {
    if (!confirm("Are you sure you want to clear the entire cache?")) return;

    isClearingCache = true;

    try {
      const result = await api.clearCache();
      notifications.add({
        type: "success",
        message: result.message,
      });
      // Refresh data after clearing
      await fetchData();
    } catch (e) {
      notifications.add({
        type: "error",
        message: e instanceof Error ? e.message : "Failed to clear cache",
      });
    } finally {
      isClearingCache = false;
    }
  }

  let isExportingLogs = false;

  async function acknowledgeAllAlerts() {
    if (!features.admin) {
      notifications.add({
        type: "error",
        message: "Admin feature is not enabled on this server",
      });
      return;
    }
    isAcknowledgingAlerts = true;
    try {
      await api.acknowledgeAllAlerts();
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
    } finally {
      isAcknowledgingAlerts = false;
    }
  }

  async function exportLogs(logType: string = "query", format: string = "csv") {
    if (isExportingLogs) return;
    isExportingLogs = true;
    try {
      const blob = await api.exportLogs(logType, format, 10000);
      const url = window.URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${logType}-logs-${new Date().toISOString().slice(0, 10)}.${format}`;
      document.body.appendChild(a);
      a.click();
      window.URL.revokeObjectURL(url);
      document.body.removeChild(a);
      notifications.add({
        type: "success",
        message: `Logs exported as ${format.toUpperCase()}`,
      });
    } catch (e) {
      notifications.add({
        type: "error",
        message: e instanceof Error ? e.message : "Failed to export logs",
      });
    } finally {
      isExportingLogs = false;
    }
  }

  let refreshInterval: ReturnType<typeof setInterval>;
  onMount(() => {
    fetchData();
    fetchConfigDump();
    refreshInterval = setInterval(fetchData, 10000);
  });

  onDestroy(() => {
    if (refreshInterval) {
      clearInterval(refreshInterval);
    }
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
    <!-- Feature Not Enabled Warning -->
    {#if !features.admin}
    <div
      class="card p-4 border-2 {$darkMode
        ? 'border-yellow-800 bg-yellow-900/20'
        : 'border-yellow-300 bg-yellow-50'}"
    >
      <div class="flex items-start gap-3">
        <svg
          class="w-6 h-6 flex-shrink-0 {$darkMode
            ? 'text-yellow-400'
            : 'text-yellow-600'}"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
          />
        </svg>
        <div>
          <h4
            class="font-semibold {$darkMode
              ? 'text-yellow-300'
              : 'text-yellow-800'}"
          >
            Admin Feature Not Enabled
          </h4>
          <p
            class="text-sm mt-1 {$darkMode
              ? 'text-yellow-400/80'
              : 'text-yellow-700'}"
          >
            The admin feature is not enabled on this server. All administrative
            operations are disabled. To enable, rebuild with
            <code
              class="px-1.5 py-0.5 rounded {$darkMode
                ? 'bg-yellow-900/40'
                : 'bg-yellow-200'} font-mono text-xs">--features admin</code
            > flag.
          </p>
        </div>
      </div>
    </div>
  {/if}

  <!-- Quick Actions -->
  <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
    <!-- Reload Config -->
    <button
      on:click={reloadConfig}
      disabled={isReloading || !features.admin}
      class="card p-5 text-left transition-colors group disabled:opacity-50 disabled:cursor-not-allowed {$darkMode
        ? 'hover:bg-gray-700/50'
        : 'hover:bg-gray-50'}"
    >
      <div class="flex items-center gap-4">
        <div
          class="w-12 h-12 rounded-lg flex items-center justify-center transition-colors {$darkMode
            ? 'bg-blue-900/30 group-hover:bg-blue-900/50'
            : 'bg-blue-100 group-hover:bg-blue-200'}"
        >
          {#if isReloading}
            <svg
              class="w-6 h-6 animate-spin {$darkMode
                ? 'text-blue-400'
                : 'text-blue-600'}"
              fill="none"
              viewBox="0 0 24 24"
            >
              <circle
                class="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                stroke-width="4"
              ></circle>
              <path
                class="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
              ></path>
            </svg>
          {:else}
            <svg
              class="w-6 h-6 {$darkMode ? 'text-blue-400' : 'text-blue-600'}"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
              />
            </svg>
          {/if}
        </div>
        <div>
          <div
            class="font-semibold {$darkMode ? 'text-white' : 'text-gray-900'}"
          >
            Reload Config
          </div>
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            Reload configuration file
          </div>
        </div>
      </div>
    </button>

    <!-- Clear Cache -->
    <button
      on:click={clearCache}
      disabled={isClearingCache || !features.admin}
      class="card p-5 text-left transition-colors group disabled:opacity-50 disabled:cursor-not-allowed {$darkMode
        ? 'hover:bg-gray-700/50'
        : 'hover:bg-gray-50'}"
    >
      <div class="flex items-center gap-4">
        <div
          class="w-12 h-12 rounded-lg flex items-center justify-center transition-colors {$darkMode
            ? 'bg-yellow-900/30 group-hover:bg-yellow-900/50'
            : 'bg-yellow-100 group-hover:bg-yellow-200'}"
        >
          {#if isClearingCache}
            <svg
              class="w-6 h-6 animate-spin {$darkMode
                ? 'text-yellow-400'
                : 'text-yellow-600'}"
              fill="none"
              viewBox="0 0 24 24"
            >
              <circle
                class="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                stroke-width="4"
              ></circle>
              <path
                class="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
              ></path>
            </svg>
          {:else}
            <svg
              class="w-6 h-6 {$darkMode
                ? 'text-yellow-400'
                : 'text-yellow-600'}"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
              />
            </svg>
          {/if}
        </div>
        <div>
          <div
            class="font-semibold {$darkMode ? 'text-white' : 'text-gray-900'}"
          >
            Clear Cache
          </div>
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            {formatNumber(serverInfo.cache_size)} entries
          </div>
        </div>
      </div>
    </button>

    <!-- Acknowledge All Alerts -->
    <button
      on:click={acknowledgeAllAlerts}
      disabled={isAcknowledgingAlerts || !features.admin}
      class="card p-5 text-left transition-colors group disabled:opacity-50 disabled:cursor-not-allowed {$darkMode
        ? 'hover:bg-gray-700/50'
        : 'hover:bg-gray-50'}"
    >
      <div class="flex items-center gap-4">
        <div
          class="w-12 h-12 rounded-lg flex items-center justify-center transition-colors {$darkMode
            ? 'bg-green-900/30 group-hover:bg-green-900/50'
            : 'bg-green-100 group-hover:bg-green-200'}"
        >
          {#if isAcknowledgingAlerts}
            <svg
              class="w-6 h-6 animate-spin {$darkMode
                ? 'text-green-400'
                : 'text-green-600'}"
              fill="none"
              viewBox="0 0 24 24"
            >
              <circle
                class="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                stroke-width="4"
              ></circle>
              <path
                class="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
              ></path>
            </svg>
          {:else}
            <svg
              class="w-6 h-6 {$darkMode ? 'text-green-400' : 'text-green-600'}"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M5 13l4 4L19 7"
              />
            </svg>
          {/if}
        </div>
        <div>
          <div
            class="font-semibold {$darkMode ? 'text-white' : 'text-gray-900'}"
          >
            Acknowledge All
          </div>
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            Acknowledge all alerts
          </div>
        </div>
      </div>
    </button>
    <!-- Export Logs -->
    <button
      on:click={() => exportLogs("alerts", "csv")}
      disabled={isExportingLogs || !features.admin}
      class="card p-5 text-left transition-colors group disabled:opacity-50 disabled:cursor-not-allowed {$darkMode
        ? 'hover:bg-gray-700/50'
        : 'hover:bg-gray-50'}"
    >
      <div class="flex items-center gap-4">
        <div
          class="w-12 h-12 rounded-lg flex items-center justify-center transition-colors {$darkMode
            ? 'bg-purple-900/30 group-hover:bg-purple-900/50'
            : 'bg-purple-100 group-hover:bg-purple-200'}"
        >
          <svg
            class="w-6 h-6 {$darkMode ? 'text-purple-400' : 'text-purple-600'}"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M12 10v6m0 0l-3-3m3 3l3-3m2 8H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
            />
          </svg>
        </div>
        <div>
          <div
            class="font-semibold {$darkMode ? 'text-white' : 'text-gray-900'}"
          >
            Export Alerts
          </div>
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            Download Security Alerts
          </div>
        </div>
      </div>
    </button>
  </div>

  <!-- Main Content Grid -->
  <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
    <!-- Cache Statistics -->
    <div class="card">
      <div class="card-header flex items-center justify-between">
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
              d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4"
            />
          </svg>
          Cache Statistics
        </h3>
        <button
          on:click={clearCache}
          disabled={isClearingCache || !features.admin}
          class="btn-danger text-xs py-1 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          Clear Cache
        </button>
      </div>
      <div class="card-body">
        <div class="grid grid-cols-2 gap-4">
          <div
            class="p-4 {$darkMode
              ? 'bg-gray-700/30'
              : 'bg-gray-100'} rounded-lg"
          >
            <div
              class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}"
            >
              Cache Size
            </div>
            <div
              class="text-2xl font-bold {$darkMode
                ? 'text-white'
                : 'text-gray-900'} mt-1"
            >
              {cacheStats.size.toLocaleString()}
            </div>
            <div
              class="text-xs {$darkMode
                ? 'text-gray-500'
                : 'text-gray-500'} mt-1"
            >
              entries
            </div>
          </div>

          <div
            class="p-4 {$darkMode
              ? 'bg-gray-700/30'
              : 'bg-gray-100'} rounded-lg"
          >
            <div
              class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}"
            >
              Hit Rate
            </div>
            <div class="text-2xl font-bold text-green-500 mt-1">
              {cacheStats.hit_rate.toFixed(1)}%
            </div>
            <div
              class="text-xs {$darkMode
                ? 'text-gray-500'
                : 'text-gray-500'} mt-1"
            >
              efficiency
            </div>
          </div>

          <div
            class="p-4 {$darkMode
              ? 'bg-gray-700/30'
              : 'bg-gray-100'} rounded-lg"
          >
            <div
              class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}"
            >
              Cache Hits
            </div>
            <div class="text-2xl font-bold text-blue-500 mt-1">
              {formatNumber(cacheStats.hits)}
            </div>
            <div
              class="text-xs {$darkMode
                ? 'text-gray-500'
                : 'text-gray-500'} mt-1"
            >
              total hits
            </div>
          </div>

          <div
            class="p-4 {$darkMode
              ? 'bg-gray-700/30'
              : 'bg-gray-100'} rounded-lg"
          >
            <div
              class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}"
            >
              Cache Misses
            </div>
            <div class="text-2xl font-bold text-yellow-500 mt-1">
              {formatNumber(cacheStats.misses)}
            </div>
            <div
              class="text-xs {$darkMode
                ? 'text-gray-500'
                : 'text-gray-500'} mt-1"
            >
              total misses
            </div>
          </div>

          <div
            class="p-4 {$darkMode
              ? 'bg-gray-700/30'
              : 'bg-gray-100'} rounded-lg"
          >
            <div
              class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}"
            >
              Evictions
            </div>
            <div class="text-2xl font-bold text-orange-500 mt-1">
              {formatNumber(cacheStats.evictions)}
            </div>
            <div
              class="text-xs {$darkMode
                ? 'text-gray-500'
                : 'text-gray-500'} mt-1"
            >
              LRU evictions
            </div>
          </div>

          <div
            class="p-4 {$darkMode
              ? 'bg-gray-700/30'
              : 'bg-gray-100'} rounded-lg"
          >
            <div
              class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}"
            >
              Expirations
            </div>
            <div class="text-2xl font-bold text-purple-500 mt-1">
              {formatNumber(cacheStats.expirations)}
            </div>
            <div
              class="text-xs {$darkMode
                ? 'text-gray-500'
                : 'text-gray-500'} mt-1"
            >
              TTL expired
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Server Information -->
    <div class="card">
      <div class="card-header">
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
              d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"
            />
          </svg>
          Server Information
        </h3>
      </div>
      <div class="card-body space-y-3">
        {#each [{ label: "Version", value: serverInfo.version }, { label: "Status", value: serverInfo.status }, { label: "Uptime", value: formatUptime(serverInfo.uptime_secs) }, { label: "Total Queries", value: formatNumber(serverInfo.total_queries) }, { label: "Memory (RSS)", value: formatBytes(serverInfo.rss_bytes) }] as item}
          <div
            class="flex items-center justify-between py-2 border-b {$darkMode
              ? 'border-gray-700/50'
              : 'border-gray-200'} last:border-0"
          >
            <span class={$darkMode ? "text-gray-400" : "text-gray-700"}
              >{item.label}</span
            >
            <span
              class="{$darkMode
                ? 'text-white'
                : 'text-gray-900'} font-mono text-sm">{item.value}</span
            >
          </div>
        {/each}
      </div>
    </div>
  </div>

  <!-- Latency Statistics -->
  <div class="card">
    <div class="card-header">
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
            d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
        Latency Statistics
      </h3>
    </div>
    <div class="card-body">
      <div class="grid grid-cols-2 md:grid-cols-5 gap-4">
        <div
          class="p-4 {$darkMode
            ? 'bg-gray-700/30'
            : 'bg-gray-100'} rounded-lg text-center"
        >
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            P50 Latency
          </div>
          <div class="text-2xl font-bold text-blue-500 mt-1">
            {latencyStats.p50_ms.toFixed(1)}
          </div>
          <div
            class="text-xs {$darkMode ? 'text-gray-500' : 'text-gray-500'} mt-1"
          >
            ms
          </div>
        </div>

        <div
          class="p-4 {$darkMode
            ? 'bg-gray-700/30'
            : 'bg-gray-100'} rounded-lg text-center"
        >
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            P95 Latency
          </div>
          <div class="text-2xl font-bold text-green-500 mt-1">
            {latencyStats.p95_ms.toFixed(1)}
          </div>
          <div
            class="text-xs {$darkMode ? 'text-gray-500' : 'text-gray-500'} mt-1"
          >
            ms
          </div>
        </div>

        <div
          class="p-4 {$darkMode
            ? 'bg-gray-700/30'
            : 'bg-gray-100'} rounded-lg text-center"
        >
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            P99 Latency
          </div>
          <div class="text-2xl font-bold text-yellow-500 mt-1">
            {latencyStats.p99_ms.toFixed(1)}
          </div>
          <div
            class="text-xs {$darkMode ? 'text-gray-500' : 'text-gray-500'} mt-1"
          >
            ms
          </div>
        </div>

        <div
          class="p-4 {$darkMode
            ? 'bg-gray-700/30'
            : 'bg-gray-100'} rounded-lg text-center"
        >
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            Max Latency
          </div>
          <div class="text-2xl font-bold text-red-500 mt-1">
            {latencyStats.max_ms.toFixed(1)}
          </div>
          <div
            class="text-xs {$darkMode ? 'text-gray-500' : 'text-gray-500'} mt-1"
          >
            ms
          </div>
        </div>

        <div
          class="p-4 {$darkMode
            ? 'bg-gray-700/30'
            : 'bg-gray-100'} rounded-lg text-center"
        >
          <div class="text-sm {$darkMode ? 'text-gray-400' : 'text-gray-700'}">
            Avg Latency
          </div>
          <div class="text-2xl font-bold text-purple-500 mt-1">
            {latencyStats.avg_ms.toFixed(2)}
          </div>
          <div
            class="text-xs {$darkMode ? 'text-gray-500' : 'text-gray-500'} mt-1"
          >
            ms
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- Configuration Reload -->
  <div class="card">
    <div class="card-header">
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
            d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
          />
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
          />
        </svg>
        Configuration
      </h3>
    </div>
    <div class="card-body">
      <div class="flex items-end gap-4">
        <div class="flex-1">
          <label
            for="configPath"
            class="block text-sm font-medium {$darkMode
              ? 'text-gray-400'
              : 'text-gray-700'} mb-2"
          >
            Config File Path
          </label>
          <input
            id="configPath"
            type="text"
            bind:value={configPath}
            class="input font-mono"
            placeholder="/etc/lazydns/config.yaml"
          />
        </div>
        <button
          on:click={reloadConfig}
          disabled={isReloading || !features.admin}
          class="btn-primary flex items-center gap-2 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {#if isReloading}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle
                class="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                stroke-width="4"
              ></circle>
              <path
                class="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
              ></path>
            </svg>
            Reloading...
          {:else}
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
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
              />
            </svg>
            Reload Configuration
          {/if}
        </button>
      </div>
      <p class="text-sm {$darkMode ? 'text-gray-500' : 'text-gray-700'} mt-3">
        Reload the configuration file to apply changes without restarting the
        server.
      </p>
    </div>
  </div>
  {/if}

  {#if activeTab === "config"}
    <!-- Configuration Not Available Warning -->
    {#if !features.admin}
      <div
        class="card p-4 border-2 {$darkMode
          ? 'border-yellow-800 bg-yellow-900/20'
          : 'border-yellow-300 bg-yellow-50'}"
      >
        <div class="flex items-start gap-3">
          <svg
            class="w-6 h-6 flex-shrink-0 {$darkMode
              ? 'text-yellow-400'
              : 'text-yellow-600'}"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
            />
          </svg>
          <div>
            <h4
              class="font-semibold {$darkMode
                ? 'text-yellow-300'
                : 'text-yellow-800'}"
            >
              Configuration Not Available
            </h4>
            <p
              class="text-sm mt-1 {$darkMode
                ? 'text-yellow-400/80'
                : 'text-yellow-700'}"
            >
              The configuration view requires an admin build. Rebuild with the
              <code
                class="px-1.5 py-0.5 rounded {$darkMode
                  ? 'bg-yellow-900/40'
                  : 'bg-yellow-200'} font-mono text-xs">--features admin</code
              >
              flag to enable it.
            </p>
          </div>
        </div>
      </div>
    {:else if configError}
      <div
        class="card p-4 border-2 {$darkMode
          ? 'border-red-800 bg-red-900/20'
          : 'border-red-300 bg-red-50'}"
      >
        <p class="text-sm {$darkMode ? 'text-red-300' : 'text-red-700'}">
          {configError}
        </p>
      </div>
    {:else if configDump}
      <!-- Top-level Settings -->
      <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <!-- Logging & Admin -->
        <div class="card">
          <div class="card-header">
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
                  d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
                />
              </svg>
              Logging & Admin
            </h3>
          </div>
          <div class="card-body space-y-0">
            {#each [{ label: "Version", value: configDump.version }, { label: "Log Level", value: configDump.log.level }, { label: "Log Format", value: configDump.log.format }, { label: "Console Output", value: configDump.log.console ? "enabled" : "disabled" }, { label: "File Logging", value: configDump.log.file_enabled ? "enabled" : "disabled" }, { label: "Admin API", value: configDump.admin ? `${configDump.admin.enabled ? "enabled" : "disabled"} (${configDump.admin.addr})` : "n/a" }] as item}
              <div
                class="flex items-center justify-between py-2 border-b {$darkMode
                  ? 'border-gray-700/50'
                  : 'border-gray-200'} last:border-0"
              >
                <span class={$darkMode ? "text-gray-400" : "text-gray-700"}
                  >{item.label}</span
                >
                <span
                  class="{$darkMode
                    ? 'text-white'
                    : 'text-gray-900'} font-mono text-sm">{item.value}</span
                >
              </div>
            {/each}
          </div>
        </div>

        <!-- Monitoring & Web -->
        <div class="card">
          <div class="card-header">
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
                  d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"
                />
              </svg>
              Servers
            </h3>
          </div>
          <div class="card-body space-y-0">
            {#each [{ label: "Monitoring", value: configDump.monitoring ? `${configDump.monitoring.enabled ? "enabled" : "disabled"} (${configDump.monitoring.addr})` : "n/a" }, { label: "WebUI", value: configDump.web ? `${configDump.web.enabled ? "enabled" : "disabled"} (${configDump.web.listen})` : "n/a" }, { label: "Total Plugins", value: String(configDump.plugin_count) }, { label: "Listeners", value: configServers.length > 0 ? configServers.map((s) => s.tag).join(", ") : "none" }] as item}
              <div
                class="flex items-center justify-between py-2 border-b {$darkMode
                  ? 'border-gray-700/50'
                  : 'border-gray-200'} last:border-0"
              >
                <span class={$darkMode ? "text-gray-400" : "text-gray-700"}
                  >{item.label}</span
                >
                <span
                  class="{$darkMode
                    ? 'text-white'
                    : 'text-gray-900'} font-mono text-sm text-right break-all max-w-[60%]">{item.value}</span
                >
              </div>
            {/each}
          </div>
        </div>
      </div>

      <!-- Sequences Flow -->
      {#if configSequences.length > 0}
        <div class="card">
          <div class="card-header">
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
                  d="M13 5l7 7-7 7M5 5l7 7-7 7"
                />
              </svg>
              Sequences
              <span class="badge badge-info">{configSequences.length}</span>
            </h3>
          </div>
          <div class="card-body space-y-3">
            {#each configSequences as seq}
              <div
                class="rounded-lg {$darkMode
                  ? 'bg-gray-700/30'
                  : 'bg-gray-100'} p-3"
              >
                <button
                  on:click={() => toggleSequence(seq.tag)}
                  class="w-full flex items-center justify-between text-left"
                >
                  <span
                    class="font-mono font-semibold {$darkMode
                      ? 'text-white'
                      : 'text-gray-900'}"
                  >
                    {seq.tag}
                    {#if seq.sequence_steps}
                      <span
                        class="ml-2 text-xs font-normal {$darkMode
                          ? 'text-gray-400'
                          : 'text-gray-700'}"
                      >
                        {seq.sequence_steps.length} steps
                      </span>
                    {/if}
                  </span>
                  <svg
                    class="w-4 h-4 transition-transform {$darkMode
                      ? 'text-gray-400'
                      : 'text-gray-500'}"
                    class:rotate-180={expandedSequences[seq.tag]}
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M19 9l-7 7-7-7"
                    />
                  </svg>
                </button>

                {#if expandedSequences[seq.tag] && seq.sequence_steps}
                  <ol
                    class="mt-3 ml-3 border-l-2 {$darkMode
                      ? 'border-primary-500/50'
                      : 'border-primary-400'} space-y-2"
                  >
                    {#each seq.sequence_steps as step, i}
                      <li class="ml-4 flex items-start gap-2">
                        <span
                          class="flex-shrink-0 w-5 h-5 rounded-full {$darkMode
                            ? 'bg-primary-500/20 text-primary-400'
                            : 'bg-primary-100 text-primary-700'} text-xs flex items-center justify-center font-semibold mt-0.5"
                        >
                          {i + 1}
                        </span>
                        <span
                          class="font-mono text-sm {$darkMode
                            ? 'text-gray-300'
                            : 'text-gray-800'}"
                        >
                          {#if step.matches}
                            <span
                              class="{$darkMode
                                ? 'text-yellow-400'
                                : 'text-yellow-600'}">matches</span
                            >
                            {step.matches}
                            <span
                              class={$darkMode
                                ? 'text-gray-500'
                                : 'text-gray-400'}>{"->"}</span
                            >
                          {/if}
                          {#if step.exec}
                            <span
                              class="{$darkMode
                                ? 'text-blue-400'
                                : 'text-blue-600'}">exec</span
                            >
                            {step.exec}
                          {/if}
                        </span>
                      </li>
                    {/each}
                  </ol>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Other Plugins -->
      {#if configOtherPlugins.length > 0}
        <div class="card">
          <div class="card-header">
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
                  d="M19.428 15.428a2 2 0 00-1.022-.547l-2.387-.477a6 6 0 00-3.86.517l-.318.158a6 6 0 01-3.86.517L6.05 15.21a2 2 0 00-1.806.547M8 4h8l-1 1v5.172a2 2 0 00.586 1.414l5 5c1.26 1.26.367 3.414-1.415 3.414H4.828c-1.782 0-2.674-2.154-1.414-3.414l5-5A2 2 0 009 10.172V5L8 4z"
                />
              </svg>
              Plugins
              <span class="badge badge-gray">{configOtherPlugins.length}</span>
            </h3>
          </div>
          <div class="card-body">
            <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
              {#each configOtherPlugins as p}
                <div
                  class="p-3 rounded-lg {$darkMode
                    ? 'bg-gray-700/30'
                    : 'bg-gray-100'}"
                >
                  <div class="flex items-center gap-2 flex-wrap">
                    <span
                      class="font-mono font-semibold {$darkMode
                        ? 'text-white'
                        : 'text-gray-900'}"
                    >
                      {p.tag}
                    </span>
                    <span class="badge badge-gray">{p.plugin_type}</span>
                  </div>
                  {#if argsPreview(p)}
                    <p
                      class="mt-1 text-xs font-mono {$darkMode
                        ? 'text-gray-400'
                        : 'text-gray-700'}"
                    >
                      {argsPreview(p)}
                    </p>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        </div>
      {/if}
    {:else}
      <div class="card">
        <div class="card-body text-center py-8">
          <p class={$darkMode ? "text-gray-400" : "text-gray-500"}>
            Loading configuration...
          </p>
        </div>
      </div>
    {/if}
  {/if}
</div>
