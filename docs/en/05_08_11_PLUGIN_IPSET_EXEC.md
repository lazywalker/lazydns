# IpSet (exec) Plugin

The executable `ipset` plugin extracts A/AAAA answers from responses and emits ipset entries (CIDR prefixes). On Linux it attempts to call the `ipset` command; on other platforms it records additions in request metadata under `ipset_added` as `Vec<(String, String)>` (set_name, cidr).

## Quick setup (shorthand)

Accepts a compact shorthand: `"<set_name>,inet,<mask> <set_name6>,inet6,<mask>"` (max two fields). Examples:

```yaml
plugins:
  - exec: ipset:myset,inet,24
  - exec: ipset:myset,inet,24 myset6,inet6,48
```

## Args (programmatic / config)

- `set_name4` / `set_name6`: optional set names for IPv4/IPv6.
- `mask4` / `mask6`: prefix lengths used to convert single A/AAAA addresses into CIDRs.

## Behavior

- Converts A/AAAA answers into CIDR prefixes using configured masks.
- On supported systems the plugin will try to add prefixes to the named ipset; otherwise it places entries into `ipset_added` metadata for external handling.

## Metadata

- `ipset_added` (Vec<(String, String)>): entries added (set, cidr).

## When to use

- Use to dynamically populate firewall address sets based on DNS responses (such as blocklists, sinkhole automation).
