<script lang="ts">
  import { link } from "svelte-spa-router";
  import active from "svelte-spa-router/active";
  import { sidebarCollapsed } from "../lib/stores";

  const navItems = [
    { path: "/", icon: "dashboard", label: "Dashboard" },
    { path: "/metrics", icon: "chart", label: "Metrics" },
    { path: "/audit", icon: "logs", label: "Audit Logs" },
    { path: "/alerts", icon: "alert", label: "Alerts" },
    { path: "/admin", icon: "settings", label: "Admin" },
  ];

  function toggleSidebar() {
    sidebarCollapsed.update((v) => !v);
  }
</script>

<aside
  class="fixed left-0 top-0 h-full bg-gray-900 border-r border-gray-800 flex flex-col z-40 transition-all duration-300"
  class:w-64={!$sidebarCollapsed}
  class:w-20={$sidebarCollapsed}
>
  <!-- Logo -->
  <div
    class="h-16 flex items-center justify-between px-4 border-b border-gray-800"
  >
    {#if !$sidebarCollapsed}
      <div class="flex items-center gap-3">
        <div
          class="w-8 h-8 rounded-lg bg-gradient-to-br from-primary-500 to-blue-600 flex items-center justify-center"
        >
          <svg
            class="w-5 h-5 text-white"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9"
            />
          </svg>
        </div>
        <span class="text-lg font-bold text-gradient">LazyDNS</span>
      </div>
    {:else}
      <div
        class="w-8 h-8 mx-auto rounded-lg bg-gradient-to-br from-primary-500 to-blue-600 flex items-center justify-center"
      >
        <svg
          class="w-5 h-5 text-white"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9"
          />
        </svg>
      </div>
    {/if}
  </div>

  <!-- Navigation -->
  <nav class="flex-1 py-4 px-3 space-y-1 overflow-y-auto">
    {#each navItems as item}
      <a
        href={item.path}
        use:link
        use:active={{ path: item.path, className: "nav-link-active" }}
        class="nav-link"
        title={$sidebarCollapsed ? item.label : undefined}
      >
        {#if item.icon === "dashboard"}
          <svg
            class="w-5 h-5 flex-shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M4 5a1 1 0 011-1h14a1 1 0 011 1v2a1 1 0 01-1 1H5a1 1 0 01-1-1V5zM4 13a1 1 0 011-1h6a1 1 0 011 1v6a1 1 0 01-1 1H5a1 1 0 01-1-1v-6zM16 13a1 1 0 011-1h2a1 1 0 011 1v6a1 1 0 01-1 1h-2a1 1 0 01-1-1v-6z"
            />
          </svg>
        {:else if item.icon === "logs"}
          <svg
            class="w-5 h-5 flex-shrink-0"
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
        {:else if item.icon === "chart"}
          <svg
            class="w-5 h-5 flex-shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
            />
          </svg>
        {:else if item.icon === "alert"}
          <svg
            class="w-5 h-5 flex-shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M12 8v4m0 4v.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
            />
          </svg>
        {:else if item.icon === "settings"}
          <svg
            class="w-5 h-5 flex-shrink-0"
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
        {/if}
        {#if !$sidebarCollapsed}
          <span>{item.label}</span>
        {/if}
      </a>
    {/each}
  </nav>

  <!-- Collapse Button -->
  <div class="p-3 border-t border-gray-800">
    <button
      on:click={toggleSidebar}
      class="w-full flex items-center justify-center p-2 rounded-lg text-gray-400 hover:text-white hover:bg-gray-800 transition-colors"
      title={$sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
    >
      <svg
        class="w-5 h-5 transition-transform duration-300"
        class:rotate-180={$sidebarCollapsed}
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          stroke-width="2"
          d="M11 19l-7-7 7-7m8 14l-7-7 7-7"
        />
      </svg>
    </button>
  </div>
</aside>
