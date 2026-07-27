# Audit & Query Logging

## Overview

Audit logging (DNS query logs + security events) is an **automatic, built-in**
feature of the WebUI server. When you build or run lazydns with the `web`
feature enabled, every DNS query and security-relevant event (rate-limit
violations, blocked domains, upstream failures, ACL denials, malformed queries,
timeouts) is published to an internal event bus in real time.

- **No configuration needed**: audit is no longer a plugin you add to your
  sequence.
- **No file output**: data flows through the event bus to the WebUI real-time
  streams and the alert engine. Use the regular lazydns log (`log.file` in your
  config) for persistent program-level logs.
- **Consumers**: the WebUI "Logs" page (SSE streams) and the alert engine
  subscribe to the event bus independently.

## Enabling

Build or run with the `web` (or `web-embed`, `full`) feature:

```bash
# Enable the WebUI + audit automatically
cargo run --features web -- -c config.yaml

# Or with embedded assets (single binary)
cargo run --features web-embed -- -c config.yaml

# Full feature set
cargo run --all-features -- -c config.yaml
```

In your config, ensure the `web` section is enabled:

```yaml
web:
  enabled: true
  listen: "127.0.0.1:8002"
```

Then open the WebUI and navigate to the **Logs** page to see queries and
security events in real time.

## What's logged

### Query logs

Every DNS query (excluding DNS-SD discovery probes) publishes:

| Field | Description |
|-------|-------------|
| `timestamp` | ISO 8601 local time |
| `query_id` | DNS transaction ID |
| `client_ip` | Client address (if available) |
| `protocol` | udp / tcp / dot / doh / doq |
| `qname` | Queried domain |
| `qtype` | A, AAAA, MX, ... |
| `rcode` | Response code (NoError, NXDomain, ...) |
| `answer_count` | Number of answer records |
| `response_time_ms` | Processing latency |
| `cached` | Whether served from cache |
| `answers` | Answer IPs for A/AAAA |

### Security events

Published by individual plugins when triggered:

| Event | Triggered by |
|-------|-------------|
| `rate_limit_exceeded` | Rate-limit plugin |
| `blocked_domain_query` | Black-hole / domain validator |
| `upstream_failure` | Forward plugin (upstream error) |
| `query_timeout` | Forward plugin (timeout) |
| `acl_denied` | ACL plugin |
| `malformed_query` | Domain validator |

## What was removed

Previously, audit was a user-configured plugin (`type: audit`) with extensive
options for file output, rotation, buffer sizes, sampling, and per-log
overrides. This has been simplified; audit is now an internal data source for
the WebUI, with file output removed entirely. If you need structured query logs
for external consumption (such as SIEM/ELK), subscribe to the WebUI SSE streams or
add a custom consumer of the event bus.
