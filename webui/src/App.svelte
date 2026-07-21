<script lang="ts">
  import { onMount } from "svelte";
  import Router from "svelte-spa-router";
  import TopNav from "./components/TopNav.svelte";
  import Notifications from "./components/Notifications.svelte";
  import Dashboard from "./routes/Dashboard.svelte";
  import AuditLogs from "./routes/AuditLogs.svelte";
  import Metrics from "./routes/Metrics.svelte";
  import Alerts from "./routes/Alerts.svelte";
  import Admin from "./routes/Admin.svelte";
  import { darkMode } from "./lib/stores";
  import { loadServerFeatures } from "./lib/features.svelte";

  const routes = {
    "/": Dashboard,
    "/metrics": Metrics,
    "/audit": AuditLogs,
    "/alerts": Alerts,
    "/admin": Admin,
  };

  // Load server features on mount
  onMount(() => {
    loadServerFeatures();
  });

  // Apply dark class to html element for proper CSS cascade
  $: if (typeof document !== "undefined") {
    if ($darkMode) {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
  }
</script>

<div
  class="min-h-screen flex flex-col transition-colors duration-200 {$darkMode
    ? 'bg-gray-950'
    : 'bg-gray-100'}"
>
  <TopNav />

  <div class="flex-1 flex flex-col pt-16">
    <main class="flex-1 p-6 overflow-auto">
      <Router {routes} />
    </main>
  </div>
</div>

<Notifications />
