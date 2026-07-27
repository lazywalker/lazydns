# Shutdown and Graceful Termination Design

This document defines the shutdown orchestration for lazydns: how servers
(DoH/DoQ/DoT/TCP/Admin/Monitoring) and plugins cooperate to achieve a
predictable, observable, and safe graceful shutdown.

Goal
----
- Ensure the system stops accepting new work, allows in-flight work to
  complete within configurable bounds, and then releases background
  resources (watchers, job tasks, connection endpoints) in a deterministic
  order with timeouts and observability.

Principles
----------
- Drain first: stop accepting new requests/connections before tearing down
  shared resources.
- Reverse init order: shut down components in the inverse order of their
  initialization (consumers before providers) to avoid use-after-free.
- Fail-fast after bounded wait: honor configurable timeouts and then
  proceed with best-effort forced cleanup while logging failures.
- Make shutdown triggers explicit: accept OS signals, admin API calls, or
  programmatic requests, and route them through a single coordinator.

Shutdown Phases (high level)
----------------------------
1. Trigger: a signal (Ctrl-C, SIGTERM), Admin request, or other controller
   publishes a shutdown event to the ShutdownCoordinator.
2. Stop producing new work (drain): each server stops accepting new
   connections/requests (such as axum `graceful_shutdown`, quinn Endpoint
   close, TcpListener shutdown) and returns early for new clients.
3. Drain wait: wait for in-flight requests to finish (use a global active
   request counter or per-server counters). Wait is bounded by
   `drain_timeout` configuration.
4. Pre-shutdown of active generators: stop job producers (Cron, watchers,
   plugin-internal repeaters) so they don't create more work during shutdown.
5. Plugin Shutdown: call `shutdown()` (via the `as_shutdown()` bridge) on
   plugins in reverse registration/initialization order. Use per-plugin
   timeouts (such as `plugin_shutdown_timeout`). Prefer serial inverse order
   by default; allow optional parallel groups when safe.
6. Final cleanup: close global resources (metrics exporters, temp files,
   thread pools), await remaining JoinHandles with overall `global_shutdown_timeout`.

Servers: service-specific notes
------------------------------
- DoH (`src/server/doh.rs`)
  - Uses axum/axum-server; integrate axum's `graceful_shutdown` by passing
    a shutdown future (oneshot/watch). Ensure TLS (rustls) shutdown is
    coordinated and requests in flight are given time to complete.

- DoQ (`src/server/doq.rs`)
  - QUIC endpoints must be explicitly closed (call `Endpoint::close` or
    `Endpoint::server` shutdown) and per-connection tasks awaited. This is
    resource sensitive; treat DoQ shutdown as high priority.

- DoT (`src/server/dot.rs`) and TCP (`src/server/tcp.rs`)
  - Break accept loop on shutdown, close listener, and track spawned
    connection task JoinHandles so they can be awaited with timeout.

- Admin/Monitoring (`src/server/admin.rs`, `src/server/monitoring.rs`)
  - Admin should remain available early in shutdown to trigger and report
    progress; it may be closed late in the sequence. Monitoring can be
    closed at the end but should expose shutdown metrics while draining.

Plugin shutdown ordering
------------------------
- Default strategy: reverse registration/initialization order. Plugin
  registry already records plugins; iterate in reverse and call
  `plugin.shutdown().await` (or use the `as_shutdown()` bridge when the
  `Plugin` trait object is used).
- Classification:
  - High-priority shutdown: plugins that spawn long-lived tasks,
    watchers, or hold outside resource handles (Cron, dataset watchers,
    any connection pools or background workers). These should be stopped
    early in the plugin shutdown phase (right after draining input).
  - Low-priority shutdown: stateless per-request plugins can be shut
    later (or omitted if they have no shutdown behavior).
- Dependency awareness: if a plugin A depends explicitly on plugin B,
  ensure A is shut down before B. If dependency graphs exist, compute a
  safe topological ordering; otherwise use reverse registration.

Coordinator API (suggested)
---------------------------
Provide a small `ShutdownCoordinator` that exposes:

- `fn signal_shutdown(&self)`: trigger shutdown (used by signal/HTTP handlers)
- `async fn wait_for_drained(&self, timeout: Duration)`: wait for active
  requests to reach zero or timeout
- `fn register_active_request(&self)` / `fn unregister_active_request(&self)`:
  helpers (or use RAII guard) to track in-flight work
- `async fn shutdown_plugins(&self, registry: &Registry)`: iterates
  reverse order and calls plugin shutdowns with per-plugin timeout

Example Rust signature (pseudocode)

```rust
struct ShutdownCoordinator { /* channels, counters, config */ }

impl ShutdownCoordinator {
    fn new(cfg: ShutdownConfig) -> Self { .. }
    fn trigger(&self) { /* send shutdown signal */ }
    async fn await_drain(&self, timeout: Duration) -> bool { .. }
    async fn shutdown_plugins(&self, reg: &Registry) { .. }
}
```

Timeouts and configuration
--------------------------
- `drain_timeout` (global): how long to wait for in-flight requests before
  forcing plugin shutdown (default: 10s).
- `plugin_shutdown_timeout`: per-plugin timeout (default: 5s).
- `global_shutdown_timeout`: total bound for full shutdown (default: 30s).

Observability and logging
-------------------------
- Emit logs at each stage: shutdown triggered, server draining started,
  active requests count, plugin shutdown start/complete/timeout.
- Expose metrics: `shutdown_in_progress`, `shutdown_stage`,
  `plugin_shutdown_duration_seconds`, `active_requests`.

Error handling
--------------
- Plugin shutdown failure: log and continue. Record errors for postmortem.
- If critical resource cannot close, log and proceed to forced cleanup at
  `global_shutdown_timeout` expiry.

Testing
-------
- Unit tests for coordinator logic: simulate active requests, trigger
  shutdown, assert drain behavior and timeouts.
- Integration tests: spawn servers that accept a long-running request,
  trigger shutdown, verify new requests are refused, in-flight completes
  or times out, and plugin shutdowns are invoked in expected order.

Implementation roadmap (next steps)
----------------------------------
1. Add `ShutdownCoordinator` type and configuration. Wire into main runtime
   entry point to handle OS signals and Admin API triggers.
2. Update servers to accept a shutdown future or coordinator handle and to
   cooperate with graceful_shutdown / endpoint close semantics.
3. Ensure plugins that hold background handles implement `Shutdown` and
   expose `as_shutdown()` when created as trait objects.
4. Add tests and the `docs/SHUTDOWN.md` (this file).

Appendix: Quick sequence (runtime)
--------------------------------
1. `coordinator.trigger()` called.
2. Servers call `stop_accepting()` (graceful shutdown starts).
3. Coordinator awaits `await_drain(drain_timeout)`.
4. Coordinator calls `stop_generators()` (Cron, watchers).
5. Coordinator calls `shutdown_plugins()` in reverse order.
6. Close remaining endpoints, await JoinHandles (bounded), exit.

-- End of design
