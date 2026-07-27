# GeoIP & GeoSite Plugins

This page documents two related geo-based plugins:

- **`geoip`**: map IP addresses to ISO country codes and set request metadata
- **`geosite`**: map domain names to categories (countries/regions or custom categories) and set request metadata

Both plugins are useful for routing and policy decisions (such as sending China-hosted sites to China-based upstreams) and can be used together in a pipeline.

---

## GeoIP Plugin

**Purpose**: Determine the country of IP addresses found in DNS responses and attach a country code to the request metadata for downstream routing decisions.

**Default behavior**:

- Examines the answer section for A/AAAA records and looks up the IP against configured CIDR ranges.
- Stores the first matched ISO-3166 alpha-2 country code under a metadata key (default: `country`).
- Runs after a response is produced and before routing decisions (priority: **-20**).

**Configuration options**:

- `metadata_key` (string, optional): metadata key to set (default: `country`).
- `files` (sequence of strings, optional): list of files with lines of form `<cidr> <country_code>`.
- `data` (sequence of strings, optional): inline data strings, same format as files.

**Text data format** (used by `files` / `data`):

```
# comment lines are ignored
8.8.8.0/24 US
1.0.1.0/24 CN
2001:4860::/32 US
```

**Example configuration**:

```yaml
plugins:
  - tag: geoip
    type: geo_ip
    config:
      metadata_key: country
      files:
        - examples/geoip/geoip.txt
```

**Implementation notes**:

- The plugin provides programmatic helpers: `add_country_cidr`, `load_from_string`, `lookup`, `country_count`, and others.
- CIDR parsing uses `ipnet::IpNet` and both IPv4 and IPv6 are supported.
- On match the plugin sets metadata and returns immediately after the first match.

**Troubleshooting**:

- If you get no country metadata, ensure the response contains A/AAAA answers and that your CIDR dataset includes the IP ranges in question.
- Large datasets can be pre-generated and loaded via `files`.

---

## GeoSite Plugin

**Purpose**: Tag domain names with categories (commonly country codes like `cn`, `us`, or other custom categories) for routing decisions.

**Default behavior**:

- Examines the first question (A/AAAA only) and attempts to match the qname against configured domain sets.
- Supports exact matches and suffix/wildcard patterns (such as `*.qq.com` matching `mail.qq.com`).
- Stores the category under a metadata key (default: `category`).
- Runs early to tag requests (priority: **70**).

**Configuration options**:

- `metadata_key` (string, optional): metadata key to set (default: `category`).
- `files` (sequence of strings, optional): list of files with lines of form `<category> <domain>`.
- `data` (sequence of strings, optional): inline data strings, same format as files.

**Text data format** (used by `files` / `data`):

```
# category domain
cn baidu.com
cn *.qq.com
us google.com
us *.facebook.com
```

**Example configuration**:

```yaml
plugins:
  - tag: geosite
    type: geo_site
    config:
      metadata_key: site_category
      files:
        - examples/geosite/geosite.txt
```

**Domain matching details**:

- Exact matches: `example.com` matches `example.com`.
- Wildcard/suffix matches: `*.example.com` and `.example.com` will match `mail.example.com`, `deep.mail.example.com`, and `example.com` itself.
- Categories are arbitrary strings; you can use ISO country codes, feature flags (`ads`, `cdn`), or any grouping you need.

**Implementation notes**:

- The plugin exposes helper methods: `add_domain`, `load_from_string`, `lookup`, `category_count`, and `domain_count`.
- Internally, domain sets keep both exact strings and suffix patterns (lowercased) for efficient checks.

**Troubleshooting**:

- If domains are not tagged, verify the plugin's data files and ensure your qname is in the expected form (lowercase matching is performed by the plugin).
- For large domain sets, consider pre-filtering or splitting categories to keep memory usage acceptable.

---

## Using GeoIP and GeoSite Together

A common pattern is to run both plugins and let later routing plugins decide based on metadata keys produced by them. For example, you might set up forwarding rules that select upstreams by `country` or `site_category` metadata.

**Example pipeline**:

```
[Listener]
  -> [plugins: hosts]
  -> [plugins: cache]
  -> [plugins: geo_site]   # tag domain category -> metadata `category`
  -> [plugins: forward]    # forward rule uses `category` or `country` metadata
  -> [plugins: geoip]      # tag answer IPs if needed -> metadata `country`
```

Note: order matters; `geo_site` runs early to tag by domain, and `geoip` runs after answers are present to tag by IP.

---

## Best Practices

- Keep GeoSite and GeoIP data files in your `examples/` or `etc/` directory and reference them from configuration.
- Use categories and country codes consistently across forwarding and routing rules.
- Validate small test files first and add larger datasets incrementally.

