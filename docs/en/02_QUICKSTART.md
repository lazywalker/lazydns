# Quickstart

A concise, runnable example to get lazydns up and running locally.

## Build
```bash
cargo build --release
```

## Run with example config
```bash
cargo run -- --config examples/acl.demo.yaml
```

## Try a demo
- `examples/acl.demo.yaml`: ACL demo
- `examples/query_summary.demo.yaml`: Query summary demo

## Docker (optional)
Examples for running with Docker and docker-compose.

---


### Example configuration snippet
A minimal server config that wires `forward` into a UDP listener.

```yaml
# Log configuration
log:
  level: info
  format: text

# Plugin execution flow
plugins:
  # Forward to upstream DNS servers
  - tag: forward
    type: forward
    args:
      concurrent: 3
      upstreams:
        - addr: "8.8.8.8:53"
        - addr: "1.1.1.1:53"
        - "https://8.8.8.8/dns-query"

  # Simple sequence that forwards all queries
  - tag: main_sequence
    type: sequence
    args:
      - exec: $forward
      - exec: accept

  # UDP server listening on :5356
  - tag: udp_server
    type: udp_server
    args:
      entry: main_sequence
      listen: ":5354"

  # TCP server listening on :5356
  - tag: tcp_server
    type: tcp_server
    args:
      entry: main_sequence
      listen: ":5354"

```

This shows a common pattern: forward every query to upstream.