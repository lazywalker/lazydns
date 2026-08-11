# Redirect Plugin

`redirect` rewrites query names from one domain to another (supports wildcard patterns).

## Args / Rules

- Provide a `rules` array in config. Each rule can be a string `"from to"` or a mapping with `from` and `to` keys.
- Rules are checked in order; the first match wins.

Example:

```yaml
plugins:
  - type: redirect
    args:
      rules:
        - "example.com example.net"
        - from: "*.old.com"
          to: "*.new.com"
```

## Behavior

- Performs case-insensitive matching. Supports `*.old.com` -> `*.new.com` wildcard replacements.
- Rewrites the question name in-place so downstream plugins see the new qname.

## When to use

- Useful for domain migrations, testing, or temporary redirect rules without changing client queries.
- Combine with other plugins (such as `hosts`, `forwarder`) to control resolution of redirected names.
