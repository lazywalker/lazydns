# Forward Plugin

The `forward` plugin forwards DNS queries to upstream resolvers. It supports multiple upstreams, load-balancing strategies, failover, optional health checks, and DNS-over-HTTPS (DoH).

## Key features

- Multiple upstreams (UDP/TCP or DoH URLs)
- Load-balancing strategies: `round_robin`, `random`, `fastest`
- Optional health checks with per-upstream metrics
- Failover with configurable `max_attempts`
- Optional concurrent/race queries (`concurrent`)
- Supports upstream tagging (for logs/metrics) via `addr|tag` syntax

## Behavior details

- The plugin answers queries by forwarding them to upstream servers when no previous plugin produced a response.
- If `concurrent` mode is enabled (legacy numeric `concurrent` > 1), the plugin races queries to all upstreams and uses the first successful response.
- Otherwise, it will attempt a sequential failover using the configured load-balancing strategy and `max_attempts`.
- DoH upstreams are supported by using HTTP(S) URLs (such as `https://dns.example/dns-query`).
- For UDP/TCP upstream strings you can optionally prefix with `udp://` or `tcp://`; the plugin will accept either and normalize addresses to include a port (default port 53 if missing).

## Configuration options

The plugin accepts an `args` map with the following keys:

- `upstreams` (required): an array of upstreams. Each entry can be either a string or a mapping.
  - String forms:
    - `8.8.8.8:53`
    - `udp://8.8.8.8:53` (prefix optional)
    - `https://dns.example/dns-query` (DoH)
    - `8.8.8.8:53|primary` (append `|tag` to provide a human tag)
  - Mapping form:
    - `{ addr: "1.1.1.1:53", tag: "cloudflare" }`
- `timeout`: integer seconds for per-query timeout (default: 5)
- `strategy`: one of `round_robin` (default), `random`, or `fastest`
- `health_checks`: boolean to enable health tracking (default: false)
- `max_attempts`: integer controlling maximum failover attempts (default: 3)
- `concurrent`: integer (legacy); values > 1 enable racing concurrent queries

## Example configurations

Basic forwarder with two UDP upstreams:

```yaml
plugins:
  - tag: forward
    type: forward
    args:
      upstreams:
        - "8.8.8.8:53"
        - "1.1.1.1:53|cloudflare"
      timeout: 5
      strategy: round_robin
      health_checks: true
      max_attempts: 3
```

Using DoH and fastest strategy:

```yaml
plugins:
  - tag: forward_secure
    type: forward
    args:
      upstreams:
        - "https://doh.example/dns-query"
        - "https://doh2.example/dns-query|doh2"
      strategy: fastest
      timeout: 3
      health_checks: true
```

Concurrent (race) mode (legacy `concurrent` > 1):

```yaml
plugins:
  - tag: forward_race
    type: forward
    args:
      upstreams:
        - "8.8.8.8:53"
        - "1.1.1.1:53"
      concurrent: 2
```

## Logs and metrics

- The plugin logs upstream query attempts, successes, failures and average response times.
- When `health_checks` is enabled the plugin updates per-upstream counters and (if metrics feature compiled) exposes Prometheus metrics:
  - `UPSTREAM_QUERIES_TOTAL` (labels: upstream, status)
  - `UPSTREAM_DURATION_SECONDS` (labels: upstream)

Look for log fields like `upstream`, `elapsed_ms`, `queries`, `successes`, and `failures` when debugging upstream behavior.

## Troubleshooting

- If queries fail:
  - Verify upstream addresses and ports are reachable from the host.
  - For DoH endpoints, verify TLS settings and consider `LAZYDNS_DOH_ACCEPT_INVALID_CERT` in test contexts.
  - Increase `timeout` for slow networks or remote resolvers.
- If health checks are enabled and many failures are reported, consider removing a flaky upstream or adjusting `max_attempts`.
- Use `strategy: fastest` to prefer faster upstreams automatically once response times have been measured.
- For race/concurrent mode, watch for increased upstream query load (multiple upstreams are queried in parallel).

## Best practices

- Use tags (`addr|tag`) to label upstreams for easier log/metric identification.
- Prefer `fastest` strategy when you have reliable latency metrics and mixed upstreams (DoH vs UDP).
- Enable `health_checks` if you want the plugin to track upstream performance and failures and to drive more informed failover.

## See also

- [Deployment recipes and forwarding examples](09_EXAMPLES_AND_RECIPES.md)
- [Plugin guide overview](05_PLUGINS_USERGUIDE.md)