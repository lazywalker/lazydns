# Domain Matching Rules Guide

## Overview

The Domain Set plugin in lazydns supports sophisticated domain name matching with multiple rule types and priority-based evaluation. This document describes the complete domain matching rule system.

## Quick Start

### Basic Examples

```yaml
# Load a domain list file with default (domain) matching
- name: domain_set
  tag: direct
  args:
    files:
      - direct-list.txt

# With specific match type
- name: domain_set
  tag: gfw
  args:
    files:
      - gfw-list.txt
    default_match_type: domain
    auto_reload: true
```

### Rule Formats in Files

```
# Comments start with #
# This is a comment

# Exact match only
full:google.com

# Domain match (default)
domain:example.com
example.com              # No prefix = uses default_match_type

# Keyword substring match
keyword:facebook

# Regular expression match
regexp:.*\.google\.com$

# Empty lines are ignored

```

## Match Types

### 1. Full Match (`full:`)

**Exact domain matching only, no subdomains.**

- Syntax: `full:example.com`
- Matches: `example.com`, `EXAMPLE.COM` (case-insensitive)
- Does NOT match: `www.example.com`, `sub.example.com`, `example.com.hk`
- Performance: **O(1)** - constant time lookup
- Use case: Block specific exact domains, whitelist specific services

#### Examples

```
full:google.com        : matches only "google.com"
full:api.github.com    : matches only "api.github.com"
                       : does NOT match "github.com" or "www.api.github.com"
```

### 2. Domain Match (`domain:`)

**Match domain and all its subdomains.**

- Syntax: `domain:example.com` or just `example.com` (uses default)
- Matches: `example.com`, `www.example.com`, `api.example.com`, `a.b.c.example.com`
- Does NOT match: `notexample.com`, `example.com.hk`, `examplecom`
- Performance: **O(levels)** - logarithmic in domain depth
- Use case: Block entire domain hierarchies (most common use)

#### Subdomain Priority

When multiple domain rules could match, the most specific (longest) rule wins:

```
Rules: com, example.com, api.example.com

Query www.example.com:
  ✓ Matches api.example.com? No
  ✓ Matches example.com? Yes (return true)
  ✗ Would also match com, but already found more specific match

Query api.example.com:
  ✓ Matches api.example.com? Yes (return true)

Query other.com:
  ✓ Matches api.example.com? No
  ✓ Matches example.com? No
  ✓ Matches com? Yes (return true)
```

#### Examples

```
domain:google.com      : matches google.com, www.google.com, maps.google.com, etc.
domain:co.uk           : matches all .co.uk domains
example.com            : equivalent to domain:example.com (if default is domain)
```

### 3. Keyword Match (`keyword:`)

**Substring/keyword matching anywhere in the domain.**

- Syntax: `keyword:google`
- Matches: `google.com`, `www.google.com`, `google.com.hk`, `mygoogle.net`, `my-google-service.org`
- Does NOT match: `gogle.com` (typo), `notgooglelike.com` (keyword not present as substring)
- Performance: **O(n)** - linear traversal
- Evaluation order: Import order (first match wins)
- Use case: Catch variations and domain names containing keywords, less precise
- Warning: Can produce false positives (for example, `keyword:ad` matches `add.com`, `advertisement.com`, `badword.com`)

#### Examples

```
keyword:facebook       : matches facebook.com, www.facebook.com, facebook.com.cn, myfacebook.net, etc.
keyword:google         : matches google.com, mygoogle.com, google.com.hk, googlechrome.com, etc.
keyword:cdn            : matches cdn.com, mycdn.net, ocdn.org, etc. (be careful!)
```

### 4. Regexp Match (`regexp:`)

**Regular expression pattern matching using Rust regex syntax (compatible with Go stdlib).**

- Syntax: `regexp:^[a-z]+\.google\.com$`
- Pattern: Standard Rust regex syntax
- Performance: **O(n·regex_complexity)** - can be CPU-intensive with complex patterns
- Evaluation order: Import order (first match wins)
- Use case: Complex pattern matching, flexible rules

#### Regex Basics

| Pattern | Matches | Does NOT match |
|---------|---------|----------------|
| `.+\.google\.com$` | `www.google.com`, `maps.google.com` | `google.com` (no prefix) |
| `^google\.` | `google.com`, `google.co.uk` | `www.google.com` |
| `(baidu\|google)` | `baidu.com`, `google.com` | `notbaidu.com` |
| `test-[0-9]+` | `test-123.com`, `test-1.org` | `test-abc.com` |

#### Examples

