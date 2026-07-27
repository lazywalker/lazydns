# Cron Plugin

The `cron` plugin runs scheduled background jobs inside the server process. It supports simple interval jobs and cron-expression schedules, and can perform HTTP requests, shell commands, or invoke other plugins programmatically.

## Purpose of this Plugin
Imageing you want a tiny, standalong deploy without external cron dependencies, but need to run periodic tasks such as: refresing domain/ip rules, pinging health endpoints, rotating logs, or invoking lightweight plugin actions. The `cron` plugin provides a flexible way to schedule and run such tasks within the lazydns server process.

Yep, i ran lazydns inside a container on a router(ROS), and it's all in one image only 7MB, the cron plugin did a good job with downloader plugin, fetching rules from github everday and the `auto_load` feature did the rest. Not extra cron setup needed.

## Key features

- Multiple named jobs in a single plugin instance
- Schedule by `interval_seconds` (fixed delay) or `cron` expression (crontab-style)
- Actions: `http` (method/url/body), `command` (shell command), `invoke_plugin` (create and execute another plugin)
- Uses machine local timezone for cron schedules (timezone config is ignored and will emit a warning)
- Graceful shutdown: spawned jobs are awaited on plugin shutdown

## Configuration

Top-level `args` contain a `jobs` sequence. Each job supports:

- `name` (string, optional): human-friendly job name (default `job`).
- `interval_seconds` (number, optional): run every N seconds. If omitted and `cron` is not provided, defaults to 1 second.
- `cron` (string, optional): crontab-style expression (minute hour day month weekday). Example: `0 */6 * * *`.
- `timezone` (string, optional): present in config but ignored by the plugin (uses local timezone).
- `action` (mapping, required): one of:
  - `http`:
    - `method` (string, optional, default `GET`)
    - `url` (string, required)
    - `body` (string, optional)
  - `command`:
    - (string) a shell command to execute (runs via `sh -c` on Unix and `cmd /C` on Windows)
  - `invoke_plugin`:
    - `type` (string, required): plugin type to create and execute
    - `args` (mapping, optional): arguments passed to the plugin's config

## Example configuration

```yaml
plugins:
  - tag: cron
    type: cron
    config:
      jobs:
        - name: ping_local
          interval_seconds: 60
          action:
            http:
              method: GET
              url: http://127.0.0.1:8000/health

        - name: refresh_cache
          cron: "0 */6 * * *" # every 6 hours
          action:
            invoke_plugin:
              type: cache
              args:
                size: 100

        - name: rotate_logs
          interval_seconds: 3600
          action:
            command: "logrotater --rotate"
```

## Scheduling details

- Cron expressions use the `cronexpr` crate; when computing next run times the plugin uses the machine local timezone as the fallback.
- If the `cron` expression cannot be parsed, the job is skipped and a warning is logged.
- When using `cron` schedules, the plugin computes the next timestamp and sleeps until that time; small clock skew may cause immediate near-zero sleeps which are guarded by a short minimum delay.

## Behavior and lifecycle

- The `cron` plugin does not run per-DNS-request (`should_execute` returns false); it runs jobs in background tasks.
- On server shutdown the plugin signals jobs to stop and awaits job tasks to finish.
- `invoke_plugin` actions construct a temporary plugin instance using the project's plugin factory and execute it with a fresh `Context` and an empty `Message`.

## Logging & troubleshooting

- Logs include job names and action types (such as `Cron job triggered, executing action`).
- If an `http` action fails, the error is logged but other jobs continue running.
- If you rely on a timezone-specific schedule, ensure the server's machine timezone is set as expected; the plugin will warn if `timezone` was set in the config because it is ignored.

## Best practices

- Use `invoke_plugin` for lightweight plugin actions; create plugins designed for idempotent, short-running execution when invoked from cron.
- Keep command actions simple and robust, and prefer dedicated helper scripts for complex tasks.
- Test jobs with short `interval_seconds` values during development before switching to longer schedules.

## See also
- Downloader plugin