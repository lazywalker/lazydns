# IP Matching Rules Guide

## Overview

The IP Set plugin in lazydns supports IP address and CIDR network matching for IP-based filtering rules. This document describes the complete IP matching rule system.

## Quick Start

### Basic Examples

```yaml
# Load an IP list file
- name: ip_set
  tag: whitelist
  args:
    files:
      - whitelist-ips.txt

# With auto-reload for dynamic updates
- name: ip_set
  tag: blacklist
  args:
    files:
      - blacklist-ips.txt
    auto_reload: true
```

### Rule Formats in Files

```
# Comments start with #
# This is a comment

# Single IPv4 address
192.168.1.1

# IPv4 CIDR network
192.168.0.0/16
10.0.0.0/8
172.16.0.0/12

# Single IPv6 address
2001:db8::1

# IPv6 CIDR network
2001:db8::/32
fe80::/10

# Empty lines are ignored
```

## IP Address Formats

### Single IPv4 Addresses

```
192.168.1.1          # Single host
8.8.8.8              # Google Public DNS
1.1.1.1              # Cloudflare DNS
127.0.0.1            # Localhost
```

Single addresses are automatically converted to `/32` CIDR notation for internal matching.

### IPv4 CIDR Networks

```
192.168.0.0/16       # Class B private network (65,536 hosts)
10.0.0.0/8           # Class A private network (16,777,216 hosts)
172.16.0.0/12        # Class B private network (1,048,576 hosts)
8.8.8.0/24           # Subnet with 256 hosts
192.168.1.128/25     # Subnet with 128 hosts
```

### Single IPv6 Addresses

```
2001:db8::1          # Single host
::1                  # Localhost
fe80::1              # Link-local address
```

Single addresses are automatically converted to `/128` CIDR notation for internal matching.

### IPv6 CIDR Networks

```
2001:db8::/32        # Typical allocation (2^96 addresses)
fe80::/10            # Link-local addresses
ff00::/8             # Multicast addresses
::/0                 # Entire IPv6 address space (rarely used)
2001:db8:1234::/48   # Subnet allocation
```

## Configuration Examples

### Basic Configuration

```yaml
- name: ip_set
  tag: whitelist
  args:
    files:
      - whitelist-ips.txt
```

Loads all IP addresses and CIDR ranges from the file.

### Multiple Files

```yaml
- name: ip_set
  tag: combined
  args:
    files:
      - trusted-networks.txt
      - partner-ips.txt
      - cdn-networks.txt
    auto_reload: true
```

Combines rules from multiple files with automatic reloading.

### Inline IP Addresses (ips Parameter)

You can specify IP rules inline using the `ips` parameter instead of external files:

**Single IP (string format):**
```yaml
- name: ip_set
  tag: local
  args:
    ips: "192.168.1.0/24"
```

**Multiple IPs (array format):**
```yaml
- name: ip_set
  tag: trusted
  args:
    ips:
      - "192.168.1.0/24"
      - "10.0.0.0/8"
      - "2001:db8::/32"
      - "127.0.0.1"
```

**Mixed files and inline addresses:**
```yaml
- name: ip_set
  tag: comprehensive
  args:
    files:
      - network-ranges.txt
    ips:
      - "192.168.0.0/16"
      - "10.0.0.0/8"
      - "172.16.0.0/12"
    auto_reload: true
```

The `ips` parameter supports:
- Single IPv4/IPv6 addresses (automatically converted to /32 or /128)
- CIDR notation (such as `192.168.0.0/24`)
- Mixed IPv4 and IPv6 in the same list
- All inline rules are processed after file rules

### Recommended File Format

```
# Private networks
10.0.0.0/8
172.16.0.0/12
192.168.0.0/16

# Link-local and loopback
127.0.0.1
::1

# Data center and cloud providers
13.107.0.0/16        # Microsoft
34.64.0.0/10         # Google Cloud
18.0.0.0/7           # AWS

# Specific services
8.8.8.8              # Google DNS
1.1.1.1              # Cloudflare DNS
8.8.4.4              # Google DNS secondary

# IPv6 examples
2001:4860:4860::8888 # Google Public DNS
2606:4700:4700::1111 # Cloudflare DNS
fe80::/10            # Link-local
```