```
regexp:.+\.github\.io$         : matches *.github.io (personal GitHub Pages)
regexp:^api\.                  : matches api.example.com, api.service.com, etc.
regexp:(qq\|wechat)            : matches qq.com, wechat.com
regexp:.*cdn.*                 : matches any domain containing "cdn"
```

#### Performance Warning

Regexp matching is CPU-intensive, especially with:
- Complex backtracking patterns
- Overlapping quantifiers (`.*.*`, `.+.+`)
- Large number of rules

**Best practices:**
- Use simpler alternatives when possible (full/domain match)
- Avoid complex patterns with many regexp rules
- Order rules by likelihood of match (common patterns first)
- Use anchors `^` and `$` to improve performance

## Matching Priority

Rules are evaluated in strict priority order. The first matching rule determines the result.

### Priority Order (Highest to Lowest)

```
Full > Domain > Regexp > Keyword
```

#### Example

```yaml
Rules:
  - full:example.com
  - domain:example.com
  - keyword:example
  - regexp:.*example.*
```

Query `example.com`:
```
1. Check Full rules: matches full:example.com ✓ (RETURN TRUE)
   (Never reaches Domain, Regexp, or Keyword checks)
```

Query `sub.example.com`:
```
1. Check Full rules: no match
2. Check Domain rules: matches domain:example.com ✓ (RETURN TRUE)
   (Never reaches Regexp or Keyword checks)
```

Query `myexample.org`:
```
1. Check Full rules: no match
2. Check Domain rules: no match
3. Check Regexp rules: matches .*example.* ✓ (RETURN TRUE)
   (Never reaches Keyword check)
```

## Performance Characteristics

### Time Complexity

| Match Type | Complexity | Notes |
|------------|-----------|-------|
| Full | O(1) | HashMap lookup |
| Domain | O(d) | d = domain depth, typically 3-4 |
| Regexp | O(n·r) | n = rules, r = regex complexity |
| Keyword | O(n·s) | n = rules, s = string length |

### Space Complexity (Approximate)

| Match Type | Memory per 10,000 rules |
|------------|----------------------|
| Full | ~1 MB |
| Domain | ~1 MB |
| Regexp | ~2-5 MB (includes compiled regex) |
| Keyword | ~0.5-1 MB |

### Benchmarks

```
Rule set size: 100,000 domains

Match type    | Avg Query Time | Remarks
-----------------------------------
Full          | < 1 µs         | Instant
Domain        | < 5 µs         | Very fast
Regexp        | 100-1000 µs    | Slow with complex patterns
Keyword       | 50-500 µs      | Linear scan
```

## Rule Evaluation Order

### Full and Domain Rules

When **multiple** full or domain rules could match, the most specific match wins:

```
Rules:
  - domain:com
  - domain:example.com
  - domain:api.example.com

Query api.example.com:
  Evaluation: Longest match wins
  Matches api.example.com (most specific) ✓
```

### Regexp and Keyword Rules

Rules are evaluated in **import order** (file order). The **first match** wins:

```
Rules (in order):
  - regexp:google
  - regexp:.*oogle
  - keyword:abc

Query "google.com":
  Matches first regexp:google ✓ (returns true immediately)
  Never evaluates remaining rules
```

## Configuration Examples

### Basic Configuration

```yaml
- name: domain_set
  tag: direct
  args:
    files:
      - direct-list.txt
```

With default domain matching (rules without prefix use domain match).

### With Custom Default Match Type

```yaml
- name: domain_set
  tag: gfw
  args:
    files:
      - gfw.txt
    default_match_type: keyword
    auto_reload: true
```

All rules without a prefix will use keyword matching.

### Multiple Files with Auto-Reload

```yaml
- name: domain_set
  tag: combined
  args:
    files:
      - blocklist.txt
      - custom-domains.txt
      - regex-patterns.txt
    default_match_type: domain
    auto_reload: true
    # Auto-reload checks files every ~200ms
```

### Inline Domain Expressions (exps Parameter)

You can also specify domain rules inline using the `exps` parameter instead of external files:

**Single rule (string format):**
```yaml
- name: domain_set
  tag: direct
  args:
    exps: "example.com"
```

**Multiple rules (array format):**
```yaml
- name: domain_set
  tag: combined
  args:
    exps:
      - "example.com"
      - "full:github.com"
      - "regexp:.+\.google\.com$"
      - "keyword:facebook"
```

**Mixed files and inline expressions:**
```yaml
- name: domain_set
  tag: comprehensive
  args:
    files:
      - blocklist.txt
    exps:
      - "example.com"
      - "full:special.service.com"
      - "regexp:^internal-.*\.local$"
    default_match_type: domain
    auto_reload: true
```

