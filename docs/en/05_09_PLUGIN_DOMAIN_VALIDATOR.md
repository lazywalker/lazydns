# Domain Validator Plugin

The `domain_validator` plugin validates DNS query domain names for RFC 1035/1123 compliance and rejects malformed queries early, reducing upstream load and improving robustness.

## Key features

- RFC-compliant domain name validation
- Configurable strict/lenient mode
- LRU cache for validation results to improve performance
- Prometheus metrics support (when enabled)

## Validation rules

The plugin validates domain names according to DNS standards:

### Basic validation
- Domain length must not exceed 253 characters
- Each label must not exceed 63 characters
- Labels must start and end with alphanumeric characters (a-z, A-Z, 0-9)
- Middle characters can be alphanumeric or hyphens (-)

### Strict mode (default)
- Rejects domains with consecutive hyphens (`--`), except for Punycode A-labels (`xn--...`)

### Lenient mode
- Allows consecutive hyphens (more permissive)

## Behavior details

- When an invalid domain is detected, the plugin returns a `REFUSED` response
- Validation results are cached in an LRU cache to reduce CPU overhead
- Invalid domains are logged with `DEBUG` level
- The plugin sets a response and terminates the pipeline for rejected queries

## Audit Integration

When the `web` feature is enabled, this plugin triggers the following security events:
- **`malformed_query`**: whenever a domain fails RFC validation rules.

## Configuration options

- `strict_mode` (bool, default: `true`): enable strict RFC compliance mode
  - `true`: reject domains with consecutive hyphens (except Punycode)
  - `false`: allow consecutive hyphens (more permissive)
- `cache_size` (number, default: `1000`): maximum number of validation results to cache
  - Larger values reduce CPU usage for repeated queries
  - Set to `0` to disable caching (not recommended)

## Example configuration

### Basic usage (strict mode)

```yaml
plugins:
  - tag: validator
    type: domain_validator
    args:
      strict_mode: true
      cache_size: 2000
```

### Lenient mode (for IDN support)

```yaml
plugins:
  - tag: validator
    type: domain_validator
    args:
      strict_mode: false
      cache_size: 1000
```

## Typical pipeline placement

Place the `domain_validator` plugin **very early** in your pipeline to reject invalid queries before they reach expensive plugins like cache or forward:

```yaml
plugins:
  - type: domain_validator
    tag: validator
    args:
      strict_mode: true
      cache_size: 1000

  - type: cache
    tag: main_cache
    args:
      size: 2048

  - type: forward
    tag: upstream
    args:
      upstreams:
        - addr: "8.8.8.8:53"
```

## Metrics (when enabled)

When the `metrics` feature is enabled, the plugin exposes Prometheus metrics:

- `dns_domain_validation_total{result}`: total validation attempts by result type
  - `result` labels: `valid`, `invalid_chars`, `invalid_length`, `invalid_format`
- `dns_domain_validation_cache_hits_total`: number of cache hits
- `dns_domain_validation_duration_seconds`: histogram of validation duration

## Use cases

### 1. Compliance enforcement
Ensure all queries comply with DNS RFC standards before forwarding to upstream resolvers.

### 2. Performance optimization
Cache validation results to reduce CPU overhead for frequently queried domains.

### 3. Security filtering
Prevent DNS tunneling attacks by rejecting malformed names early.

## Troubleshooting

### Legitimate domains are being rejected

**Symptom**: Valid domains like `xn--example-something` are rejected.

**Solution**: Punycode A-labels (`xn--...`) are allowed even in strict mode. If other domains with consecutive hyphens are legitimate, set `strict_mode: false`:

```yaml
args:
  strict_mode: false
```

### High CPU usage

**Symptom**: CPU usage is high even with domain validation enabled.

**Solution**: Increase `cache_size` to cache more validation results:

```yaml
args:
  cache_size: 5000
```

## Best practices

1. **Place early in pipeline**: The validator should run before cache and forward plugins to reject queries as early as possible
2. **Use appropriate mode**: Enable `strict_mode: true` for security-focused deployments, `false` for international domain support
3. **Size cache appropriately**: Set `cache_size` based on your query volume (1000-5000 is typical)
4. **Monitor metrics**: Track validation rejections to identify potential issues or attacks

## Performance notes

- Validation is very fast: typically < 10μs per domain
- Cache hits are even faster: < 1μs
- The plugin uses async RwLock for cache access to minimize contention
- Default cache size (1000) is suitable for most deployments

## Differences from similar plugins

Unlike `domain_set` or `acl` plugins, `domain_validator` focuses on **structural RFC validation** rather than policy-based filtering. It ensures queries are well-formed before they reach other plugins.

Use `domain_validator` for RFC compliance, and `domain_set` + `black_hole` for domain blocking/blocklisting.
