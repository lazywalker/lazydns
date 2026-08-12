# Plugins Guide (User)

## Plugin System Overview
Explain plugin execution flow and common plugin types:
- `query` plugins (manipulate DNS queries/responses)
- `exec` plugins (side-effecting tasks)
- `flow` plugins (control flow)

## Key concepts
- Plugin types:
  - **Query** plugins operate on request/response path and may return a DNS response.
  - **Exec** plugins perform side-effecting actions (such as update ipset, download files).
  - **Flow** plugins control execution (such as jump/goto/return semantics).
- **Plugin factory / builder**: registration mechanism that allows plugins to be discovered and built from configuration.
- **Datasets**: text-based domain and IP lists used by dataset plugins; support for auto-reload and merging of files/inline expressions.

## Example plugin sequence (ASCII)
This sequence shows a typical plugin chain where `hosts` provides immediate answers, `cache` handles cached responses, and `forward` sends queries to upstream resolvers when needed.

```
Client
  |
  v
 [Listener]
  |
  v
[Request Handler]
  |
  v
+--------------------------+
| Plugin: hosts            |  => If match -> Respond (short-circuit)
+--------------------------+
  |
  v
+--------------------------+
| Plugin: cache            |  => If cached -> Respond
+--------------------------+
  |
  v
+--------------------------+
| Plugin: forward          |  => Query upstream and return response
+--------------------------+
  |
  v
 Response -> Client
```
This pipeline is intentionally simple: listeners hand requests to a single handler which executes an ordered set of plugins. Plugins may short-circuit the pipeline by producing a response (such as `hosts` or `cache`) or let the request continue to upstreams.



## Built-in Plugins
Short pages or subsections for major plugins with purpose & example config:
- [`forward`](05_02_PLUGIN_FORWARD.md): forward queries to upstreams
- [`cache`](05_03_PLUGIN_CACHE.md): hierarchical cache with TTL handling and LazyCache
- [`hosts`](05_01_PLUGIN_HOSTS.md): static hosts mapping
- [`acl`](05_04_PLUGIN_ACL.md): allow/deny by client IP
- [`geoip`](05_05_PLUGIN_GEOIP_GEOSITE.md): IP-based country tagging (GeoIP)
- [`geosite`](05_05_PLUGIN_GEOIP_GEOSITE.md): domain category tagging (GeoSite)
- [`cron`](05_06_PLUGIN_CRON.md): scheduled background jobs (HTTP, command, invoke-plugin)
- `dataset.*`: domain/ip sets
  * [`domain_set`](05_07_01_PLUGIN_DOMAIN_SET.md): domain matching dataset (full/domain/regexp/keyword)
  * [`ip_set`](05_07_02_PLUGIN_IP_SET.md): extract A/AAAA addresses and materialize ipset entries
- `sequence`: [`Sequence plugin`](05_00_PLUGIN_SEQUENCE.md), a rule-based executor to compose plugins into chains
- `executable.*`: exec-style plugins (downloader, ipset, nftset)
  * [`arbitrary`](05_08_01_PLUGIN_ARBITRARY.md): return predefined DNS records for matching queries
  * [`blackhole`](05_08_02_PLUGIN_BLACK_HOLE.md): return configured A/AAAA answers (sinkhole aliases)
  * [`collector`](05_08_03_PLUGIN_COLLECTOR.md): simple in-process query counter; optional Prometheus collector when built with `metrics` feature
  * [`debug_print`](05_08_04_PLUGIN_DEBUG_PRINT.md): log queries/responses for debugging
  * [`downloader`](05_08_05_PLUGIN_DOWNLOADER.md): download remote files and atomically update local files
  * [`drop_resp`](05_08_06_PLUGIN_DROP_RESP.md): clear any existing response in the execution context
  * [`dual_selector`](05_08_07_PLUGIN_DUAL_SELECTOR.md): filter answers by IPv4/IPv6 preference
  * [`ecs`](05_08_08_PLUGIN_ECS.md): prepare EDNS0 Client Subnet options (ECS)
  * [`edns0opt`](05_08_09_PLUGIN_EDNS0OPT.md): add arbitrary EDNS0 options for upstream queries
  * [`fallback`](05_08_10_PLUGIN_FALLBACK.md): try child plugins in order with automatic failover
  * [`ipset` (exec)](05_08_11_PLUGIN_IPSET_EXEC.md): materialize A/AAAA answers into ipset entries
  * [`mark`](05_08_12_PLUGIN_MARK.md): set lightweight metadata marks on the request
  * [`nftset`](05_08_13_PLUGIN_NFTSET.md): materialize A/AAAA answers into nftables sets
  * [`query_summary`](05_08_14_PLUGIN_QUERY_SUMMARY.md): build and store a concise request summary
  * [`rate_limit`](05_08_15_PLUGIN_RATE_LIMIT.md): per-client rate limiting
  * [`redirect`](05_08_16_PLUGIN_REDIRECT.md): rewrite query names (supports wildcards)
  * [`reverse_lookup`](05_08_17_PLUGIN_REVERSE_LOOKUP.md): cache A/AAAA -> name mappings and answer PTR
  * [`ros_addrlist`](05_08_18_PLUGIN_ROS_ADDRLIST.md): RouterOS address list helper and notifier
  * [`sleep`](05_08_19_PLUGIN_SLEEP.md): pause execution for a duration
  * [`ttl`](05_08_20_PLUGIN_TTL.md): fix or clamp TTL values on responses
- [`domain_validator`](05_09_PLUGIN_DOMAIN_VALIDATOR.md): reject malformed domain names early
- Flow control plugins (used inside sequences):
  * [`goto`](05_09_01_PLUGIN_GOTO.md): jump to a target sequence, replacing current execution
  * [`jump`](05_09_02_PLUGIN_JUMP.md): jump to a target sequence, then return and continue
  * [`accept`](05_09_03_PLUGIN_ACCEPT.md): stop and accept the current response
  * [`reject`](05_09_04_PLUGIN_REJECT.md): reject the query with REFUSED
  * [`return`](05_09_05_PLUGIN_RETURN.md): return from the current jump frame
  * [`prefer_ipv4`](05_09_06_PLUGIN_PREFER_IPV4.md): keep only IPv4 answers
  * [`prefer_ipv6`](05_09_07_PLUGIN_PREFER_IPV6.md): keep only IPv6 answers



## Example plugin configuration
```yaml
plugins:
  - tag: cache
    type: cache
    args:
      size: 10240
```

## Ordering

There is no priority system. Execution order is determined entirely by the `sequence` plugin: steps run top to bottom, and `jump` / `goto` can redirect flow to other sequences.