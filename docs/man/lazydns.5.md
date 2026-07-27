---
title: lazydns
section: 5
version: "0.2.50"
date: 2025-12-28
manual: "lazydns manual"
---

## NAME

lazydns \- configuration file format and options

## DESCRIPTION

This page documents the YAML configuration schema used by `lazydns`.
The configuration is expressed in YAML and controls listeners, plugins,
upstreams, caching, metrics, and the admin interface.

The following is a concise schema description of commonly used fields. The
exact layout and optional fields may evolve; consult shipped example
configuration for the project version you run.

## CONFIGURATION FIELDS

- `log` (mapping)
:  Logging configuration. Common keys: `level` (info/debug/trace) and
  `format` (text/json). Example:

  ```yaml
  log:
    level: info
    format: text
  ```

- `admin` (mapping)
:  Administrative HTTP endpoint configuration:

  ```yaml
  admin:
    enabled: true
    addr: "127.0.0.1:8000"
  ```

- `metrics` (mapping)
:  Prometheus metrics options:

  ```yaml
  metrics:
    enabled: true
    addr: "127.0.0.1:8001"
  ```

- `plugins` (sequence)
:  Ordered list of plugin declarations. Each entry is an object with a
  `tag`, `type` and optional `args`. Server listeners are configured as
  plugins (for example `udp_server`, `tcp_server`, `doh_server`, `dot_server`,
  `doq_server`) and should be declared in the `plugins` array.

  Example plugin entry:

  ```yaml
  - tag: forward
    type: forward
    args:
      concurrent: 2
      upstreams:
        - addr: "8.8.8.8:53"
        - addr: "1.1.1.1:53"
  ```

## EXAMPLES

Minimal example configuration:

```yaml
# Logging
log:
  level: info
  format: text

# Enable admin API and metrics
admin:
  enabled: true
  addr: "127.0.0.1:8000"

metrics:
  enabled: true
  addr: "127.0.0.1:8001"

# Plugin list (each plugin is an object with `tag`, `type` and `args`)
plugins:
  - tag: forward
    type: forward
    args:
      concurrent: 2
      upstreams:
        - addr: "8.8.8.8:53"
        - addr: "1.1.1.1:53"

  - tag: udp_server
    type: udp_server
    args:
      entry: forward
      listen: ":5354"
```

## FILE LOCATION

See github: https://github.com/lazywalker/lazydns

Start from `examples/config.example.yaml` for a compact, runnable
configuration. A more complete demo is available at `examples/etc/config.yaml`.
Sample dataset files live under `examples/etc/` (for example `direct-list.txt`,
`gfw.txt`, `china-ip-list.txt`).

## SEE ALSO

`lazydns.1`(1)