## Matching Behavior

### Exact Network Matching

IP addresses are matched against CIDR networks using network containment logic:

**Rule:** `192.168.0.0/24`

```
192.168.0.0     : ✓ Match (network address)
192.168.0.1     : ✓ Match
192.168.0.128   : ✓ Match
192.168.0.255   : ✓ Match (broadcast address)
192.168.1.0     : ✗ No match (different network)
192.167.255.255 : ✗ No match
```

**Rule:** `10.0.0.0/8` (16,777,216 addresses)

```
10.0.0.0        : ✓ Match
10.1.2.3        : ✓ Match
10.255.255.255  : ✓ Match
11.0.0.0        : ✗ No match
```

### Single Address Rules

Single addresses are treated as `/32` (IPv4) or `/128` (IPv6) networks:

**Rule:** `192.168.1.100` (stored internally as `192.168.1.100/32`)

```
192.168.1.100   : ✓ Match (exact)
192.168.1.101   : ✗ No match
192.168.1.99    : ✗ No match
```

### IPv4 and IPv6 Coexistence

Both IPv4 and IPv6 rules can coexist in the same IP Set:

```yaml
- name: ip_set
  tag: dual_stack
  args:
    files:
      - dual-stack-networks.txt
```

**File contents:**
```
# IPv4 private ranges
10.0.0.0/8
172.16.0.0/12
192.168.0.0/16

# IPv6 private ranges
fd00::/7
fe80::/10

# Single addresses
8.8.8.8
2001:4860:4860::8888
```

The plugin automatically detects and handles both address families.

## Performance Characteristics

### Time Complexity

```
Network lookup: O(n)
Where n = number of CIDR rules

Matching algorithm: Linear scan of all networks
No optimization for sorted/indexed lookups
```

**Typical query times:**
- 100 networks: < 1 µs
- 1,000 networks: < 10 µs
- 10,000 networks: < 100 µs

### Space Complexity

```
IPv4 networks: ~12 bytes each
IPv6 networks: ~20 bytes each
Single IPs stored as /32 or /128
```

**Memory per 10,000 networks:**
- Pure IPv4: ~120 KB
- Pure IPv6: ~200 KB
- Mixed IPv4/IPv6: ~150-180 KB

## Best Practices

### Design Considerations

1. **Use CIDR notation for ranges** instead of listing individual IPs
   ```yaml
   # Good: efficient
   - name: ip_set
     args:
       ips:
         - "192.168.0.0/24"

   # Bad: wasteful
   - name: ip_set
     args:
       ips:
         - "192.168.0.1"
         - "192.168.0.2"
         - "192.168.0.3"
         # ... 250+ more entries
   ```

2. **Organize by scope:** public, private, datacenter, CDN
   ```yaml
   - name: ip_set
     tag: private
     args:
       ips:
         - "10.0.0.0/8"
         - "172.16.0.0/12"
         - "192.168.0.0/16"

   - name: ip_set
     tag: public_cdn
     args:
       ips:
         - "8.8.8.0/24"
         - "1.1.1.0/24"
   ```

3. **Use files for large lists** (>1000 entries)
   ```yaml
   - name: ip_set
     tag: large_list
     args:
       files:
         - cdn-ip-ranges.txt
       auto_reload: true
   ```

4. **Inline for small, frequently changed rules**
   ```yaml
   - name: ip_set
     tag: exceptions
     args:
       ips:
         - "203.0.113.45"  # Current exception
   ```

### Documentation

- Comment CIDR ranges with their purpose
- Document why specific networks are whitelisted/blacklisted
- Keep separate files for different categories

