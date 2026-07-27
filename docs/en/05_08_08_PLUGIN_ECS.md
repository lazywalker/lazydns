# ECS (EDNS Client Subnet) Plugin

`ecs` prepares EDNS0 Client Subnet (ECS) options and stores them in the request `Context` metadata for downstream forwarders to include in upstream queries.

## Arguments

- `forward` (bool): copy client-provided EDNS0 options if present.
- `send` (bool): derive ECS from the client IP (from metadata `client_addr`) and attach it.
- `preset` (string): use a preset IP address instead of deriving from client.
- `mask4` (int): IPv4 source prefix length (default: 24).
- `mask6` (int): IPv6 source prefix length (default: 48).

## Examples

Basic forwarding of client-provided options:

```yaml
plugins:
  - type: ecs
    args:
      forward: true
```

Derive from client address and send ECS:

```yaml
plugins:
  - type: ecs
    args:
      send: true
      mask4: 24
      mask6: 56
```

Use a preset IP for ECS (testing / fixed subnet):

```yaml
plugins:
  - type: ecs
    args:
      preset: "192.0.2.1"
      mask4: 24
```

## Exec quick-setup

`ecs` supports an exec-style quick_setup string with comma-separated `key=value` options, for example:

```
ecs: forward=true
ecs: send=true,mask4=20,mask6=40
ecs: preset=192.0.2.1
```

## Metadata keys

- `edns0_options` (Vec<(u16, Vec<u8>)>): encoded EDNS0 options produced by this plugin.
- `edns0_preserve_existing` (bool): whether to preserve existing options when forwarding.

## When to use

- Use `ecs` when upstream selection should be influenced by (or should include) client subnet information, for example when interacting with geo-aware authoritative servers or CDNs.

