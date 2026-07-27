# Hosts Plugin

The `hosts` plugin provides local name-to-IP mappings similar to `/etc/hosts`. It's useful for local overrides, test environments, split-horizon behavior, or to block/redirect specific domains.

## Features

- Static mappings from domain name to one or more IP addresses
- Case-insensitive domain matching; trailing dot tolerated
- Supports both IPv4 and IPv6 (A and AAAA answers)
- Fast in-memory lookup with O(1) complexity
- Load from one or more files and optionally auto-reload on changes
- Plugin priority: **100** (typically placed early in the pipeline)

## Behavior

- Responds only to `A` and `AAAA` queries. Other query types are ignored.
- For matched names, the plugin constructs an authoritative response and sets TTL=3600 on added records.
- If the plugin finds a match it **short-circuits** the pipeline by setting a response in the request `Context`.
- Multiple IPs for a single name produce multiple answers in the response.

## Hosts file syntax

Lines follow a flexible format similar to common hosts files:

- `<ip> <hostname1> [hostname2] ...` (IP first)
- ` <hostname1> [hostname2] ... <ip>` (hostname-first also supported)
- Lines beginning with `#` are comments
- Blank lines are ignored

Examples:

```
# IPv4 and IPv6 for localhost
127.0.0.1    localhost
::1          localhost ip6-localhost
# Multiple names on one line
93.184.216.34 example.com www.example.com
# Hostname-first example (supported)
example.org 203.0.113.5
```

The parser accepts both IP-first and hostname-first formats on the same line and will associate every hostname token with all parsed IPs on that line.

## Configuration

The plugin supports the following configuration keys:

- `files`: (string or sequence) paths to one or more hosts files to load
- `auto_reload`: (bool) enable automatic reload when any watched file changes

Example YAML configuration for the plugin:

```yaml
plugins:
  - tag: hosts
    type: hosts
    args:
      files:
        - examples/etc/hosts.txt
      auto_reload: true
```

Notes:
- Files are aggregated (combined) and parsed together when the plugin initializes and when auto-reload triggers.
- If `auto_reload` is enabled, the plugin watches the configured files and reloads after small debounce delay.

## Debugging and Troubleshooting

- Increase verbosity (such as `-v` / `-vv`) to see plugin initialization logs.
- Common log messages:
  - `Hosts loaded (wrapper)`: indicates hosts were successfully loaded and reports number of entries and files.
  - `Failed to read hosts file`: read error for a configured file (file missing or permission issue).
  - `Failed to parse hosts file during auto-reload`: parsing error (invalid IP/token) when reloading.
- If a hostname is not matching:
  - Verify the entry exists and has a valid IP address
  - Ensure plugins are ordered so `hosts` runs before any plugin that would short-circuit or override its response

## Best practices

- Place `hosts` early in the pipeline (default priority=100) to ensure local overrides are applied before expensive upstream queries.
- Use `auto_reload: true` during development for fast iteration, and consider disabling it for production deployments if filesystem stability is a concern.
- Keep host files small and focused (such as project-specific overrides) if you use auto-reload to avoid frequent reloads.

## See also

- [Pipeline examples](01_INTRODUCTION.md)
