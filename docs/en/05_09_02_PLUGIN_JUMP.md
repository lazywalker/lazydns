# Jump Plugin

`jump` is a control-flow plugin used inside sequences to execute another plugin or sequence temporarily and then return to continue the caller sequence. It implements push/return semantics: the runtime executes the target and then resumes the next step of the original sequence.

## Arguments

- This plugin is typically invoked as a sequence step; it does not require a separate config block. The target may be provided as a scalar or as a mapping with explicit arguments.

## Usage

Two common ways to invoke `jump` inside a sequence:

- Inline scalar form (compact):

```yaml
# sequence: main
- exec: jump audit_handler
```

- TODO: Explicit map form (clearer when passing args):

```yaml
# sequence: main
- exec:
    jump: audit_handler
```
Both forms instruct the runtime to execute the plugin or sequence named `audit_handler`, then return to continue the calling sequence.

## Behavior

- When executed, `jump` sets the metadata key `jump_target` (string) to the requested target name and sets the `RETURN_FLAG` to true to signal the sequence executor that control flow needs to handle the jump.
- `SequencePlugin` detects `jump_target` and resolves the target from the injected `__plugin_registry` in context, then executes the target plugin/sequence immediately.
- After the target completes, `SequencePlugin` will continue with the next step of the calling sequence (push/return).
- If a jump target sets `RETURN_FLAG` internally, the sequence implementation takes care to preserve the caller's flow (so target-local return flags do not inadvertently stop the caller). However, if a jump target sets `goto_label`, that will cause a sequence replacement once the target returns (see notes).

## Differences vs `goto`

- `jump`: Push/return; execute target, then continue caller sequence.
- `goto`: Replace; stop caller sequence and replace it with target sequence.

Use `jump` for auxiliary tasks (logging, metrics, checks) that should not permanently alter the caller's flow.

## Edge Cases and Notes

- Jump targets can themselves `jump` to other targets (nested jumps are supported).
- If a jump target sets `goto_label`, that label will be observed after the target returns and the `PluginHandler` will replace the entire entry sequence with the goto target. Use this intentionally and with caution.
- The runtime preserves the caller's `RETURN_FLAG` state across jump target execution so that target-local `RETURN_FLAG` settings do not inadvertently stop the caller (see implementation notes in code).
- Targets passed to `jump` must be registered in the plugin registry (such as sequences or exec plugins).

