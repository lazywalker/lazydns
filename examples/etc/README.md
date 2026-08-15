# LazyDNS Example Configuration

This directory contains a complete working example of LazyDNS configuration based on the [lazymosdns](https://github.com/lazywalker/lazymosdns) setup.

## Features Demonstrated

- **DNS Caching**: Cache query results for faster responses
- **Multiple Upstreams**: Forward queries to both local (CN) and remote (international) DNS servers
- **Domain Lists**: Separate handling for CN domains, GFW domains, and ad domains
- **IP Lists**: China IP detection and whitelisting
- **Hosts File**: Custom DNS records with auto-reload support
- **Auto-Reload**: All data provider plugins (hosts, domain_set, ip_set) support automatic file reloading
- **Fallback**: Intelligent fallback between local and remote DNS servers
- **RouterOS Integration**: Add domains to RouterOS address lists 
(optional)
- **Rate Limiting and Validation**: Basic rate limiting and query validation

## Generated Data Files

`apple-cn.txt`, `china-ip-list.txt`, `direct-list.txt`, `hosts-github.txt`,
`proxy-list.txt` and `reject-list.txt` are fetched from upstream rule
repositories by the `update` script (or the downloader cron in `config.yaml`)
and are not tracked by git. On a fresh clone, run `./update` once (with
`WORKDIR` pointing at this directory) or let the cron downloader fetch them
before starting the server.

## Quick Start

### 1. Run the Example

From the repository root:

```bash
target/release/lazydns -c config.yaml -d examples/etc/
```

### 2. Test with dig

Test basic DNS resolution:

```bash
# Test localhost resolution (from hosts.txt)
dig @127.0.0.1 -p 5354 localhost

# Test a CN domain (should use local DNS)
dig @127.0.0.1 -p 5354 baidu.com

# Test an international domain (should use remote DNS)
dig @127.0.0.1 -p 5354 google.com

# Test AAAA (IPv6) query
dig @127.0.0.1 -p 5354 AAAA google.com
```

### 3. Test with nslookup

```bash
# Test basic resolution
nslookup -port=5354 localhost 127.0.0.1

# Test domain resolution
nslookup -port=5354 baidu.com 127.0.0.1
```

## Configuration Files

### Main Configuration

- **config.yaml**: Main configuration file defining all plugins and their execution flow

### Data Files

- **hosts.txt**: Custom hosts file for local DNS resolution
- **hosts-github.txt**: GitHub-related hosts entries
- **direct-list.txt**: List of domains that should use local (CN) DNS servers
- **apple-cn.txt**: Apple domains for CN region
- **my-domain-list.txt**: Custom domain list
- **proxy-list.txt**: Domains that should use remote DNS servers
- **proxy-ext-list.txt**: Extended GFW domain list
- **reject-list.txt**: Ad/tracking domains to block
- **china-ip-list.txt**: China IP address ranges (CIDR format)
- **white-ip-list.txt**: Whitelisted IP addresses


## License

This example configuration is provided as-is for demonstration purposes.
