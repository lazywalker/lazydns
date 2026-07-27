# LazyDNS

<!-- Repository badges -->

[![Rust](https://img.shields.io/badge/rust-2024--edition-orange)](https://www.rust-lang.org/)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](https://opensource.org/licenses/GPL-3.0)
[![CI](https://github.com/lazywalker/lazydns/actions/workflows/ci.yml/badge.svg)](https://github.com/lazywalker/lazydns/actions)
[![crates.io](https://img.shields.io/crates/v/lazydns.svg)](https://crates.io/crates/lazydns)
[![docs.rs](https://docs.rs/lazydns/badge.svg)](https://docs.rs/lazydns)
[![codecov](https://codecov.io/gh/lazywalker/lazydns/branch/master/graph/badge.svg)](https://codecov.io/gh/lazywalker/lazydns)
[![Dependabot](https://img.shields.io/badge/Dependabot-enabled-brightgreen.svg)](https://github.com/lazywalker/lazydns/network/updates)
[![Maintenance](https://img.shields.io/maintenance/yes/2026)](https://github.com/lazywalker/lazydns)


## Documentation

- **[Docs Home](docs/README.md)** - Main documentation index
- **[Installation Guide](docs/en/03_INSTALLATION.md)** - Installation methods
- **[Configuration Guide](docs/en/04_CONFIGURATION.md)** - Configuration file reference
- **[Implementation](docs/IMPLEMENTATION.md)** - Phased implementation roadmap

## Current Status:

- [x] DNS Protocol & Servers (UDP/TCP/DoT/DoH/DoQ)
- [x] Plugin System & Core Plugins
- [x] **LazyCache with Stale-Serving** - Background refresh cache with TTL support
- [x] **Environment Variable Configuration** - Runtime config via METRICS_ADDR, ADMIN_ADDR, and others.
- [x] **Prometheus Monitoring** - Metrics collection, health checks, and admin interface
- [x] **Advanced Condition Matching** - Flexible rule-based DNS query processing with 30+ matching plugins (Hosts, Domain, IP, GeoIP, GeoSite ...)
- [x] Caching with LRU eviction
- [x] Encrypted DNS (DoT RFC 7858, DoH RFC 8484, DoQ RFC 9250)
- [x] Multi-profile builds (minimal/full) with UPX compression
- [x] Documentation and Docker packaging


## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Acknowledgments

- [mosdns](https://github.com/IrineSistiana/mosdns) - The original inspiration
- [hickory-dns](https://github.com/hickory-dns/hickory-dns) - Rust DNS library
- The Rust community
