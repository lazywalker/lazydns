<script lang="ts">
  import { darkMode } from "../lib/stores";
  import { features } from "../lib/features.svelte";
  import { createEventDispatcher } from "svelte";
  import ConfigValue from "./ConfigValue.svelte";
  import type { ConfigDumpResponse, ConfigPluginSummary } from "../lib/api";

  // Configuration dump (fetched by the parent Admin page).
  export let configDump: ConfigDumpResponse | null = null;
  // Error from fetching the dump (e.g. backend unavailable).
  export let configError: string | null = null;
  // Error from the last config reload (persistent banner); cleared by the user.
  export let reloadError: string | null = null;

  const dispatch = createEventDispatcher<{ dismissreloaderror: void }>();

  // Expandable sequences: keyed by plugin tag (true = expanded).
  let expandedSequences: Record<string, boolean> = {};
  // Expandable plugins (args detail): keyed by plugin tag (true = expanded).
  let expandedPlugins: Record<string, boolean> = {};

  function toggleSequence(tag: string) {
    expandedSequences[tag] = !expandedSequences[tag];
  }

  function togglePlugin(tag: string) {
    expandedPlugins[tag] = !expandedPlugins[tag];
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
</script>

<div class="space-y-6">
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
  {:else if reloadError}
    <!-- Persistent reload error banner (Task 4): the toast is fleeting, but a
         config validation failure needs to stay visible until dismissed. -->
    <div
      class="card p-4 border-2 {$darkMode
        ? 'border-red-800 bg-red-900/20'
        : 'border-red-300 bg-red-50'}"
    >
      <div class="flex items-start gap-3">
        <svg
          class="w-6 h-6 flex-shrink-0 {$darkMode
            ? 'text-red-400'
            : 'text-red-600'}"
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
        <div class="flex-1 min-w-0">
          <h4
            class="font-semibold {$darkMode
              ? 'text-red-300'
              : 'text-red-800'}"
          >
            Configuration Reload Failed
          </h4>
          <p
            class="text-sm mt-1 break-words font-mono {$darkMode
              ? 'text-red-400/90'
              : 'text-red-700'}"
          >
            {reloadError}
          </p>
        </div>
        <button
          on:click={() => dispatch("dismissreloaderror")}
          class="flex-shrink-0 {$darkMode
            ? 'text-red-400 hover:text-red-300'
            : 'text-red-500 hover:text-red-700'}"
          title="Dismiss"
        >
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
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
                <button
                  on:click={() => togglePlugin(p.tag)}
                  class="w-full flex items-center justify-between text-left"
                >
                  <div class="flex items-center gap-2 flex-wrap min-w-0">
                    <span
                      class="font-mono font-semibold {$darkMode
                        ? 'text-white'
                        : 'text-gray-900'}"
                    >
                      {p.tag}
                    </span>
                    <span class="badge badge-gray">{p.plugin_type}</span>
                    {#if !expandedPlugins[p.tag] && argsPreview(p)}
                      <span
                        class="text-xs font-mono {$darkMode
                          ? 'text-gray-400'
                          : 'text-gray-700'} truncate"
                      >
                        {argsPreview(p)}
                      </span>
                    {/if}
                  </div>
                  <svg
                    class="w-4 h-4 flex-shrink-0 transition-transform {$darkMode
                      ? 'text-gray-400'
                      : 'text-gray-500'}"
                    class:rotate-180={expandedPlugins[p.tag]}
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

                {#if expandedPlugins[p.tag]}
                  <div
                    class="mt-3 pt-3 border-t {$darkMode
                      ? 'border-gray-600'
                      : 'border-gray-200'}"
                  >
                    <ConfigValue value={p.args_summary} />
                  </div>
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