The `exps` parameter supports the same rule format as files:
- Prefix with `full:`, `domain:`, `keyword:`, or `regexp:` for specific match types
- Rules without prefix use the `default_match_type`
- All inline rules are processed after file rules

### Recommended File Format

```
# Direct access (fast, no censorship)
# Domain format (matches subdomains)
domain:example.com
github.com
www.wikipedia.org

# Exact matches for specific services
full:dns.google

# Keywords for broad categories
keyword:cdn

# Complex patterns
regexp:.+\.local$
regexp:^192-168-.*\.nip\.io$
```

## File Format

### Supported Formats

1. **Domain (default)**
   ```
   example.com
   sub.example.com
   ```

2. **With Prefix**
   ```
   full:exact.com
   domain:parent.com
   keyword:google
   regexp:.+\.example\.com$
   ```

3. **Comments**
   ```
   # This is a comment
   # Comments must be on their own line
   example.com
   ```

4. **Whitespace**
   ```
   Leading and trailing whitespace is trimmed
     example.com    :  matches "example.com"
   ```

5. **Empty Lines**
   ```
   Empty lines are silently ignored
   ```

### Example File

```
# Direct access domains (no blocking)
# Updated: 2024-12-26

# GitHub and services
github.com
www.github.io
api.github.com

# Exact services
full:dns.google.com
full:8.8.8.8

# CDN and infrastructure
keyword:cdn
keyword:cloudflare

# User agent patterns
regexp:.*bot.*
regexp:.*crawler.*

# Personal domains
domain:*.example.com
```

## Best Practices

### 1. Rule Organization

```
1. Full matches (most specific, fastest)
2. Domain matches (common case)
3. Regexp patterns (complex logic)
4. Keyword matches (broad patterns)
```

### 2. Performance Optimization

- Use Full/Domain matches for known domains (O(1))
- Place frequently matched rules early in Regexp/Keyword sections
- Avoid excessive Regexp rules with complex backtracking
- Don't use Keyword matches for everything (use Domain instead)

### 3. Accuracy vs Coverage

- **High accuracy**: Use `full:` and specific `domain:` rules
- **Coverage**: Use `keyword:` and `regexp:` patterns
- **Balance**: Mix all types appropriately for your use case

### 4. Maintenance

- Organize rules by category with comments
- Use `auto_reload: true` for frequently updated lists
- Test rule changes (benchmark and functional tests)
- Document complex regexp patterns

## Troubleshooting

### Rule Not Matching

1. Check case sensitivity (all rules are case-insensitive)
2. Verify prefix is correct (`full:`, `domain:`)
3. Check trailing dots (automatically normalized)
4. Verify rule priority (check previous rules)

Example:
```
# These all match "example.com":
domain:example.com
DOMAIN:EXAMPLE.COM
example.com.              # trailing dot normalized
Example.Com              # case normalized

# These do NOT match "example.com":
full:www.example.com     # full requires exact match
keyword:exam             # keyword is substring, would match but rule is for "exam"
```

### Performance Issues

1. **Slow query matching**: Check Regexp/Keyword rule count
2. **Large memory usage**: Consider splitting into multiple file sets
3. **File reload delays**: Auto-reload debounce is 200ms (configurable)

### Debugging

Enable tracing to see matching details:
```
RUST_LOG=debug lazydns
```

## API Reference

### Rust Code

```rust
use lazydns::plugins::dataset::{DomainRules, MatchType};

let mut rules = DomainRules::new();

// Add rules
rules.add_rule(MatchType::Full, "exact.com");
rules.add_rule(MatchType::Domain, "example.com");
rules.add_rule(MatchType::Keyword, "google");
rules.add_rule(MatchType::Regexp, r".+\.github\.io$");

// Parse lines
rules.add_line("domain:test.com", MatchType::Domain);

// Check matches
assert!(rules.matches("exact.com"));
assert!(rules.matches("sub.example.com"));
assert!(rules.matches("test-google.com"));
assert!(rules.matches("mysite.github.io"));
```

### Statistics

```rust
let stats = rules.stats();
println!("Full: {}", stats.full_count);
println!("Domain: {}", stats.domain_count);
println!("Regexp: {}", stats.regexp_count);
println!("Keyword: {}", stats.keyword_count);
```

## See Also

- [IP Matching Rules](IP_MATCHING_RULES.md)
- [Plugins Overview](IMPLEMENTATION.md)
- [Configuration Guide](UPSTREAM_FEATURES.md)
