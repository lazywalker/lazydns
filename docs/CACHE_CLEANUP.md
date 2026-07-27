# Cache Cleanup Configuration Guide

## Overview

LazyDNS Cache Plugin now includes a **periodic cleanup mechanism** to actively manage memory usage by removing expired cache entries and detecting memory pressure conditions.

### Problem Solved

Previously, when cache entries expired, they would remain in memory until:
1. The cache became full and LRU eviction kicked in
2. The entry was accessed and detected as expired

This could lead to **memory buildup in low-traffic scenarios** where expired entries accumulate without being accessed.

## Configuration Parameters

The cache plugin now supports three cleanup-related parameters:

### 1. `enable_cleanup` (boolean, default: `true`)
Enable or disable the periodic cleanup task.

```yaml
plugins:
  - type: cache
    args:
      size: 10000
      enable_cleanup: true  # Enable periodic cleanup
```

### 2. `cleanup_interval_secs` (integer, default: `60`)
How often (in seconds) the cleanup task should run.

```yaml
plugins:
  - type: cache
    args:
      size: 10000
      enable_cleanup: true
      cleanup_interval_secs: 30  # Run cleanup every 30 seconds
```

**Recommended values:**
- **30-60 seconds**: Default range, balances memory management with CPU overhead
- **10-20 seconds**: For high-traffic servers with large caches
- **120+ seconds**: For low-traffic servers or memory-constrained environments

### 3. `cleanup_pressure_threshold` (float, 0.0-1.0, default: `0.8`)
Trigger cleanup when cache reaches this percentage of maximum size.

```yaml
plugins:
  - type: cache
    args:
      size: 10000
      enable_cleanup: true
      cleanup_interval_secs: 60
      cleanup_pressure_threshold: 0.8  # Cleanup at 80% capacity
```

**Behavior:**
- At regular intervals, the cleanup task removes expired entries
- If cache usage exceeds the threshold, an additional cleanup pass is triggered
- This helps prevent memory pressure spikes before LRU eviction kicks in

**Recommended values:**
- **0.7-0.8**: For balanced memory management
- **0.5-0.6**: For aggressive cleanup (more frequent)
- **0.9+**: For conservative cleanup (less frequent)

## Complete Configuration Example

### Scenario 1: High-Traffic Server (Recommended)

```yaml
plugins:
  - tag: cache
    type: cache
    args:
      size: 50000              # Large cache
      negative_cache: true
      negative_ttl: 300
      enable_cleanup: true
      cleanup_interval_secs: 30       # Check every 30 seconds
      cleanup_pressure_threshold: 0.75 # Cleanup at 75% capacity
      enable_lazycache: true           # Boost with lazy refresh
      lazycache_threshold: 0.05
```

**Benefits:**
- Active memory management prevents unbounded growth
- LazyCache keeps hot entries fresh without cache misses
- Pressure-based cleanup prevents memory pressure spikes

### Scenario 2: Low-Traffic Server (Conservative)

```yaml
plugins:
  - tag: cache
    type: cache
    args:
      size: 5000
      negative_cache: true
      negative_ttl: 300
      enable_cleanup: true
      cleanup_interval_secs: 120       # Check every 2 minutes
      cleanup_pressure_threshold: 0.85 # Only cleanup when near full
```

**Benefits:**
- Less frequent cleanup reduces CPU overhead
- Cache has room to grow without constant pressure
- Still prevents memory leaks from expired entries

### Scenario 3: Memory-Constrained Environment

```yaml
plugins:
  - tag: cache
    type: cache
    args:
      size: 1000
      negative_cache: false           # Disable negative caching to save memory
      enable_cleanup: true
      cleanup_interval_secs: 10       # Aggressive cleanup every 10 seconds
      cleanup_pressure_threshold: 0.6 # Cleanup at 60% capacity
```

**Benefits:**
- Tight memory management
- Frequent cleanup ensures minimal unused entries
- Negative caching disabled to reduce memory footprint

### Scenario 4: Cleanup Disabled (Manual Management)

