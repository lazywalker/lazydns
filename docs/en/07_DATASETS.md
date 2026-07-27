# Datasets & Formats

This page summarizes common dataset types supported by lazydns, example file formats, and tips
for creating and maintaining dataset files. For detailed rule syntax and matching priority,
see [DOMAIN_MATCHING_RULES.md](docs/DOMAIN_MATCHING_RULES.md).

**Domain sets**

- Purpose: provide a reusable list of domain matching rules that other plugins can query.
- Supported match types: `full`, `domain`, `regexp`, `keyword` (priority: full > domain > regexp > keyword).
- Line format and examples (one rule per line):

```text
# comments and blank lines are ignored
full:example.com           # exact match only
domain:example.com         # match example.com and all subdomains
keyword:google             # substring/keyword match
regexp:^.+\.google\.com$ # regex pattern (Rust regex syntax)
example.org                # no prefix: uses the dataset's default match type (usually `domain`)
```

- Normalization: entries are trimmed and lowercased; trailing dots on queries are ignored at match time.
- Files may contain mixed prefixes; unprefixed lines use the configured `default_match_type`.
- Loading sources: domain sets can be loaded from one or more files (configured in the plugin
	`files` list) and from inline expressions (`exps`). See the `DomainSetPlugin` for options.
- Auto-reload: when `auto_reload` is enabled the plugin watches listed files and reloads them
	atomically on change. Use atomic replacements (write-to-temp + rename) for safe updates.

	**Memory & auto-reload:** after large dataset loads or reloads, lazydns will attempt to
	hint the system allocator to return freed pages to the OS (via `malloc_trim(3)`) on Linux
	with the GNU C library. This is a best-effort, platform-specific optimization and is a
	no-op on other platforms or allocator implementations.

Examples and recommendations:

- Place dataset examples under `examples/etc/` (such as `examples/etc/direct-list.txt`,
	`examples/etc/gfw.txt`).
- Prefer `domain:` for large domain lists that should match subdomains; use `full:` when only
	an exact match is intended.
- Avoid overly broad `keyword:` entries that may cause false positives.

**IP sets**

- Purpose: lists of IP addresses and CIDR blocks used for routing, blocking, or selecting upstreams.
- File format: plain text with one CIDR or IP per line. Comments starting with `#` are ignored.

```text
# single IP
192.0.2.5
# CIDR range
198.51.100.0/24
::1/128
```

- Matching: loader treats each non-comment line as an IP/CIDR entry. Some plugins/materializers
	support converting these lists into `ipset` or `nft` rules (see `ipset` and `nftset` plugins).
- Auto-reload: supported similarly to domain sets; replace files atomically to avoid partial loads.

**Geosite / GeoIP datasets**

- Geosite: lists of domains grouped by geography or categories (commonly used for routing decisions).
- GeoIP: IP-to-country databases (such as MaxMind or other-lite formats) used to determine country
	membership for IPs.
- Sources and format: geosite files are usually plain domain lists with optional grouping metadata;
	GeoIP files vary by source; the repository includes downloader helpers and example locations.
- Usage: many plugins accept geosite/geoip paths or rely on dataset plugins that expose membership
	queries. Use the `downloader` plugin to fetch and atomically update large third-party datasets.

**Creating custom datasets**

- File format: keep simple line-based formats (one item per line, `#` for comments). For domain rules
	support the same `prefix:value` styles shown above to keep behavior predictable.
- Parsers: implement a small loader that normalizes entries and supports the same prefixes. If you
	need specialized parsing (such as CSV with extra columns) provide a conversion step to the plain
	list format and let lazydns consume the normalized file.
- Integration tips:
	- Use atomic file replacement (write to a temp file then rename) when updating datasets.
	- Prefer smaller, focused files (one purpose per file) and combine them via the plugin `files`
		array when needed.
	- For large remote datasets, use `downloader` with `temp_file` + `on_success` rename semantics.

**Examples and locations**

- Example dataset files in this repo: [examples/etc](examples/etc). Use these as templates.
- Refer to `DomainSetPlugin` and the source in `src/plugins/dataset/domain_set.rs` for runtime
	behavior and functions (`add_line`, `add_rule`, matching priority, auto-reload).
