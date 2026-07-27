# Sequence Plugin

The `sequence` plugin executes a configurable chain of plugin steps (actions) and optional match conditions. It is the core rule engine used to compose complex request handling logic from smaller plugins (forwarders, cache, ipset writers, debug helpers, and others). The behavior and many built-in matchers/actions are inspired by the MOSDNS `sequence` plugin but adapted to this project's plugin and builder model.

## Overview

- A sequence is an ordered list of rules. Each rule contains zero or more matchers (conditions) and a single action (an executable plugin).
- When a rule's matchers all evaluate to true (or when a rule has no matchers), the rule's action is executed.
- Actions may produce responses, set metadata, or trigger side effects. Sequence execution stops early when an action signals a return/accept/reject or when an action returns an error.
- For efficiency and clarity, define any referenced plugins (by `tag`) earlier in the configuration so the sequence can resolve them at build time.

## Basic configuration shape

This repository implements a simplified sequence constructor in the executable plugin wrapper: the builder accepts a `plugins` array for simple sequences, and the broader configuration format supports a richer `args`-based `rules` structure. Example (YAML):

```yaml
- tag: main
  type: sequence
  args:
    # Simple plugin list (resolved by name)
    plugins:
      - cache
      - forward

    # Or a richer rule list (matchers + exec)
    rules:
      - matches:
          - qname example.com
        exec: black_hole 127.0.0.1 ::1
      - matches:
          - env MOSDNS_ENABLE_CACHE
        exec: cache 1024
      - exec: forward 8.8.8.8
```

Notes:
- `plugins` (simple list) is a convenience for a straight, unconditional sequence of plugin tags.
- `rules` supports per-rule `matches` (an array of matcher expressions) and an `exec` string (an executable plugin name or quick-setup string).

## Matchers (built-in)

Sequence uses a set of built-in matchers for request and response inspection. Common matchers include:

- `qname`: match the query name against a domain expression or a `domain_set` tag.
- `client_ip`, `resp_ip`, `ptr_ip`: match IPs (single, CIDR, or an `ip_set` tag / file list).
- `qtype`, `qclass`, `rcode`: numeric or name-based checks on the request/response fields.
- `cname`: test whether a CNAME in the response matches an expression.
- `has_resp`: true when a response exists.
- `has_wanted_ans`: true if the response contains an answer matching the question type.
- `mark`: match previously-set marks/metadata (useful for composing boolean logic across rules).
- `env`: check environment variable presence/value at runtime (useful feature toggles).
- `random`: probabilistic matcher with a float probability.
- `_true` / `_false`: always-true / always-false helpers for templating rules.
- `string_exp`: experimental string-based matchers for SNI, URL path, or env values.

Each matcher accepts arguments in the same style as other dataset and matcher plugins (tags, inline expressions, or file references). See the individual matcher plugin docs (`domain_set`, `ip_set`) for details on expression formats.

## Built-in actions (common)

Sequence rules typically invoke small, executable plugins as actions. Common actions available in this project include:

- `forward`: send queries to upstream resolvers (supports UDP/TCP/DoT/DoH addresses).
- `cache`: perform caching (simple convenience wrapper; for advanced cache options configure the `cache` plugin directly).
- `black_hole`: synthesize A/AAAA responses with provided IPs.
- `drop_resp`: clear any existing response (useful as part of fallback flows).
- `ecs` / `edns0opt`: attach EDNS0 client-subnet or options.
- `ipset` / `nftset`: materialize A/AAAA answers into kernel sets (or write metadata on unsupported platforms).
- `query_summary`: write a compact summary into request metadata and log it.
- `metrics_collector`: collect per-sequence Prometheus-style metrics (if enabled at build time).
- `prefer_ipv4` / `prefer_ipv6`: experimental helpers that bias dual-stack results.
- `ttl`: rewrite or clamp TTL values on responses.
- `mark`: set lightweight request metadata flags or values.
- Debug / control helpers: `sleep`, `debug_print`, `drop_resp`.

Actions are normal plugins and support the same quick-setup string or `args` configuration used by executable plugins across the project.

## Control flow operations

Sequence includes several control operations for structured flow across multiple sequences or within a sequence:

- `accept`: stop executing further rules in the current sequence and any parent sequences. Treat the current response (if any) as final.
- `reject [rcode]`: stop execution and return a response with the specified `rcode` (default REFUSED).
- `return`: stop executing the current sequence and, if this sequence was invoked via a `jump`, return control to the caller sequence.
- `goto <sequence_tag>`: jump to another sequence (stop current sequence and start target sequence).
- `jump <sequence_tag>`: push into another sequence; when the target sequence finishes the caller continues (unless the target `accept`/`reject`s).

Use `goto`/`jump` to build modular rule sets and delegate handling to named sequences (define target sequences with `tag` and `type: sequence`).

## Error handling

- If an action/plugin returns an error while executing a sequence step, the sequence stops immediately and the error is propagated. This prevents partial or inconsistent processing when a required action fails.

## Examples

1) Simple ordered plugins (unconditional):

```yaml
- tag: simple_chain
  type: sequence
  args:
    plugins: [ mark, cache, forward ]
```

2) Blocking ad domains and forwarding otherwise:

```yaml
- tag: main
  type: sequence
  args:
    rules:
      - matches:
          - qname $ad_domains
        exec: reject 5
      - exec: forward 8.8.8.8
```

3) Conditional caching controlled by environment:

```yaml
- tag: main
  type: sequence
  args:
    rules:
      - matches:
          - env ENABLE_CACHE
        exec: cache 2048
      - exec: forward 1.1.1.1
```

4) Jump/goto example: delegate local handling:

```yaml
- tag: handle_local_query
  type: sequence
  args:
    rules:
      - exec: forward 192.168.0.1

- tag: main
  type: sequence
  args:
    rules:
      - matches:
          - qname internal.example.com
        exec: goto handle_local_query
      - exec: forward 8.8.8.8
```

## Implementation notes (this codebase)

- This repository provides a `SequencePlugin` executable wrapper that runs a vector of `SequenceStep` entries (either unconditional `Exec` steps or conditional `If` steps). The runtime expects plugin instances for each action; when building sequences from configuration, plugin references (tags) should be resolved by the plugin builder/registry earlier in the configuration.
- The `SequencePlugin::init` currently includes a simplified path that accepts a `plugins` array (names) and returns a sequence placeholder; full rule parsing and tag resolution is handled by the project's builder system.
- Sequence steps set and read request `Context` metadata for marks, ipset/nftset additions, metrics, and other side-effects. Many executable plugins (such as `ipset`, `nftset`, `ros_addrlist`) write back metadata when kernel integration is unavailable.

## Troubleshooting & tips

- Define child plugins (by `tag`) before referencing them in a sequence so they can be resolved at build time.
- Use `debug_print` and `query_summary` in early stages of a sequence when troubleshooting rule ordering and data flow.
- Keep performance in mind: actions that perform network calls (forward, downloader, and others) will affect sequence latency. Use `metrics_collector` for per-sequence telemetry where needed.

## See also

- `domain_set`, `ip_set`, and other dataset plugins used by matchers
- `forward`, `cache`, `ipset`, `nftset`, `debug_print`, and other executable plugins used as actions