```yaml
plugins:
  - tag: cache
    type: cache
    args:
      size: 10000
      enable_cleanup: false           # Rely on LRU eviction only
```

**Use cases:**
- Very high-traffic servers where LRU is sufficient
- Custom memory management via external tools
- Testing/debugging scenarios

## How It Works

### Periodic Cleanup Process

1. **Timer Tick**: At each `cleanup_interval_secs` interval, a cleanup task runs
2. **Expiration Scan**: Iterates through all cache entries and collects expired ones
3. **Removal**: Deletes expired entries and updates statistics
4. **Pressure Check**: If cache size > `max_size * cleanup_pressure_threshold`, run cleanup again
5. **Logging**: Records number of entries removed in debug logs

### Memory Impact

- **Without cleanup**: Expired entries stay in memory until eviction by LRU or access detection
- **With cleanup**: Expired entries removed within `cleanup_interval_secs` seconds
- **Pressure threshold**: Prevents cache from silently filling up to capacity with stale entries

### Performance

- **CPU overhead**: Minimal - O(n) scan only at intervals
- **Memory overhead**: Negligible - reuses existing cache structure
- **No blocking**: Cleanup runs asynchronously in background task

## Monitoring

Check cache cleanup activity in logs:

```
[DEBUG] Cleanup removed 42 expired cache entries
[DEBUG] Memory pressure detected: 8234 / 10000
[DEBUG] Pressure cleanup removed 156 entries (total in this cycle: 198)
```

### Admin API

The cache statistics API shows cleanup impact:

```bash
curl http://localhost:5380/api/cache/stats
```

Example response:
```json
{
  "size": 1234,
  "hits": 5000,
  "misses": 500,
  "evictions": 42,
  "expirations": 198,
  "hit_rate": 0.909
}
```

- `size`: Current number of entries
- `expirations`: Total entries removed by cleanup or expiration check

## Best Practices

1. **Start with defaults**: `cleanup_interval_secs: 60`, `cleanup_pressure_threshold: 0.8`
2. **Monitor memory**: Watch memory usage and adjust `size` as needed
3. **Tune interval**: Increase interval for low-traffic servers to reduce overhead
4. **Use LazyCache**: Combine cleanup with `enable_lazycache` for best performance
5. **Watch logs**: Check debug logs for cleanup activity to verify it's working

## Troubleshooting

### Memory Still Growing

**Symptoms**: Memory usage increases over time despite cleanup being enabled

**Solutions**:
1. Lower `cleanup_pressure_threshold` (such as 0.7 instead of 0.8)
2. Reduce `cleanup_interval_secs` (such as 30 instead of 60)
3. Reduce cache `size` if possible
4. Check if TTLs are too long (entries expire slowly)

### High CPU Usage

**Symptoms**: CPU spikes every `cleanup_interval_secs` seconds

**Solutions**:
1. Increase `cleanup_interval_secs` (such as 120 instead of 60)
2. Reduce cache `size` (fewer entries = faster cleanup)
3. Disable cleanup if cache is small and LRU is sufficient

### No Entries Being Cleaned

**Symptoms**: "Cleanup removed 0 entries" in logs constantly

**Potential causes**:
- Entries haven't expired yet (check TTL values)
- Cleanup threshold not reached (entries still being accessed, updating TTL)
- Enable_cleanup is false

**Verification**:
```bash
# Check if entries are expiring
grep "Cache entry expired" /var/log/lazydns.log
```

## Summary

The cache cleanup mechanism provides:

| Feature | Benefit |
|---------|---------|
| **Periodic expiration removal** | Prevents memory buildup from stale entries |
| **Pressure-based triggers** | Detects and reacts to memory pressure |
| **Configurable intervals** | Tunable for different traffic patterns |
| **Async background task** | No impact on DNS query latency |
| **Detailed statistics** | Observable through logs and API |

By properly configuring cleanup parameters, you can ensure your DNS cache stays healthy and efficient across all traffic scenarios.
