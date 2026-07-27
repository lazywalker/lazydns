# ACL (Query Access Control List) Plugin

The `acl` plugin controls access to the DNS server based on the client's IP address. It evaluates an ordered list of IP network rules and either allows the query to continue or returns a REFUSED response.

## Behavior

- Rules are evaluated in the order configured; the first matching rule wins.
- If no rule matches, the plugin uses the configured `default` action (`allow` or `deny`).
- The plugin reads the client IP from request metadata key `client_ip`. If `client_ip` is absent, the plugin defaults to `127.0.0.1` (and logs a warning).
- Default priority: **2000** (runs very early in the pipeline).

## Configuration

- `default` (string, optional): `allow` or `deny`. Defaults to `deny` when omitted.
- `rules` (sequence of mappings, optional): list of rule objects with fields:
  - `network` (string): CIDR network (such as `192.168.0.0/16`).
  - `action` (string): `allow` or `deny`.

Example:

```yaml
plugins:
  - tag: query_acl
    type: query_acl
    config:
      default: deny
      rules:
        - network: 192.168.0.0/16
          action: allow
        - network: 10.0.0.0/8
          action: allow
```

Notes:

- Use `allow`-list mode by setting `default: deny` and listing allowed networks.
- Use `deny`-list mode by setting `default: allow` and listing blocked networks.
- Rule order matters: place more specific networks before broader ones.


## Implementation details

- The plugin exposes `AclAction` (`Allow`/`Deny`) and evaluates using `ipnet::IpNet`.
- On `Deny` the plugin creates a REFUSED DNS response and sets it in the request context.
- The plugin supports initialization from plugin config and will return a configuration error if rule entries are malformed.

## Troubleshooting

- If clients are being unexpectedly denied, ensure the server populates `client_ip` metadata with the correct remote address.
- Check rule ordering for overlapping networks (place specific prefixes first).
