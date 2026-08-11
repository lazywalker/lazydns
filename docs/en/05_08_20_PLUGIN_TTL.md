# TTL Plugin

`ttl` rewrites or clamps TTLs on response records. It supports fixing all TTLs to a single value or applying a min/max range.

## Exec quick-setup

- Fixed TTL: `ttl:60` sets all record TTLs to 60.
- Range: `ttl:30-300` enforces min=30, max=300.

## Args

- `ttl` (integer): fixed TTL. Overrides `min`/`max` when non-zero.
- `min` (integer): minimum TTL; raises TTLs below this value.
- `max` (integer): maximum TTL; lowers TTLs above this value.
- If all three are omitted, defaults to a fixed TTL of 300.

## Behavior

- If `ttl` is set (non-zero), all record TTLs are replaced with that value.
- Otherwise `min` / `max` are applied to clamp existing TTLs.

## When to use

- Enforce caching policies, limit DNS amplification, or normalize TTLs across upstream variability.
- Use to reduce load on upstream servers by increasing TTLs within acceptable bounds.
