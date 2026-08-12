# Implementation status vs upstream mosdns

Compares lazydns (Rust) against the upstream mosdns feature list (see `UPSTREAM_FEATURES.md`).

## Summary

Core DNS functionality, plugin system, all five transports, cache with persistence and lazy refresh, and a WebUI dashboard are implemented. Gaps are in native netlink integration and some upstream plugin parity.

## 1. Core DNS

Wire-format parse/serialize is built on `hickory-proto` 0.24. See `src/dns/`:

- `message.rs` - DNS message (header + 4 sections); `records()` / `records_mut()` iterate all RRs; DNSSEC bits (AD, CD)
- `question.rs` - Question struct (qname, qtype, qclass)
- `record.rs` - ResourceRecord (name, type, class, ttl, rdata)
- `rdata.rs` - RData enum (A, AAAA, CNAME, NS, PTR, MX, TXT, SOA, SRV, OPT, CAA, DS, RRSIG, NSEC, DNSKEY, etc.)
- `types.rs` - RecordType, RecordClass, OpCode, ResponseCode
- `wire.rs` - parse_message / serialize_message

Record types: A, AAAA, CNAME, MX, NS, PTR, SOA, TXT, SRV fully supported. OPT (EDNS0), DS, RRSIG, NSEC, DNSKEY, SVCB, HTTPS, CAA defined in the enum.

Status: IMPLEMENTED.

## 2. Transports and servers

All five transports via the `Server` trait (`src/server/mod.rs`):

| Transport | File | Feature |
|-----------|------|---------|
| UDP | `udp.rs` | always |
| TCP | `tcp.rs` | always |
| DoT | `dot.rs` | `dot` |
| DoH | `doh.rs` | `doh` |
| DoQ | `doq.rs` | `doq` |
| Admin API | `admin.rs` | `admin` |
| Monitoring | `monitoring.rs` | `metrics` |

`ServerLauncher` (`launcher.rs`) spawns servers from plugin config. A shared `spawn_server` helper handles the oneshot + spawn + error-log pattern for all transport types.

`RequestHandler` trait and `DefaultHandler` (`handler.rs`) wire requests to the plugin entry point, with `RequestContext` carrying client IP and protocol.

Status: IMPLEMENTED.

## 3. Plugin system

Core traits in `src/plugin/`:

- `traits.rs` - `Plugin` (execute, init, aliases, as_any, as_shutdown, spawn_background_task), `ExecPlugin` (quick_setup), `Shutdown`, `BackgroundTask`, `Matcher`
- `context.rs` - `Context` holds request/response Messages and typed metadata; `set_refused()` builds a REFUSED response echoing the request
- `builder.rs` - `PluginBuilder` resolves `$tag` references and builds plugin instances from `PluginConfig`
- `factory.rs` - auto-registration via `#[derive(RegisterPlugin)]` + `linkme::distributed_slice`
- `registry.rs` - runtime lookup by name/tag
- `condition/` - condition builders (qname, qname_neg, qtype, qclass, rcode, has_cname, has_resp, resp_ip, resp_ip_neg)

`PluginHandler` (`mod.rs`) runs the entry plugin, handles control-flow metadata (`goto_label`, `jump_target`, `RETURN_FLAG`), and does post-processing: cache store, reverse-lookup IP save, and audit query logging.

Status: IMPLEMENTED.

## 4. Plugins

### Server-facing

| Plugin | Path | Notes |
|--------|------|-------|
| `forward` | `forward/{mod,engine,builder,types}.rs` | UDP multiplexing (qid demux), DoH (reqwest), concurrent racing, health tracking, load balancing (round-robin/random/fastest) |
| `cache` | `cache/{mod,entry,persistence,stats}.rs` | LRU + LazyCache (pre-expiry background refresh) + stale-serving + binary persistence (`dump_file`/`dump_interval`); cache key includes DNSSEC flags (DO/AD/CD) |
| `hosts` | `dataset/hosts.rs` | HashMap O(1), multiple IPs/domain, file-watch auto-reload |
| `acl` | `acl.rs` | IP-based allow/deny |
| `geoip` | `geoip.rs` | Country-code matching |
| `geosite` | `geosite.rs` | Category/domain matching |
| `domain_validator` | `domain_validator.rs` | RFC 1035/1123 name validation, rejects malformed queries early |
| `rate_limit` | `executable/ratelimit.rs` | Per-IP token-bucket / window limiting |
| `redirect` | `executable/redirect.rs` | Query name rewriting (wildcard, multi-rule, first-match-wins) |
| `ecs` | `executable/ecs.rs` | EDNS Client Subnet |
| `cron` | `cron.rs` | Scheduled tasks (`cronexpr`); drives downloader |

