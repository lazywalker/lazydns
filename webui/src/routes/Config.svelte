<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, type ConfigDumpResponse, type ConfigPluginSummary } from "../lib/api";
  import { darkMode } from "../lib/stores";
  import { features } from "../lib/features.svelte";

  let config: ConfigDumpResponse | null = null;
  let loading = true;
  let error: string | null = null;
  // Expandable sequences: keyed by plugin tag (true = expanded).
  let expandedSequences: Record<string, boolean> = {};

  async function fetchData() {
    try {
      config = await api.getConfigDump();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load configuration";
      console.error("Config fetch error:", e);
    } finally {
      loading = false;
    }
  }

  function toggleSequence(tag: string) {
    expandedSequences[tag] = !expandedSequences[tag];
  }

  // Render a short one-line summary of a plugin's args based on common keys.
  function argsPreview(p: ConfigPluginSummary): string {
    // args_summary may be null/undefined when the plugin has no args (e.g.
    // a bare `blackhole` plugin). Guard against null to avoid a crash.
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
  $: servers = config ? config.plugins.filter((p) => isServer(p)) : [];
  $: sequences = config ? config.plugins.filter((p) => p.is_sequence) : [];
  $: otherPlugins = config
    ? config.plugins.filter((p) => !isServer(p) && !p.is_sequence)
    : [];

  function isServer(p: ConfigPluginSummary): boolean {
    return p.plugin_type.endsWith("_server") || p.plugin_type === "server";
  }

  let refreshInterval: ReturnType<typeof setInterval>;
  onMount(() => {
    fetchData();
    // Config is mostly static; refresh every 30s.
    refreshInterval = setInterval(fetchData, 30000);
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
        Configuration
      </h1>
      <p class="{$darkMode ? 'text-gray-400' : 'text-gray-700'} mt-1">
        Read-only view of the currently loaded configuration
      </p>
    </div>
    <button
      on:click={fetchData}
      disabled={loading}
      class="btn-secondary flex items-center gap-2 disabled:opacity-50"
    >
      {#if loading}
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
        Loading...
      {:else}
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
          />
        </svg>
        Refresh
      {/if}
    </button>
  </div>

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
  {:else if error}
    <div
      class="card p-4 border-2 {$darkMode
        ? 'border-red-800 bg-red-900/20'
        : 'border-red-300 bg-red-50'}"
    >
      <p class="text-sm {$darkMode ? 'text-red-300' : 'text-red-700'}">
        {error}
      </p>
    </div>
  {:else if config}
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
          {#each [
            { label: "Version", value: config.version },
            { label: "Log Level", value: config.log.level },
            { label: "Log Format", value: config.log.format },
            {
              label: "Console Output",
              value: config.log.console ? "enabled" : "disabled",
            },
            {
              label: "File Logging",
              value: config.log.file_enabled ? "enabled" : "disabled",
            },
            {
              label: "Admin API",
              value: config.admin
                ? `${config.admin.enabled ? "enabled" : "disabled"} (${config.admin.addr})`
                : "n/a",
            },
          ] as item}
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
          {#each [
            {
              label: "Monitoring",
              value: config.monitoring
                ? `${config.monitoring.enabled ? "enabled" : "disabled"} (${config.monitoring.addr})`
                : "n/a",
            },
            {
              label: "WebUI",
              value: config.web
                ? `${config.web.enabled ? "enabled" : "disabled"} (${config.web.listen})`
                : "n/a",
            },
            {
              label: "Total Plugins",
              value: String(config.plugin_count),
            },
            {
              label: "Listeners",
              value: servers.length > 0
                ? servers.map((s) => s.tag).join(", ")
                : "none",
            },
          ] as item}
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
    {#if sequences.length > 0}
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
            <span class="badge badge-info">{sequences.length}</span>
          </h3>
        </div>
        <div class="card-body space-y-3">
          {#each sequences as seq}
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
                              : 'text-yellow-600'}">matches</span>
                          {step.matches}
                          <span class={$darkMode ? 'text-gray-500' : 'text-gray-400'}>{"->"}</span>
                        {/if}
                        {#if step.exec}
                          <span
                            class="{$darkMode
                              ? 'text-blue-400'
                              : 'text-blue-600'}">exec</span>
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
    {#if otherPlugins.length > 0}
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
            <span class="badge badge-gray">{otherPlugins.length}</span>
          </h3>
        </div>
        <div class="card-body">
          <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
            {#each otherPlugins as p}
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
</div>
