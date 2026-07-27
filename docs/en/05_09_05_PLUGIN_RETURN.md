
# Return Plugin

`return` stops executing the current sequence and signals the runtime to return control to the caller (if any). It is a control-flow primitive used to short-circuit sequence execution without modifying the response.

## Exec quick-setup

- Quick: `return` (used as `exec: return` to stop the current sequence).

## Args

- This plugin takes no arguments; the quick-setup string must be exactly `return` with no trailing data.

## Behavior

- When executed, `return` sets the internal `RETURN_FLAG` and halts further execution of the current sequence.
- If the current sequence was invoked via `jump`, control returns to the caller sequence; otherwise execution simply stops for the active sequence.
- It does not create or modify DNS responses or other metadata beyond the return flag.

## When to use

- Exit a sequence early after performing local checks or side-effects (such as logging or metrics) so the caller can resume.
- Use inside helper sequences invoked with `jump` when you want to return control to the originator without accepting/rejecting the response.

## Examples

- Stop a helper sequence and return to caller:

```
- exec: some_helper_sequence
- exec: return
```

- Use `return` when you want the caller sequence to continue after a jump-based helper finishes.

