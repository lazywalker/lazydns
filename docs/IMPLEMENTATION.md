# Implementation status vs upstream mosdns

This document summarizes the current implementation status of the Rust `lazydns` project against the upstream `mosdns` feature list (see `upstream-features.md`). It lists implemented features, partial implementations, and known gaps. Paths reference current source files where applicable.

## Summary

- Overall status: large portion of core features and many plugins implemented in Rust with the goal of parity.
- Focus so far: plugin architecture, forward/cache/hosts, control-flow plugins, and executable plugins including `reverse_lookup`, `ipset`, and `nftset`.

## 1. Core DNS functionality

- DNS parsing & serialization: Implemented. See `src/dns/*` (message, wire, record, rdata, types).
- Supported record types: implemented for the common set (A, AAAA, CNAME, MX, NS, PTR, SOA, TXT, SRV). SVCB/HTTPS and CAA are present in `RecordType` definitions (`src/dns/types.rs`).

Status: IMPLEMENTED (core parsing and record support).

## 2. Transport & server features

- UDP and TCP servers: Implemented (`src/server/udp.rs`, `src/server/tcp.rs`).
- DoT (DNS over TLS): Implemented (`src/server/dot.rs`, `src/server/tls.rs`).
- DoH (DNS over HTTPS): Implemented (`src/server/doh.rs`).
- DoQ (DNS over QUIC): implemented (`src/server/doq.rs`).
- Multi-listen, concurrency, connection handling: Implemented via `tokio`-based servers (`src/server/*`).

Status: PARTIAL: UDP/TCP/DoH/DoT/DoQ present, not all features.

## 3. Plugin system

- Plugin architecture: Implemented (`src/plugin/*`, `src/plugins/mod.rs`).
- Execution flow, context, and conditional execution: Implemented (`src/plugin/context.rs`, `src/plugins/advanced.rs`, `src/plugin/builder.rs`).

### Core plugin coverage (select)

- `forward`: Implemented (`src/plugins/forward.rs`); supports multiple upstreams and concurrent queries. Transport feature parity (DoH/DoT/DoQ upstream) is partial on transport side.
- `cache`: Implemented (`src/plugins/cache/mod.rs`). - TODO: `lazy_cache_ttl`
- `hosts`: Implemented (`src/plugins/hosts.rs`). Parser supports both ip-first and hostname-first lines, multiple IPs per line, and mixed ordering across files; unit tests verify A/AAAA behavior and hostname-first parsing.
- `domain_set` / `geosite`: Implemented (`src/plugins/domain_matcher.rs`, `src/plugins/geosite.rs`).
- `ip_set` / IP matching: Implemented (`src/plugins/ip_matcher.rs`, `src/plugins/data_provider.rs`).
- `geoip`: Implemented (`src/plugins/geoip.rs`); GeoIP integration present; check for data loader details.

### Executable & control plugins

- `sequence`, `parallel`, `if`, `goto`, `return`, `drop_resp`: Implemented (`src/plugins/advanced.rs`, `src/plugins/control_flow.rs`).
- `ttl`: Implemented (`src/plugins/executable/ttl.rs`).
- `query_summary`: Implemented (`src/plugins/executable/query_summary.rs`).
- `reverse_lookup`: Implemented with in-memory cache and save hook (`src/plugins/executable/reverse_lookup.rs`). Integration: `PluginHandler` calls `save_ips_after` after response population.
- `arbitrary`, `black_hole`, `drop_resp`: Implemented in `src/plugins/executable/*.rs`.

### ipset / nftset integration

- `ipset`: Implemented (`src/plugins/executable/ipset.rs`). Behavior:

  - Computes CIDR prefixes from A/AAAA answers.
  - QuickSetup parser present.
  - On Linux, invokes system `ipset` binary via `std::process::Command` (guarded with `cfg(target_os = "linux")`).
  - On other platforms records metadata (`ipset_added`) for tests/visibility.

- `nftset`: Implemented (`src/plugins/executable/nftset.rs`). Behavior mirrors `ipset`:
  - Computes prefixes, QuickSetup parser.
  - On Linux uses `nft` binary; otherwise records metadata (`nftset_added_v4`, `nftset_added_v6`).

Status: IMPLEMENTED (CLI-based integration). Note: upstream native netlink integration is not used; a native implementation could be added later.

## 4. Configuration system

- YAML config loader and validation: Implemented (`src/config/*`) with `PluginBuilder` and `PluginConfig` parsing. Example configs included in `examples/etc/config.yaml`.
- Hot reload: partial; `ConfigReloader` exists, verify runtime hot-reload semantics for production.

Status: PARTIAL: YAML loading and validation implemented; hot-reload present as a reloader component.

## 5. Advanced features

- Performance: designed for async `tokio` concurrency; memory pools and advanced tuning are incremental work (some pool utilities exist in project).
- Observability: metrics and monitoring modules exist (`src/server/monitoring.rs`, `src/metrics` planned). Prometheus-style exposure may be partial.
- Security: TLS support for DoT/DoH implemented. Certificate handling present in `src/server/tls.rs`.

Status: PARTIAL: basic observability and TLS present; more integrations possible.

## 6. Deployment & management

- Standalone binary and Docker artifacts: project includes `Dockerfile` and `docker-compose.yml` in workspace root.
- CLI flags and signal handling: implemented in `src/main.rs` (config path, working dir, log level, graceful shutdown via ctrl-c).

Status: IMPLEMENTED (basic deployment support present).

## 7. Testing coverage

- Unit tests: extensive unit tests across DNS, plugin, and executable modules (run via `cargo test`).
- Integration tests: added integration tests for the reverse-lookup save hook and ipset/nftset metadata behavior under `tests/`.

Status: IMPLEMENTED: good test coverage; integration tests added for key behaviors.

## Gaps and recommended next steps

1. DoQ (DNS over QUIC): implement DoQ server and transport support to match upstream feature set.
2. Replace CLI-based ipset/nft manipulation with native netlink integration (via a Rust netlink crate) for more robust system integration and error handling.
3. Expand documentation per-plugin (config examples and QuickSetup documentation) and add README snippets linking `examples/etc/config.yaml` to plugin behaviors.
4. Add further integration tests for multi-plugin sequences (such as forward->ipset->ros_addrlist flow) and permissioned system behaviors.
5. Verify Prometheus metrics coverage and add exporter where missing.

## File references (key files)

- Core DNS: `src/dns/*` (types.rs, message.rs, record.rs, wire.rs)
- Server: `src/server/*` (`udp.rs`, `tcp.rs`, `doh.rs`, `dot.rs`)
- Plugin system: `src/plugin/*`, `src/plugins/*`
- Executable plugins: `src/plugins/executable/*` (includes `ipset.rs`, `nftset.rs`, `reverse_lookup.rs`, `ttl.rs`, `query_summary.rs`)
- Config and examples: `src/config/*`, `examples/etc/config.yaml`
