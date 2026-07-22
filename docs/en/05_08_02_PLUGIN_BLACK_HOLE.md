# Black Hole (blackhole) Plugin

The `blackhole` plugin (also aliased as `sinkhole`, `black_hole`, `null_dns`) returns configured A/AAAA answers for matching queries. It's a lightweight authoritative-like plugin useful for sinkholing domains, local overrides, or testing.

## Behavior

- If the incoming query contains a single question of type A or AAAA and the plugin is configured with matching addresses, it returns those addresses as answers.
- If no matching type or no configured addresses, the plugin does nothing and allows the pipeline to continue.
- The plugin is safe to place early in a pipeline to short-circuit upstream queries for specific names.

## Configuration

The plugin accepts an `ips` argument (sequence of IP strings) or can be created via the exec quick-setup shorthand.

- `ips`: list of IPv4/IPv6 address strings.

Example (YAML):

```yaml
plugins:
  - tag: blackhole
    type: blackhole
    config:
      ips:
        - 192.0.2.1
        - 2001:db8::1
```

Quick exec-style shorthand (exec plugin):

```yaml
plugins:
  - exec: blackhole:192.0.2.1,2001:db8::1
```

Accepted prefixes for exec quick-setup: `blackhole`, `black_hole`, `sinkhole`, `null_dns`.

## Response details

- For A queries, configured IPv4 addresses are returned as A records.
- For AAAA queries, configured IPv6 addresses are returned as AAAA records.
- TTL used in records defaults to 300 seconds (as constructed by the plugin).

## Audit Integration

When matching a query and returning a response, this plugin will trigger a `blocked_domain_query` security event (published automatically when the `web` feature is enabled).

## Aliases

- `blackhole`, `black_hole`, `sinkhole`, `null_dns` are all recognized aliases; the plugin registers these for convenience.

## Examples & use cases

- Sinkhole known-malicious domains by tagging them with `blackhole` plugin configured with a controlled IP.
- Provide quick local overrides for testing services without running a full authoritative server.
- Use in exec sequences for simple response injection in tests.

## Troubleshooting

- If no answers are returned, confirm the plugin was configured with appropriate IPv4 addresses for A queries or IPv6 for AAAA queries.
- For exec quick-setup ensure the prefix (before `:`) is one of the supported aliases.

