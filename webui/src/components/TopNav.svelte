<script lang="ts">
  import { link } from "svelte-spa-router";
  import active from "svelte-spa-router/active";
  import {
    topNavCollapsed,
    darkMode,
    isLiveMode,
    unacknowledgedAlerts,
    notifications,
  } from "../lib/stores";

  const navItems = [
    { path: "/", icon: "dashboard", label: "Dashboard" },
    { path: "/metrics", icon: "chart", label: "Metrics" },
    { path: "/audit", icon: "logs", label: "Logs" },
    { path: "/alerts", icon: "alert", label: "Alerts" },
    { path: "/admin", icon: "settings", label: "Admin" },
  ];

  function toggleTheme() {
    darkMode.toggle();
  }

  function toggleLiveMode() {
    isLiveMode.update((v) => !v);
    notifications.add({
      type: "info",
      message: $isLiveMode ? "Live updates disabled" : "Live updates enabled",
    });
  }
</script>

<nav
  class="fixed top-0 left-0 right-0 h-16 border-b flex items-center px-4 gap-4 z-40 transition-colors duration-200 {$darkMode
    ? 'bg-gray-900 border-gray-800'
    : 'bg-white border-gray-200'}"
>
  <!-- Logo -->
  <div class="flex items-center gap-3 flex-shrink-0 logo-hide-xs">
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
    <span class="text-lg font-bold text-gradient hidden sm:inline">LazyDNS</span
    >
  </div>

  <!-- Navigation Items -->
  <div
    class="flex items-center gap-1 transition-all duration-300 overflow-hidden"
    class:w-auto={!$topNavCollapsed}
    class:w-0={$topNavCollapsed}
  >
    {#each navItems as item}
      <a
        href={item.path}
        use:link
        use:active={{ path: item.path, className: "nav-link-active" }}
        class="nav-link-top whitespace-nowrap"
        title={item.label}
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
          <div class="relative">
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
                d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
              />
            </svg>
            {#if $unacknowledgedAlerts > 0}
              <span
                class="absolute -top-2 -right-2 flex items-center justify-center min-w-[1.125rem] h-4.5 px-1 text-xs font-bold text-white bg-red-500 rounded-full"
              >
                {$unacknowledgedAlerts}
              </span>
            {/if}
          </div>
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
        {#if !$topNavCollapsed}
          <span class="hidden md:inline text-sm">{item.label}</span>
        {/if}
      </a>
    {/each}
  </div>

  <!-- Spacer -->
  <div class="flex-1 spacer-hide-xs"></div>

  <!-- Theme Toggle Button -->
  <button
    on:click={toggleTheme}
    class="flex items-center justify-center p-2 rounded-lg transition-colors {$darkMode
      ? 'text-gray-400 hover:text-white hover:bg-gray-800'
      : 'text-gray-600 hover:text-gray-900 hover:bg-gray-100'}"
    title={$darkMode ? "Switch to light mode" : "Switch to dark mode"}
  >
    {#if $darkMode}
      <!-- Sun icon for switching to light mode -->
      <svg
        class="w-5 h-5"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          stroke-width="2"
          d="M12 3v1m0 16v1m9-9h-1M4 12H3m15.364 6.364l-.707-.707M6.343 6.343l-.707-.707m12.728 0l-.707.707M6.343 17.657l-.707.707M16 12a4 4 0 11-8 0 4 4 0 018 0z"
        />
      </svg>
    {:else}
      <!-- Moon icon for switching to dark mode -->
      <svg
        class="w-5 h-5"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          stroke-width="2"
          d="M20.354 15.354A9 9 0 018.646 3.646 9.003 9.003 0 0012 21a9.003 9.003 0 008.354-5.646z"
        />
      </svg>
    {/if}
  </button>

  <!-- Live Button -->
  <button
    on:click={toggleLiveMode}
    class="flex items-center gap-2 px-3 py-1.5 rounded-lg transition-colors border {$isLiveMode
      ? $darkMode
        ? 'bg-green-900 bg-opacity-30 text-green-400 border-green-700'
        : 'bg-green-50 text-green-700 border-green-300'
      : $darkMode
        ? 'bg-gray-800 text-gray-400 border-gray-700'
        : 'bg-gray-100 text-gray-600 border-gray-300'}"
    title={$isLiveMode ? "Disable live updates" : "Enable live updates"}
  >
    <span class="relative flex h-2.5 w-2.5">
      {#if $isLiveMode}
        <span
          class="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"
        ></span>
      {/if}
      <span
        class="relative inline-flex rounded-full h-2.5 w-2.5"
        class:bg-green-400={$isLiveMode}
        class:bg-gray-500={!$isLiveMode}
      ></span>
    </span>
    <span class="text-sm font-medium">
      {$isLiveMode ? "Live" : "Paused"}
    </span>
  </button>
</nav>

<style>
  :global(.nav-link-top) {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
    transition: all 0.3s ease;
    font-size: 0.875rem;
  }

  :global(.dark .nav-link-top) {
    color: #9ca3af;
  }

  :global(.nav-link-top) {
    color: #4b5563;
  }

  :global(.dark .nav-link-top:hover) {
    color: white;
    background-color: #1f2937;
  }

  :global(.nav-link-top:hover) {
    color: #111827;
    background-color: #f3f4f6;
  }

  :global(.dark .nav-link-top.nav-link-active) {
    color: white;
    background: linear-gradient(
      to right,
      rgba(59, 130, 246, 0.2),
      rgba(37, 99, 235, 0.2)
    );
    border: 1px solid rgba(59, 130, 246, 0.3);
  }

  :global(.nav-link-top.nav-link-active) {
    color: #1d4ed8;
    background: linear-gradient(
      to right,
      rgba(59, 130, 246, 0.1),
      rgba(37, 99, 235, 0.1)
    );
    border: 1px solid rgba(59, 130, 246, 0.3);
  }

  /* Hide full logo (icon + text) on very narrow screens */
  @media (max-width: 440px) {
    :global(.logo-hide-xs) {
      display: none !important;
    }
  }
  @media (max-width: 420px) {
    :global(.spacer-hide-xs) {
      display: none !important;
    }
  }
</style>
