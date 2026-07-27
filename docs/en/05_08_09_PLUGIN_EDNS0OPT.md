# EDNS0 Options Plugin

`edns0opt` lets you add arbitrary EDNS0 options to outgoing queries. It stores the options in the context metadata so that forwarders can include them in upstream requests.

## Arguments

- `options`: sequence of `{ code: <number>, data: [<byte>, ...] }`: the option code and raw bytes.
- `preserve_existing` (boolean): whether to preserve existing EDNS0 options (default: true).

## Example

```yaml
plugins:
  - type: edns0opt
    args:
      options:
        - code: 8
          data: [0, 1, 24, 0, 192, 168, 1]
      preserve_existing: false
```

## Behavior

- Stores EDNS0 options in metadata key `edns0_options` for downstream consumption.
- Useful for adding ECS, cookies, or other EDNS0 extensions without modifying forwarding code.
