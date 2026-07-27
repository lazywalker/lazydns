# Upstream mosdns: Feature Summary

This document summarizes the feature set and implementation notes of the upstream `mosdns` project (source: `IrineSistiana/mosdns`). It highlights core capabilities, transport features, plugins, operational behaviors, and recommended improvements that are relevant when maintaining parity or planning feature work in `lazydns`.

---

## 1. Core DNS functionality

- DNS message parsing and serialization (wire format).
- Support for common record types (A, AAAA, CNAME, MX, NS, PTR, SOA, TXT, SRV) and several less-common types (such as SVCB/HTTPS, CAA in type lists).
- Asynchronous, highly concurrent server model (uses goroutines in upstream; `lazydns` maps to `tokio`).

## 2. Transports & Servers

- UDP server (standard DNS over UDP).
- TCP server (DNS over TCP for large responses).
- DoH (DNS over HTTPS): HTTP POST `application/dns-message`.
- DoT (DNS over TLS): TLS-protected DNS (RFC 7858).
- DoQ (DNS over QUIC): planned/experimental in upstream.
- Multi-listen, concurrent handling, and tunable timeouts and limits.

## 3. Plugins & Extensions (select)

The upstream project exposes a plugin architecture with numerous built-in plugins. Key executable and core plugins include:

- forward: Forward queries to upstream resolvers (supports DoH upstreams). Includes health checks, load-balancing strategies (round-robin, random, fastest), concurrent queries (race mode), failover, and per-upstream metrics.
- cache: Caching layer for query responses.
- hosts: Local hosts file parsing and matching (supports ip-first and hostname-first formats, multiple IP entries per line).
- ip_set / nftset: System integration plugins that export IPs to `ipset` or `nft` (CLI) on Linux; they compute prefixes and optionally aggregate CIDRs.
- ros_addrlist: RouterOS (MikroTik) integration for managing address-lists; supports dry-run and aggregation.
- reverse_lookup: Generates reverse PTR records and supports save hooks.
- rate_limiter: Per-client or global query limiting.
- ttl: TTL rewrite/clamping plugin (fixed TTL or min/max range).
- query_summary, metrics_collector: Observability and stats gathering components.
- control & flow: sequence/parallel/if/goto/return/drop_resp; plugins to build control-flow logic.
- executable helpers: arbitrary, sleep, debug_print, drop_resp.

Note: Many plugins have a `QuickSetup` parsing style and both mnemonic and tag-based config options used by the plugin builder.

## 4. Data Providers & File-based Resources

- `domain_set`, `ip_set`, and `hosts` support a set of files or inline data.
- Optional `auto_reload` configuration: when enabled and `files: [...]` are provided, a `FileWatcher` monitors the underlying files and triggers rebuilds on changes.

## 5. FileWatcher / Auto-reload behavior

Key features and improvements in upstream:

- Centralized `FileWatcher` implementation that handles fsnotify events with debouncing and robust handling of REMOVE/RENAME/CREATE scenarios.
- Debounce to avoid duplicate reloads (default ~500ms).
- Re-add and retry logic for REMOVE/RENAME situations (exponential backoff retries) to handle atomic editor saves and rediscovery.
- Scheduled reload after CREATE to avoid missing atomic replace patterns (configurable short delay, default ~250ms).

## 6. Configuration & Environment Overrides

- Uses `viper` to load YAML configuration, with `AutomaticEnv()` enabled and a key replacer mapping `.` -> `_`. This allows environment-based overrides like:

  - `PLUGINS_HOSTS_ARGS_AUTO_RELOAD`
  - `PLUGINS_ADD_GFWLIST_ARGS_PASSWD`

- There is an `applyPluginEnvOverrides` helper that maps `PLUGINS_<IDENT>_ARGS_<KEY>` into plugin `Args` maps. It supports boolean / numeric parsing where appropriate.

## 7. Observability & Logging

- Metrics: Prometheus-style metrics and plugin-level counters (queries, successes, failures, response durations). Plugins optionally expose these metrics when health checks are enabled.
- Logging: `time_format` configurable in `LogConfig` with values like `timestamp`, `iso8601`, `rfc3339`, `custom:<layout>`.
- Tests and docs encourage using readable timestamps and consistent encoding for production vs development.

## 8. Security & Secrets

- Recommendations and docs for secret management (`docs/SECURE_SECRETS.md`): Docker secrets, Kubernetes secrets, Vault, and others.
- Some plugins and helpers are designed to avoid leaking secrets to logs; the env override helper is conscious of secrets handling.

## 9. Testing & CI Practices

- Unit tests for `FileWatcher`, `ros_addrlist` synchronization logic, and time-format and logger behavior.
- Integration tests should avoid modifying repository example files; use `outputDir` or `./tmp/` for test outputs.
- CI should run `go test ./...` and ensure tests do not mutate tracked sample files.

## 10. Deployment & Operations

- Official Docker images are published; prebuilt binaries are available.
- Configurable CLI and flags for working directories and log levels.
- Router/OS integration (ros_addrlist) supports dry-run and multiple sync strategies (replace, append, diff-sync).

## 11. Known suggestions / proposed enhancements (from upstream notes)

- Add `outputDir` and `dry_run` to various writing functions for safer tests and more predictable behavior.
- Consider replacing CLI-based `ipset` / `nft` invocation with native netlink-based implementations for robustness.
- Continue hardening FileWatcher behavior for edge-case filesystems/editors and add tests for the edge cases.
- Improve plugin args decoding after env overrides (map → typed struct conversion) for better ergonomics.

---

## Appendix: Where to look in upstream repo

- Core: `coremain/`, `server/`, `pkg/`, `plugin/`
- Docs & recommendations: `docs/UPSTREAM_CHANGES.md`, `docs/SECURE_SECRETS.md`, `README.md`
- Plugin implementations: `plugin/executable/`, `plugin/data_provider/`

---

(from reading upstream repository source and docs; useful as a checklist when porting features to `lazydns`.)
