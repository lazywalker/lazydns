# Configuration (Core)

This document describes the active configuration shape used by lazydns. The project uses a
plugin-driven configuration where most runtime behavior is provided by plugins declared in the
`plugins` list. Example configuration files are available under the `examples/` directory and are
recommended as a starting point.

## Top-level fields

- `log`: logging configuration
- `admin`: admin API settings (enable, listen address)
- `metrics`: monitoring/Prometheus settings
- `plugins`: ordered list of plugin declarations (data providers, processors, servers, and others)

Note: the legacy `server` section is no longer used; server listeners are configured as
plugins (such as `udp_server`, `tcp_server`, `doh_server`, `dot_server`, `doq_server`).

## Minimal example

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

## Plugin declaration shape

Each entry in the top-level `plugins` array follows this shape:

- `tag` (string): a local name used to reference the plugin from sequences or other plugins.
- `type` (string): plugin type identifier (see plugin pages under `docs/en/05_PLUGINS_USERGUIDE.md`).
- `args` (map): plugin-specific arguments.

Example:

```yaml
- tag: domain_reject_list
  type: domain_set
  args:
    files:
      - "reject-list.txt"
    auto_reload: true
```

## Common plugin argument patterns

- Data providers (domain_set, ip_set, hosts)
  - `files`: list of filenames relative to the configuration directory
  - `exps`: inline expressions (for small lists)
  - `auto_reload`: watch files and reload automatically when changed
  - `default_match_type` (domain_set): `full`, `domain`, `regexp`, or `keyword`

- Downloader / Cron
  - `downloader` accepts a `files` array of `{ url, path }` entries and supports timeouts
    and concurrency options.
  - `cron` provides scheduled `jobs` that can `invoke_plugin` actions to run other plugins.

- Forward/upstream plugins
  - `concurrent`: number of parallel upstream queries
  - `upstreams`: list of `{ addr: "udp://..." | "tcp://..." | "8.8.8.8:53" }`
  - `health_checks`, `max_attempts` for failover behavior

- Sequence/fallback logic
  - `sequence` plugin `args` is an ordered array of steps. Steps support keys like:
    - `exec`: a plugin tag or quick-setup (such as `accept`, `drop_resp`)
    - `matches`: a condition (such as `qname $domain_list`, `has_resp`, `qtype 1`)
    - `jump`: jump to another sequence tag
  - `fallback` plugin accepts `primary` and `secondary`.

- Server plugins (udp_server, tcp_server, doh_server, dot_server, doq_server)
  - `entry`: the sequence tag to use as the processing entry point
  - `listen`: address and port (such as `:5354` or `127.0.0.1:5354`)
  - TLS servers (`doh`, `dot`, `doq`) accept `cert_file` and `key_file` paths

## Paths and updates

- File paths in plugin `args` are interpreted relative to the configuration directory. Use
  absolute paths if you need to locate files outside the config tree.
- For atomic updates, write new dataset files to a temporary location and rename into place.
  Many plugins (downloader, domain_set, ip_set, hosts) rely on atomic replacement to avoid
  partial reads during updates.

## Where to find examples

- Start from `examples/config.example.yaml` for a compact, runnable configuration.
- Look at `examples/etc/config.yaml` for a more complete demo combining downloader, cron,
  dataset providers, cache and server plugins.
- Sample dataset files live under `examples/etc/` (such as `direct-list.txt`, `gfw.txt`,
  `china-ip-list.txt`) and are good templates for building your own lists.

## Further reading

- Plugin reference and usage: `docs/en/05_PLUGINS_USERGUIDE.md`
- Domain matching rules and line syntax: `docs/DOMAIN_MATCHING_RULES.md`
