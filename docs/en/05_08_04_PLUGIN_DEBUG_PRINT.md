# Debug Print Plugin

`debug_print` is an `exec`-style helper plugin that logs DNS query and/or response details to the application log. It is useful for debugging plugin pipelines, verifying question/answer flow, and tracing how plugins transform responses.

## What it does

- Logs query information (qname, qtype, qclass) when configured to print queries.
- Logs a response summary and per-answer details (name, type, ttl) when configured to print responses.
- Supports a `prefix` option to clearly identify log lines from different instances.

## Quick (exec) setup

`debug_print` supports a compact exec-style quick setup string. Options are comma-separated and include:

- `queries`: print queries
- `responses`: print responses
- `prefix=VALUE`: override the log prefix (default: `DNS`)

Examples:

- Print both queries and responses (default):

```yaml
plugins:
  - exec: debug_print:
```

- Print queries only:

```yaml
plugins:
  - exec: debug_print:queries
```

- Print responses with a custom prefix:

```yaml
plugins:
  - exec: debug_print:responses,prefix=TEST
```

## Logging level

`debug_print` uses `info!` for summary lines and `debug!` for per-answer details. To see detailed answers enable the `debug` log level for the process (for example via `RUST_LOG=debug`).

## When to use

- Rapidly inspect queries/responses during development or troubleshooting.
- Short-lived monitoring of pipeline behavior in staging without instrumenting code.

