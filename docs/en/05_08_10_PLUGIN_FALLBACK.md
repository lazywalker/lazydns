# Fallback Plugin

`fallback` tries a sequence of child plugins in order, falling back to the next when a child returns an error or (by default) when its response is empty. It can be configured to only fallback on errors.

## Modes of configuration

- Declarative config via `primary` / `secondary` (and similar) names in `args`.
- Exec quick-setup with a comma-separated list of plugin names to try in order.

## Arguments

- `primary` (string): primary child plugin name (optional)
- `secondary` (string): secondary child plugin name (optional)
- `error_only` (bool): when true, only fallback on errors (default: false)

## Examples

Configuration using names:

```yaml
plugins:
  - type: fallback
    args:
      primary: upstream_a
      secondary: upstream_b
```

Exec quick-setup (compact):

```yaml
plugins:
  - exec: fallback:upstream_a,upstream_b
```

## Behavior

- Tries each child plugin in order. If a child returns an error, the fallback logic moves to the next child.
- By default, an empty response from a child also triggers a fallback. Set `error_only=true` to only fallback on errors.
- The plugin supports unresolved child names (pending resolution) that can be resolved later via a plugin registry.

## When to use

- Use `fallback` when you have multiple upstreams or strategies and want automatic failover or secondary attempts when the primary fails or produces no answers.

