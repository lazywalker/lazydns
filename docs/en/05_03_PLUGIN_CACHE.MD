# Cache Plugin

The `cache` plugin stores DNS responses to reduce upstream traffic and improve latency. It supports TTL-respecting caching, negative caching, LRU eviction, LazyCache background refresh, stale-serving, and persistence across restarts.

## Key features

- TTL-based expiration with remaining-TTL updates on serve
- LRU eviction when cache reaches configured size
- Negative caching (NXDOMAIN/SERVFAIL) with separate TTL
- LazyCache: background refresh of hot entries before expiry
- Stale-serving via `cache_ttl`: return stale response while refreshing
- DNSSEC-safe cache keys (DO/AD/CD flags included)
- Persistence: dump/load cache to survive restarts

## Configuration options

- `size` (number, default: `1024`): maximum cache entries
- `negative_cache` (bool, default: `false`): cache error responses
  - `negative_ttl` (number, default: `300`): TTL for negative entries (seconds)
- `enable_lazycache` (bool, default: `false`): refresh hot entries in background
  - `lazycache_threshold` (number, default: `0.05`): fraction of original TTL below which refresh triggers
- `cache_ttl` (number, default: disabled): stale-serving window in seconds
- `dump_file` (string, optional): path to persist cache across restarts
  - `dump_interval` (number, default: `600`): seconds between periodic dumps

## Persistence

When `dump_file` is set, the cache is saved to a binary file on shutdown and periodically (every `dump_interval` seconds, or after 1024 writes). On startup, the file is loaded and entries that have not yet expired are restored. Entries are stored in DNS wire format with their original TTL and cache timestamp.

## Example

```yaml
plugins:
  - tag: my_cache
    type: cache
    args:
      size: 2048
      negative_cache: true
      negative_ttl: 300
      enable_lazycache: true
      lazycache_threshold: 0.05
      cache_ttl: 600
      dump_file: /var/lib/lazydns/cache.dump
      dump_interval: 300
```

## Metrics

- `lazydns_cache_hits_total` / `lazydns_cache_misses_total`
- `lazydns_cache_evictions_total` / `lazydns_cache_expirations_total`
- `lazydns_cache_size` (current entries)
