# Domain Set Plugin

The `domain_set` plugin provides a flexible domain-matching dataset that other plugins can consume. It loads domain rules from files or inline expressions and exposes them via request metadata for fast lookups.

## Purpose

- Maintain lists of domains with different match semantics (exact, domain+subdomains, regex, keyword).
- Share precompiled domain sets with other plugins (the plugin writes an `Arc<RwLock<DomainRules>>` into metadata keyed by `domain_set_<name>`).

## Match types & priority

The plugin supports four match types. Evaluation priority is: **Full > Domain > Regexp > Keyword**.

- `full`: exact case-insensitive match (such as `full:example.com`). Does not match subdomains.
- `domain`: domain and all subdomains (such as `domain:example.com` matching `example.com` and `www.example.com`). This is the default match type.
- `regexp`: regular expression match (Rust `regex` syntax). Patterns are compiled and evaluated in import order.
- `keyword`: substring match (case-insensitive). Rules evaluated in import order.

When multiple domain rules could match, the priority and domain specificity rules ensure deterministic behavior (more specific domain wins before less specific TLD rules).

## Data formats

Rules may be provided in files or inline via the `exps` argument (sequence or single string). Each non-comment line may be one of:

- `full:example.com`
- `domain:example.com`
- `regexp:.+\.google\.com$`
- `keyword:google`
- `example.com` (uses `default_match_type`, typically `domain`)

Lines starting with `#` are ignored. Leading/trailing whitespace is trimmed.

## Configuration

- `tag` / plugin name: used as the domain-set name if provided.
- `files` (string or sequence): paths to domain list files to load.
- `exps` (string or sequence): inline expressions to load.
- `auto_reload` (bool): enable file watcher to reload when files change (default: false).
- `default_match_type` / `match_type` (string): one of `full`, `domain`, `regexp`, `keyword` (default: `domain`).

Example configuration:

```yaml
plugins:
  - tag: cn-domains
    type: domain_set
    config:
      files:
        - examples/etc/my-domain-list.txt
      auto_reload: true
      default_match_type: domain
```

Or inline expressions:

```yaml
plugins:
  - tag: sample-set
    type: domain_set
    config:
      exps:
        - full:exact.com
        - domain:example.com
        - regexp:.+\.github\.io$
        - keyword:ads
```

## Usage

When the plugin executes, it stores the compiled `DomainRules` in request metadata under the key `domain_set_<name>` where `<name>` is the plugin tag or effective name. Other plugins implementing `Matcher` can read this metadata and call into it.


## Behavior

- Loading: files are loaded first and merged, then inline `exps` are applied.
- Auto-reload: when enabled, changes to any configured file trigger a reload and replace the rules atomically.
- Matching: trailing dots are normalized and matching is case-insensitive.
- Regex rules with invalid patterns are skipped and a warning is logged.

## Diagnostics & stats

- The plugin logs counts of rules loaded (full, domain, regexp, keyword) after loading.
- Use the plugin's `stats()` method to inspect counts programmatically.

## Troubleshooting

- If expected domains are not matching, ensure rules use the intended match type (use `full:` for exact matches and `domain:` for subdomains).
- For large datasets, prefer `domain`/`full` rules where possible (they are O(1) lookups) and avoid excessive regex rules which are O(n) to evaluate.
- If auto-reload isn't picking up changes, verify the process has filesystem read permission and the watched paths are correct.

## Best practices

- Use `default_match_type: domain` for common host-lists so that plain `example.com` matches subdomains.
- Place more specific domain rules (longer suffixes) before broader ones when relying on domain specificity.
- Keep heavy regex usage to a minimum; prefer targeted patterns.
