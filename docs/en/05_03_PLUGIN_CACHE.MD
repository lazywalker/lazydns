# Cache Plugin

The `cache` plugin stores DNS responses to reduce upstream traffic and improve latency. It supports TTL-respecting caching, negative caching, LRU eviction, prefetch and a LazyCache optimization for refreshing hot entries before they expire.

## Key features

- TTL-based expiration and remaining-TTL updates when serving cached responses
- LRU-style eviction when cache reaches configured size
- Negative caching (cache NXDOMAIN/SERVFAIL) with configurable TTL
- Optional prefetch (refresh before expiry) and LazyCache (background refresh of hot items)
- Stale-serving with `cache_ttl` (return stale response and refresh in background)
- Statistics: hits, misses, evictions, expirations
- Default priority: **50** (run early to serve cached responses)

## Behavior details

- The plugin computes a cache key from the first question: `lowercase-qname:qtype:qclass`.
- On cache hit the plugin returns a cached response with TTLs adjusted to the remaining message TTL.
- If the message TTL expires but `cache_ttl` is configured, the plugin can return a stale response (short TTL) and trigger a background refresh.
- LazyCache: when enabled, hot entries whose remaining TTL drops below `lazycache_threshold` will be refreshed in background to avoid misses.
- [TODO] Prefetch: similar to lazycache but uses a prefetch threshold to perform synchronous/async refresh before expiry. 

## Configuration options

- `size` (number, default: `1024`): maximum number of cache entries
- `negative_cache` (bool, default: `false`): enable caching of negative (NXDOMAIN/SERVFAIL) responses
  - `negative_ttl` (number, default: `300`): TTL for negative cache entries (seconds)
- `enable_lazycache` (bool, default: `false`): enable LazyCache hot-entry background refresh
  - `lazycache_threshold` (number, default: `0.05`): fraction (0.0–1.0) of TTL when lazy refresh is considered
- `cache_ttl` (number, default: disabled): if set, stale responses are served for this many seconds while a background refresh happens

## Example configuration

```yaml
plugins:
  - tag: my_cache
    type: cache
    config:
      size: 2048
      negative_cache: true
      negative_ttl: 300
      enable_lazycache: true
      lazycache_threshold: 0.05
      cache_ttl: 30
```

Place `cache` early in your pipeline (priority=50) so it can return cached responses before heavier plugins run.


## Metrics & stats

- `CacheStats` exposes counters and a human-readable summary in logs and (when metrics are enabled) Prometheus metrics:
  - cache hits, misses, evictions, expirations
- LazyCache maintains its own stats (refresh attempts, successes, failures)

## Troubleshooting

- If you observe low hit rates:
  - Increase cache `size` or review key distribution (ensure qname/qtype/qclass are stable)
  - Ensure `cache` is placed early in the pipeline and not shadowed by other plugins
- If `cache_ttl` stale-serving returns unexpected results, check the `lazy_refresh_handler` metadata and ensure the pipeline includes a handler capable of re-querying upstreams (typical in normal server pipelines).
- Watch logs for: `Cache hit`, `Cache entry expired`, `Evicted LRU cache entry` to diagnose behavior.

## Best practices

- Size the cache to your traffic and memory constraints; default `1024` entries is a starting point.
- Use `negative_cache` to reduce repeated failing queries for known-bad names.
- Consider enabling `lazycache` when your upstreams are stable and you want to keep hot entries refreshed without traffic spikes from cache misses.
- Use `cache_ttl` carefully: it serves stale responses during refresh but should be short enough to avoid long-lived stale answers.
