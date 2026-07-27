# Dual Selector Plugin

`dual_selector` filters DNS answers by IPv4/IPv6 preference. It is useful when you need to prefer A or AAAA records, or provide fallback behavior between the two address families.

## Preferences

- `ipv4`: keep only A records
- `ipv6`: keep only AAAA records
- `ipv4_prefer_ipv6_fallback`: prefer IPv4; if none, keep IPv6
- `ipv6_prefer_ipv4_fallback`: prefer IPv6; if none, keep IPv4
- `both`: keep both A and AAAA

## Configuration (plugin args)

The `dual_selector` plugin is configured via `args.preference` in the plugin config.

```yaml
plugins:
  - tag: dual
    type: dual_selector
    args:
      preference: "ipv4" # ipv4 | ipv6 | ipv4_prefer_ipv6_fallback | ipv6_prefer_ipv4_fallback | both
```


## When to use

- Use when you need to enforce IP family policy for clients that prefer one family or for environments where only one family is routable.

