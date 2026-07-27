# IP_SET Plugin

The `ip_set` plugin provides a lightweight IP dataset: it loads IP addresses and CIDR ranges from files or inline entries, exposes the compiled networks for other plugins to use, and supports auto-reload on file changes.

This page documents configuration, data formats, usage, and integration patterns.

## Purpose

- Maintain a shared IP/CIDR dataset (such as blocklists, private-net lists, CDN prefixes).
- Provide fast membership checks for other plugins (such as filtering, routing, or ipset materialization).

## Key behaviors

- Loads CIDRs and single IPs (converted to /32 or /128) from files or inline `ips` entries.
- Stores compiled `IpNet` networks in shared state (`Arc<RwLock<Vec<IpNet>>>`).
- When executed, the plugin sets metadata `ip_set_<name>` containing the shared networks for downstream plugins.
- Supports `auto_reload` to watch files and atomically replace the dataset on change.

## Supported data formats

- Files: plain text, one entry per line. Lines starting with `#` are comments.
  - Each line may be a CIDR (`192.0.2.0/24`, `2001:db8::/32`) or a single IP (`1.2.3.4`, `2001:db8::1`).
- Inline `ips`: a string or sequence of strings using the same formats as files.

Invalid lines are skipped and logged at debug level.

## Configuration

Top-level plugin arguments (YAML) supported by `ip_set`:

- `files` (string or sequence): paths to one or more files containing IP/CIDR entries.
- `ips` (string or sequence): inline IP/CIDR entries.
- `auto_reload` (bool): enable file-watcher-based live reloads (default: false).
- `tag` / plugin name: used as the dataset name and for the metadata key; if absent the plugin effective name is used.

Example configuration (file-backed):

```yaml
plugins:
  - tag: local-ips
    type: ip_set
    config:
      files:
        - examples/etc/china-ip-list.txt
      auto_reload: true
```

Example configuration (inline):

```yaml
plugins:
  - tag: test-ips
    type: ip_set
    config:
      ips:
        - 1.1.1.1
        - 192.168.0.0/16
        - 2001:db8::/32
```

## Usage & integration

- When `execute()` runs, the plugin writes an `Arc<RwLock<Vec<IpNet>>>` into request metadata under the key `ip_set_<name>` (for example `ip_set_local-ips`).
- Other plugins can read this metadata and perform membership checks efficiently.
- The plugin also implements the `Matcher` trait: `matches_context(ctx)` returns `true` if any IP in the response answers belongs to the dataset.

## Auto-reload behavior

- When `auto_reload` is enabled a file watcher invokes a reload callback on changes (with a debounce).
- Reload replaces the in-memory `networks` atomically and logs counts and timing information.

## Diagnostics & stats

- The plugin logs the number of networks loaded and the number of source files processed.
- Use `plugin.stats()` (exposed via the plugin's API) to get counts for programmatic inspection.

## Troubleshooting

- If expected IPs do not match, verify input files contain valid CIDR or IP entries and that the plugin successfully loaded them (check logs).
- Ensure file read permissions are correct for the process when using `auto_reload`.

## Best practices

- Prefer CIDR entries for broad ranges and single IPs only where necessary.
- Keep file sizes reasonable or split large datasets to reduce reload impact.
- Combine `ip_set` with executor plugins (such as `ipset`) or custom plugins that read `ip_set_<name>` metadata to materialize or act on matched IPs.

## Example pipeline

```
[Downloader plugin] -> updates files
        |
   [ip_set plugin] -> loads networks, sets metadata
        |
   [ipset plugin] -> reads metadata / response and materializes sets
```

## See also

- [Domain Set plugin](05_07_01_PLUGIN_DOMAIN_SET.md) for domain name datasets