```
# Example file structure
trusted-networks.txt:
  - 10.0.0.0/8          # Internal corporate
  - 192.168.0.0/16      # Office locations

partner-ips.txt:
  - 203.0.113.0/24      # Partner A
  - 198.51.100.0/24     # Partner B
```

## Troubleshooting

### IP Not Matching

**Problem:** Expected IP to match but it doesn't

**Common causes:**
1. IP notation error (missing CIDR prefix)
   ```yaml
   # Wrong: missing /32
   ips:
     - "192.168.1.100"  # Actually works - auto-converted to /32
   
   # Correct: explicit CIDR
   ips:
     - "192.168.1.100/32"
   ```

2. Wrong address family
   ```yaml
   # IPv4 rule won't match IPv6
   ips:
     - "192.168.1.0/24"  # Only matches IPv4
   
   # Need both for dual-stack
   ips:
     - "192.168.1.0/24"
     - "2001:db8::/32"
   ```

3. Network contains check is exclusive of wrong boundaries
   - `10.0.0.0/24` matches `10.0.0.1` through `10.0.0.254`
   - Broadcast address `10.0.0.255` is technically in the network but not always matched depending on kernel

### File Load Errors

**Problem:** Rules not loading from file

**Check:**
1. File path is correct and readable
2. File contains valid IP addresses or CIDR notation
3. Enable debug logging to see parsing errors
4. Invalid entries are skipped with debug log messages

### Performance Issues

**Problem:** DNS queries are slow with IP Set plugin

**Solutions:**
1. Reduce number of rules (1000+ rules: ~100µs per query)
2. Use CIDR notation instead of individual IPs
3. Order most-frequently-matched rules first (hint: no optimization exists currently)
4. Consider splitting into multiple IP Sets by category

## API Reference

### Plugin Configuration

```yaml
- name: ip_set
  tag: <unique-identifier>
  args:
    files:                    # Optional: list of files to load
      - path/to/file1.txt
      - path/to/file2.txt
    ips:                      # Optional: inline IP addresses/networks
      - "192.168.0.0/24"
      - "10.0.0.0/8"
      - "127.0.0.1"
      - "::1"
    auto_reload: true         # Optional: auto-reload files (default: false)
    # auto_reload interval: ~200ms (not configurable)
```

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `tag` | string | Yes | - | Unique plugin identifier |
| `files` | array | No | empty | File paths containing IP rules |
| `ips` | array/string | No | empty | Inline IP addresses or networks |
| `auto_reload` | bool | No | false | Auto-reload files on change |

### Return Value

All IP Set plugin operations return a boolean:

```
true  : IP is in the matched set
false : IP is not in the set
```

## Examples

### Whitelist Model

Allow only specific networks:

```yaml
- name: ip_set
  tag: whitelist
  args:
    ips:
      - "10.0.0.0/8"           # Internal
      - "203.0.113.0/24"       # Partner
```

### Blacklist Model

Deny specific networks:

```yaml
- name: ip_set
  tag: blacklist
  args:
    files:
      - blocked-ranges.txt
    ips:
      - "192.0.2.0/24"         # Malicious actor
```

### CDN Detection

Match against CDN provider networks:

```yaml
- name: ip_set
  tag: cdn_networks
  args:
    files:
      - cdn-ip-ranges.txt      # Updated regularly
    auto_reload: true
```

### Geolocation-based Filtering

Combined with other plugins to filter by region (basic approach):

```yaml
- name: ip_set
  tag: china_ips
  args:
    files:
      - china-ip-ranges.txt
    auto_reload: true
```

### Multi-region Whitelist

Allow traffic from specific regions only:

```yaml
- name: ip_set
  tag: allowed_regions
  args:
    ips:
      - "198.51.100.0/24"      # US region
      - "203.0.113.0/24"       # EU region
      - "192.0.2.0/24"         # APAC region
```

## Related Documentation

- [Domain Matching Rules](DOMAIN_MATCHING_RULES.md)
- [Plugin System Architecture](IMPLEMENTATION.md)

