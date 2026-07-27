
# Reject Plugin

`reject` creates an immediate error response for the current query and stops further rule execution. It constructs a DNS response mirroring the request (same ID and questions) and sets a response code such as `NXDOMAIN`, `REFUSED`, or `SERVFAIL`.

## Exec quick-setup

- Default: `reject` (equivalent to `reject:nxdomain`) returns `NXDOMAIN`.
- Named codes: `reject:nxdomain`, `reject:refused`, `reject:servfail`.
- Numeric: `reject:5` (any valid u8) sets the response code to the provided numeric value.

## Args

- The plugin accepts an optional argument after the prefix to select the response code. Valid forms:
	- `nxdomain` or `nx`
	- `refused` or `ref`
	- `servfail` or `serv`
	- a numeric DNS RCODE (such as `2`, `3`, `5`)

## Behavior

- When executed, `reject` builds a DNS response with:
	- the same message ID as the request,
	- `response` flag set,
	- the request's questions copied into the response,
	- the response code set per the plugin argument.
- It sets the internal return flag and halts sequence execution.

## When to use

- Immediately return a DNS error to the client without forwarding to upstream servers.
- Implement policy-based denials (such as blocking certain queries, refusing TCP/UDP upgrades, or deliberately returning SERVFAIL for testing).

## Examples

- Default NXDOMAIN:

```
- exec: reject
```

- Explicit refused:

```
- exec: reject:refused
```

- Numeric RCODE:

```
- exec: reject:5
```

