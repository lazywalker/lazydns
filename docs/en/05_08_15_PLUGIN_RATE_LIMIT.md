# Rate Limit Plugin

`rate_limit` enforces per-client query limits within a time window to mitigate abuse.

## Args

- `max_queries` (int): maximum queries allowed per window (default 100).
- `window_secs` (int): window duration in seconds (default 60).

## Example

```yaml
plugins:
  - tag: rate_limiter
    type: rate_limit
    args:
      max_queries: 100
      window_secs: 60
```

## Behavior

- Tracks request counts per client IP and rejects (sets response code) when limits are exceeded.
- Stores internal counters in an in-memory map and periodically cleans old entries.

## Audit Integration

When the `web` feature is enabled, this plugin will trigger a `rate_limit_exceeded` security event whenever a client's request is rejected.

## When to use

- Place early in the execution chain to protect resources from high-rate clients.
- Useful for public DNS servers or APIs to prevent abuse and ensure fair usage.
