---
title: lazydns
section: 1
version: "0.2.50"
date: 2025-12-28
manual: "lazydns manual"
---

## NAME

lazydns - a lightweight, plugin-based DNS server and forwarder

## SYNOPSIS

```sh
lazydns [-c|--config <file>] [-d|--dir <dir>] [-v|--verbose] [-V|--version] [-h|--help]
```

## DESCRIPTION

`lazydns` is a high-performance DNS server and forwarding proxy designed to be
extensible via plugins. It supports a YAML configuration file, optional
Prometheus metrics, an administrative HTTP endpoint, and a variety of plugins
for caching, filtering, geoip, and external integration.

This manual page documents the basic command-line interface and behavior.

## OPTIONS

- `-c, --config <file>`
:  Configuration file path. Default: `config.yaml` when running from a
   checkout or the packaged location when installed.

- `-d, --dir <dir>`
:  Working directory to use when running the server. Affects relative paths
   and where runtime files are resolved from.

- `-v, --verbose <count>`
:  Increase logging verbosity. Repeat to raise level: `-v` (debug), `-vv`
   (trace), `-vvv` (trace plus external crate logs).

- `-V, --version`
:  Print version information and exit.

- `-h, --help`
:  Print this help message and exit.

Note: runtime settings such as metrics and admin endpoint addresses are
configured via the YAML configuration file (`lazydns.5`) rather than via the
command-line. Use the configuration file to set `metrics.addr` and
`admin.addr`.

## SIGNALS AND RELOAD

`lazydns` supports configuration reloads without dropping existing queries in
most runtime environments. When running under systemd send `SIGHUP` or use the
admin API to trigger a reload, depending on your deployment. See the
administration manual page `lazydns.8` for service-specific instructions.

## FILES

- `/etc/lazydns/config.yaml`
:  Default system configuration file installed by packaging. If absent,
   `lazydns` will copy a default configuration from the package share on
   first run.

## EXAMPLES

Run with a custom config file:

```sh
lazydns --config /path/to/config.yaml
```

Run in foreground for debugging:

```sh
lazydns --config ./config.example.yaml
```

## SEE ALSO

`lazydns.5`(5), `lazydns.8`(8)

## AUTHOR

The lazydns project authors; see the project's Git repository for a list of
contributors.

## BUGS

Report bugs via the project's issue tracker on GitHub: https://github.com/lazywalker/lazydns/issues
