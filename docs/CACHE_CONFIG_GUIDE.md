# Cache Plugin Configuration Guide

## Overview

The Cache plugin in lazydns provides DNS response caching with advanced features like lazy refresh and stale-serving. This guide explains all configuration options and how to optimize caching for your DNS server.

## Configuration Options

### Basic Settings

#### `size`

- **Type**: Integer
- **Default**: 1024
- **Description**: Maximum number of cached entries. When the cache reaches this limit, the least recently used (LRU) entries are evicted.
- **Example**: `size: 2048`

#### `negative_cache`

- **Type**: Boolean
- **Default**: true
- **Description**: Whether to cache negative responses (NXDOMAIN, SERVFAIL). Enabling this reduces load on upstream servers for non-existent domains.
- **Example**: `negative_cache: false`

#### `negative_ttl`

- **Type**: Integer (seconds)
- **Default**: 300
- **Description**: Time-to-live for cached negative responses.
- **Example**: `negative_ttl: 600`

### LazyCache Features

#### `enable_lazycache`

- **Type**: Boolean
- **Default**: false
- **Description**: Enables proactive refresh of cached entries before they expire. When a cached entry is accessed and its remaining TTL falls below the threshold, a background refresh is triggered.
- **Example**: `enable_lazycache: true`

#### `lazycache_threshold`

- **Type**: Float (0.0-1.0)
- **Default**: 0.05
- **Description**: Threshold for triggering lazy refresh. Refresh occurs when remaining TTL percentage drops below this value (such as 0.05 = 5% of original TTL).
- **Example**: `lazycache_threshold: 0.1`

### Stale-Serving (Extended Cache TTL)

#### `cache_ttl`

- **Type**: Integer (seconds)
- **Default**: Not set (disabled)
- **Description**: Enables stale-serving with extended cache duration. When DNS message TTL expires but cache TTL remains, serves stale response with small TTL (5 seconds) and refreshes in background. Set to desired cache duration (such as 600 for 10 minutes).
- **Example**: `cache_ttl: 600`

## Example Configurations

### Basic Cache

```yaml
- tag: cache
  type: cache
  args:
    size: 1024
    negative_cache: true
    negative_ttl: 300
```

### With LazyCache

```yaml
- tag: cache
  type: cache
  args:
    size: 2048
    negative_cache: true
    negative_ttl: 300
    enable_lazycache: true
    lazycache_threshold: 0.05
```

### With Stale-Serving

```yaml
- tag: cache
  type: cache
  args:
    size: 1024
    negative_cache: true
    negative_ttl: 300
    enable_lazycache: false # Disable to test stale-serving
    cache_ttl: 600
```

### High-Performance Cache

```yaml
- tag: cache
  type: cache
  args:
    size: 5000
    negative_cache: true
    negative_ttl: 600
    enable_lazycache: true
    lazycache_threshold: 0.1
    cache_ttl: 1800
```

## Best Practices

### Memory Usage

- Start with `size: 1024` and monitor memory usage.
- Increase size for high-traffic servers, but watch for memory pressure.
- Consider `negative_cache: false` if you prefer to re-query failed domains.

### Performance Tuning

- Enable `enable_lazycache` for hot domains to reduce latency.
- Use lower `lazycache_threshold` (such as 0.02) for more aggressive refresh.
- Set `cache_ttl` to 2-3x typical DNS TTL for stale-serving benefits.

### Monitoring

- Monitor cache hit rates and refresh statistics via admin API (`/api/cache/control`).
- Watch logs for cache misses and refresh activities.
- Use metrics endpoint for Prometheus integration.

## Troubleshooting

### Common Issues

**High Cache Miss Rate**

- Increase `size` if cache is full.
- Check if domains are being skipped (such as via `!qname` conditions).

**Stale Responses Not Working**

- Ensure `cache_ttl` is set and greater than message TTL.
- Disable `enable_lazycache` to isolate stale-serving behavior.
- Wait for message TTL to expire before testing.

**Memory Issues**

- Reduce `size` or disable `negative_cache`.
- Monitor via `top` or admin API.

### Log Messages

- `Cache hit`: Normal cache hit.
- `Lazycache threshold REACHED`: Proactive refresh triggered.
- `LazyCache TTL hit (stale entry)`: Stale-serving activated.
- `Storing response in cache`: New entry cached with TTL info.

## Advanced Usage

### Conditional Caching

Use sequence conditions to skip caching for specific domains:

```yaml
- matches: "!qname example.com"
  exec: $cache
```

### Integration with Other Plugins

Cache works well with:

- `hosts`: Cache static responses.
- `forward`: Cache upstream responses.
- `ttl`: Modify TTLs before caching.

## Migration from Other DNS Servers

If migrating from other DNS servers:

- **Unbound**: Similar to `rrset-cache-size` and `msg-cache-size`.
- **Bind9**: Compare to `max-cache-size` and `max-ncache-ttl`.
- **mosdns**: `cache_ttl` replaces `lazy_cache_ttl`.

Adjust sizes based on your query patterns and available memory.
