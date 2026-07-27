# NftSet Plugin

`nftset` is an exec helper that synthesizes CIDR prefixes from A/AAAA answers and attempts to add them to nftables sets via the `nft` command. On unsupported platforms it records additions in metadata keys `nftset_added_v4` and `nftset_added_v6`.

## Quick setup (shorthand)

Shorthand format: `"family,table,set,addr_type,mask [ ... ]"`, for example:

```yaml
plugins:
  - exec: nftset:inet,my_table,my_set,ipv4_addr,24
```

## Args

- `ipv4` / `ipv6` objects: optional table/set/mask configuration.

## Behavior

- Converts answers to CIDRs using provided masks and adds them to nftables sets when available.
- Writes additions to metadata when `nft` is not available.

## When to use

- Use for firewall automation on systems using nftables.
- Useful for dynamically populating address sets based on DNS responses (such as for blocking or sinkholing).