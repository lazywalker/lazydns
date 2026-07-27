# Mark Plugin

`mark` sets lightweight metadata marks on the request `Context`. Marks may be boolean (presence) or string values and can be read by downstream plugins for routing or policy decisions.

## Arguments

- `name` (required): mark key
- `value` (optional): string value to set; if omitted the mark is boolean `true`.

## Exec quick-setup

Exec string format: `"mark key [value]"`.

Examples:

```yaml
plugins:
  - exec: mark:vip_customer
  - exec: mark:priority high
```

## Behavior

- Stores boolean marks as `bool` metadata and value marks as `String` metadata under the provided key.
- Setting the same mark twice overwrites the previous value.

## When to use

- Use `mark` to annotate requests for conditional plugin logic (such as bypassing cache, preferring upstreams, or tagging telemetry).
- Marks can be used in combination with other plugins that read metadata to influence behavior based on request attributes.