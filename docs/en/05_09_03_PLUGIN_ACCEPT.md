
# Accept Plugin

`accept` finalizes the current response and stops execution of further rules and parent sequences. It does not modify response records; it only signals that the current response should be treated as final.

## Exec quick-setup

- Quick: `accept` stops processing and accepts the current response (use as `exec: accept`).

## Args

- This plugin takes no arguments. Use the quick-setup string `accept` when specifying in `exec` lists.

## Behavior

- When executed, `accept` sets the internal return flag and immediately halts further sequence execution.
- It does not change response contents or metadata (other than the return flag).
- If there is no response present when `accept` runs, it still halts execution; it does not synthesize a response.

## When to use

- Finalize a response produced earlier in the sequence (for example after `respond`, `set_answers`, or a successful `forward`).
- Short-circuit remaining rules to prevent later plugins from modifying the already-built response.
- Use in complex sequence flows to explicitly mark a response as complete and stop parent sequences from continuing.

## Examples

- Basic sequence that builds a response then accepts it:

```
- exec: respond
- exec: accept
```

- Use `accept` to stop a rule chain once a desired condition is met (such as a match or successful upstream reply).