### Executable (inline `exec:` in sequences)

`ttl`, `black_hole`, `arbitrary`, `query_summary`, `debug_print`, `drop_resp`, `sleep`, `dual_selector`, `edns0opt`, `mark`, `reverse_lookup`, `downloader`, `collector` (Prometheus variant under `metrics` feature).

All in `src/plugins/executable/`.

### Datasets

`domain_set` (full/domain/regexp/keyword match types), `ip_set` (CIDR), `arbitrary`. All in `src/plugins/dataset/`.

### Flow control

`sequence`, `goto`, `jump`, `accept`, `reject`, `return`, `prefer_ipv4`, `prefer_ipv6`. In `src/plugins/executable/sequence.rs` and `src/plugins/flow/`.

### Linux integration

`ipset` (`executable/ipset.rs`) and `nftset` (`executable/nftset.rs`) compute CIDR prefixes from A/AAAA answers and invoke `ipset` / `nft` binaries on Linux; record metadata on other platforms.

Status: IMPLEMENTED (CLI-based, not native netlink).

## 5. Cache subsystem

- LRU eviction with periodic cleanup (60s interval, 0.8 pressure threshold)
- LazyCache: proactively refreshes entries when remaining TTL drops below 5%
- Stale-serving via `cache_ttl`: serves stale at TTL=0 while refreshing
- Negative caching with configurable `negative_ttl`
- Persistence: binary dump (`LZDNSCv1` format, atomic temp+rename) to `dump_file` every N changes; loaded on startup and on shutdown

Status: IMPLEMENTED.

## 6. Audit and WebUI

Audit is part of the `web` feature (no standalone plugin). When enabled:

- `PluginHandler` auto-logs every query via `log_query_for_context`
- Plugins emit security events (ACL deny, rate-limit, malformed query) via `AUDIT_LOGGER`
- Event bus (`audit/event_bus.rs`) fans out to SSE stream and alert engine
- WebUI (`src/web/`): real-time dashboard, audit SSE stream, config viewer, admin ops, WebSocket metrics

Status: IMPLEMENTED (feature `web`).

## 7. Metrics

Prometheus gauges/counters in `src/metrics/mod.rs` (cache hits/misses, DNS queries, upstream stats, domain validation). Process memory metrics (RSS/VMS/cgroup) in `src/metrics/memory/`. Exposed via monitoring server (feature `metrics`).

Status: IMPLEMENTED.

## 8. Configuration

YAML config with serde, env var substitution, `!include`, and hot-reload via file watcher (`config/reload.rs`). Validation in `config/validation.rs` checks ranges and required keys for known plugins. Plugin args are free-form YAML parsed by each plugin's `init()`.

Feature flags (`Cargo.toml`): `cron`, `log`, `log-ansi`, `log-file`, `dot`, `doh`, `doq`, `admin`, `metrics`, `web`, `web-embed`. The `full` feature enables everything except `web-embed`.

Status: IMPLEMENTED.

## 9. Testing

Unit tests across all modules (950+ tests). Integration tests in `tests/`:

- `integration_cache.rs`, `integration_ratelimit.rs`, `integration_doq.rs`
- `integration_ipset_nftset.rs`, `integration_save_hook.rs`
- `integration_tls_doh_dot.rs`, `integration_test.rs` (wire format)
- `server_test.rs` (real UDP queries), `web_api_test.rs`

Status: IMPLEMENTED.

## Gaps and next steps

1. Replace CLI-based ipset/nftset with native netlink integration.
2. Add more per-plugin validation coverage (only 5 plugin types validated today).
3. Expand integration tests for multi-plugin sequences.
4. DoH/DoT upstream transport in forward (currently UDP + DoH upstream only).
