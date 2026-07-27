# Downloader Plugin

`downloader` is an `exec` plugin used to download remote rule files (or other artifacts) and atomically write them to local paths. It is commonly used together with `cron` for scheduled updates of dataset files.

## Supported modes

- Direct configuration: configure `downloader` as a plugin with `files` in its `args`.
- Invoke from other plugins: pass a downloader `args` block (for example via `cron` using `invoke_plugin`).

## Arguments

- `files`: sequence of `{ url: "...", path: "..." }`. Required. Each item specifies the source URL and the destination path to write.
- `timeout_secs`: request timeout in seconds (default: 30).
- `concurrent` (boolean): whether to download files concurrently (default: false).
- `max_retries`: number of retry attempts for each file (default: 3).
- `retry_delay_secs`: base retry delay in seconds (default: 2). Backoff is exponential per attempt.

## Example: direct plugin

```yaml
plugins:
  - tag: file_downloader
    type: downloader
    args:
      files:
        - url: "https://example.com/reject-list.txt"
          path: "examples/reject-list.txt"
        - url: "https://example.com/gfw.txt"
          path: "examples/gfw.txt"
      timeout_secs: 30
      concurrent: false
      max_retries: 3
      retry_delay_secs: 2
```

## Example: invoked by `cron`

```yaml
- tag: cron_scheduler
  type: cron
  args:
    jobs:
      - name: update_rules
        cron: "0 0 */6 * * *" # every 6 hours
        action:
          invoke_plugin:
            type: "downloader"
            args:
              files:
                - url: "https://example.com/reject-list.txt"
                  path: "examples/reject-list.txt"
              timeout_secs: 30
              concurrent: false
```

## Behavior details

- Downloads are written to a temporary `.tmp` file and renamed into place (atomic update).
- Non-success HTTP codes and empty responses are treated as download failures and will be retried according to `max_retries` and exponential backoff.
- When `concurrent` is enabled, each file is downloaded in a separate Tokio task and errors for individual tasks are surfaced.

## Troubleshooting

- If downloads fail, check network connectivity and that the process has write permission to the configured `path`.
- Inspect application logs; the plugin logs progress and per-file warnings/errors (`info`/`warn`/`debug`).

## Notes

- The plugin validates that at least one valid `{url,path}` entry exists in `files` and will fail initialization otherwise.
- Use absolute or repository-relative paths for `path` depending on your deployment.
