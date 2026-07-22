# Domain Validator Plugin

The `domain_validator` plugin validates DNS query domain names for RFC compliance and filters invalid or malicious queries. It helps protect against DNS abuse, malformed queries, and can block specific domains using a blacklist.

## Key features

- RFC-compliant domain name validation
- Configurable strict/lenient mode
- Domain blacklist with wildcard support
- LRU cache for validation results to improve performance
- Prometheus metrics support (when enabled)
- Default priority: **2100** (runs very early to filter invalid queries)

## Validation rules

The plugin validates domain names according to DNS standards:

### Basic validation
- Domain length must not exceed 253 characters
- Each label must not exceed 63 characters
- Labels must start and end with alphanumeric characters (a-z, A-Z, 0-9)
- Middle characters can be alphanumeric or hyphens (-)

### Strict mode (default)
- Rejects domains with consecutive hyphens (`--`)
- More stringent format checks

### Lenient mode
- Allows consecutive hyphens (for IDN domains like `xn--example`)
- More permissive validation

## Blacklist matching

The blacklist supports three matching modes:

1. **Exact match**: `"example.com"` blocks only `example.com`
2. **Suffix match**: `"example.com"` blocks `example.com`, `sub.example.com`, `deep.sub.example.com`, etc.
3. **Wildcard match**: `"*.blocked.org"` blocks `sub.blocked.org`, `any.blocked.org`, etc., but not `blocked.org` itself

## Behavior details

- When an invalid domain is detected, the plugin returns a `REFUSED` response
- Validation results are cached in an LRU cache to reduce CPU overhead
- Blacklisted domains are logged with `WARN` level
- Invalid domains are logged with `DEBUG` level
- The plugin sets a response and terminates the pipeline for rejected queries

## Audit Integration

When the `web` feature is enabled, this plugin triggers the following security events:
- **`malformed_query`**: whenever a domain fails RFC validation rules.
- **`blocked_domain_query`**: whenever a domain matches the blacklist.

## Configuration options

- `strict_mode` (bool, default: `true`): enable strict RFC compliance mode
  - `true`: reject domains with consecutive hyphens
  - `false`: allow IDN-style domains (more permissive)
- `cache_size` (number, default: `1000`): maximum number of validation results to cache
  - Larger values reduce CPU usage for repeated queries
  - Set to `0` to disable caching (not recommended)
- `blacklist` (array of strings, default: `[]`): list of domains to block
  - Supports exact match, suffix match, and wildcard patterns
  - Case-insensitive matching

## Example configuration

### Basic usage (strict mode)

```yaml
plugins:
  - tag: validator
    type: domain_validator
    config:
      strict_mode: true
      cache_size: 2000
```

### With blacklist

```yaml
plugins:
  - tag: validator
    type: domain_validator
    config:
      strict_mode: true
      cache_size: 1000
      blacklist:
        - "malicious.com"          # Blocks malicious.com and *.malicious.com
        - "tracking.example.com"   # Blocks tracking.example.com and sub-domains
        - "*.ads.com"              # Blocks *.ads.com but not ads.com itself
        - "phishing-site.org"
```

### Lenient mode (for IDN support)

```yaml
plugins:
  - tag: validator
    type: domain_validator
    config:
      strict_mode: false  # Allow consecutive hyphens for punycode domains
      cache_size: 1000
```

## Typical pipeline placement

Place the `domain_validator` plugin **very early** in your pipeline (it has priority=2100 by default) to reject invalid queries before they reach expensive plugins like cache or forward:

```yaml
plugins:
  - type: domain_validator
    tag: validator
    config:
      strict_mode: true
      cache_size: 1000
      blacklist:
        - "malware.example.com"
        - "*.ads.example.org"
  
  - type: cache
    tag: main_cache
    config:
      size: 2048
  
  - type: forward
    tag: upstream
    config:
      upstreams:
        - "8.8.8.8:53"
```

## Metrics (when enabled)

When the `metrics` feature is enabled, the plugin exposes Prometheus metrics:

- `dns_domain_validation_total{result}`: total validation attempts by result type
  - `result` labels: `valid`, `invalid_chars`, `invalid_length`, `invalid_format`, `blacklisted`
- `dns_domain_validation_cache_hits_total`: number of cache hits
- `dns_domain_validation_duration_seconds`: histogram of validation duration

## Use cases

### 1. Security filtering
Block known malicious domains and prevent DNS tunneling attacks by rejecting malformed names.

### 2. Compliance enforcement
Ensure all queries comply with DNS RFC standards before forwarding to upstream resolvers.

### 3. Ad/tracker blocking
Use the blacklist to block advertising and tracking domains without requiring external zone files.

### 4. Performance optimization
Cache validation results to reduce CPU overhead for frequently queried domains.

## Troubleshooting

### Legitimate domains are being rejected

**Symptom**: Valid domains like `xn--example-something` are rejected.

**Solution**: Set `strict_mode: false` to allow consecutive hyphens required for punycode/IDN domains.

```yaml
config:
  strict_mode: false  # Allow IDN domains
```

### High CPU usage

**Symptom**: CPU usage is high even with domain validation enabled.

**Solution**: Increase `cache_size` to cache more validation results:

```yaml
config:
  cache_size: 5000  # Increase from default 1000
```

### Blacklist not working

**Symptom**: Blacklisted domains are still being resolved.

**Solution**: 
1. Check that domain names are lowercase in the blacklist
2. Verify the validator plugin runs before forward plugins
3. Check logs for `WARN` messages about rejected domains
4. Remember that `*.example.com` doesn't block `example.com` itself (use both if needed)

```yaml
config:
  blacklist:
    - "ads.example.com"   # Blocks ads.example.com and subdomains
    - "*.ads.example.com" # Redundant with suffix match above
```

## Best practices

1. **Place early in pipeline**: The validator should run before cache and forward plugins to reject queries as early as possible
2. **Use appropriate mode**: Enable `strict_mode: true` for security-focused deployments, `false` for international domain support
3. **Size cache appropriately**: Set `cache_size` based on your query volume (1000-5000 is typical)
4. **Monitor metrics**: Track validation rejections to identify potential issues or attacks
5. **Keep blacklist manageable**: Large blacklists can impact memory; consider using dedicated blocklist plugins for extensive filtering

## Performance notes

- Validation is very fast: typically < 10μs per domain
- Cache hits are even faster: < 1μs
- Blacklist checking uses hash sets: O(n) for n patterns but very fast in practice
- The plugin uses async RwLock for cache access to minimize contention
- Default cache size (1000) is suitable for most deployments

## Security considerations

- The validator protects against DNS tunneling by rejecting malformed domains
- Blacklist can be used for quick blocking without DNS zone files
- Strict mode helps prevent exploitation of DNS parsing vulnerabilities
- Combined with rate limiting, provides defense against DNS abuse

## Differences from similar plugins

Unlike ACL or domain set plugins, `domain_validator` focuses on **structural validation** rather than policy-based filtering. It ensures queries are well-formed before they reach other plugins.

Use `domain_validator` for RFC compliance and basic security, and `acl`/`domain_set` for policy-based allow/deny rules.
