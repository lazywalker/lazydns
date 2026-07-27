# Goto Plugin

`goto` is a control-flow plugin used inside sequences to transfer execution to another sequence. It implements "replace" semantics: when a plugin sets a `goto` target the current sequence stops and the plugin system replaces it with the target sequence.

## Arguments

- This plugin does not take custom arguments when used as a step. Instead it is invoked in a sequence step to indicate a target sequence name.

## Usage

There are two common ways to invoke `goto` inside a sequence:

- Inline scalar form (common and compact):

```yaml
# sequence: main
- exec: goto special_handling
```

- TODO: Explicit map form (equivalent, sometimes clearer in complex configs):

```yaml
# sequence: main
- exec:
    goto: special_handling
```

Both forms instruct the runtime to replace the current sequence with the sequence named `special_handling`.

## Behavior

- When executed, `goto` sets the metadata key `goto_label` to the target sequence name and sets the internal `RETURN_FLAG` to `true`.
- The `SequencePlugin` detects the `goto_label` and stops executing further steps in the current sequence.
- `PluginHandler` (the request-level executor) observes `goto_label` after the sequence returns and will look up and execute the target sequence instead.
- Because `goto` replaces the remaining steps, the original sequence does not continue after the target completes.

## Example

A simple example showing `goto` used to route DNS queries that match a condition into an alternative path:

```yaml
plugins:
  - tag: main
    type: sequence
    args:
      - matches: qname $dns_sd_rules
        exec: goto dns_sd_sink
      - exec: $upstream

  - tag: dns_sd_sink
    type: sequence
    args:
      - exec: dbg queries
      - exec: black_hole 127.0.0.1
      - exec: accept
```

Execution for a matching query:
- `main` sequence starts
- `matches` step detects DNS-SD query: `goto dns_sd_sink`
- `main` sequence stops, `dns_sd_sink` sequence starts
- `dns_sd_sink` executes its steps: logs query, sinks it, accepts
- `upstream` step in `main` is never executed unless $dns_sd_rules does not match

## Differences vs `jump`

- `goto`: Replace semantics; stops the current sequence and transfers control to the target sequence. Use when you want to route to an entirely different processing path.
- `jump`: Push/return semantics; executes a target plugin/sequence, then returns and continues the caller sequence. Use when you need auxiliary work that should not permanently alter the caller flow (such as logging, rate checks).

## Implementation notes

- Internally `goto` sets `goto_label` metadata (string) and `RETURN_FLAG` (boolean). `SequencePlugin` breaks on `goto_label` and `PluginHandler` performs the replacement.
- The target must be a registered sequence (plugin registry must contain the named sequence).

## Notes

- Use `goto` when you need to perform a full redirect to alternate processing (error handling, quarantine, specialized handling).
- Be cautious when chaining `goto` targets; the system will execute goto targets in a loop. If the target sets another `goto_label`, it will be handled in turn.


