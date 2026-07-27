# Logging Usage Guide

Introduction: This guide explains how to control lazydns logging via configuration file, environment variables, or command line.

## Configuration Options

The `log` section in the config file supports:

- `level`: `trace|debug|info|warn|error` - Default log level for lazydns
- `console`: `true|false` - Whether to output logs to console/stdout (default: false)
- `format`: `text` (default) or `json` - Output format (json outputs structured logs)
- `file`: File logging configuration (optional)
  - `enabled`: `true|false` - Whether file logging is enabled (default: false)
  - `path`: Path to the log file
  - `rotation`: Rotation policy configuration
    - `type`: `time|size|both|never` - Rotation trigger type
    - `period`: `daily|hourly|never` - Time period for rotation (for time/both)
    - `max_size`: Maximum file size in bytes before rotation (for size/both)
    - `max_files`: Number of rotated files to keep (for size/both)
    - `compress`: Whether to compress rotated files (reserved for future use)

## Rotation Types

### Time-based Rotation

Rotates logs based on time periods using local timezone.

```yaml
log:
  level: info
  file:
    enabled: true
    path: /var/log/lazydns/app.log
    rotation:
      type: time
      period: daily  # or 'hourly'
```

Generated filenames: `app.log.2026-01-09`, `app.log.2026-01-08`, and so on.

### Size-based Rotation

Rotates logs when file exceeds a size limit with numbered backups.

```yaml
log:
  level: info
  file:
    enabled: true
    path: /var/log/lazydns/app.log
    rotation:
      type: size
      max_size: 10M   # 10MB in bytes
      max_files: 5         # Keep 5 rotated files
```

Generated filenames: `app.log`, `app.log.1`, `app.log.2`, ..., `app.log.5`

### Hybrid Rotation (Time + Size)

Rotates on whichever trigger fires first - useful for high-traffic scenarios.

```yaml
log:
  level: info
  file:
    enabled: true
    path: /var/log/lazydns/app.log
    rotation:
      type: both
      period: daily        # Rotate daily
      max_size: 10M   # OR when file exceeds 10MB
      max_files: 5         # Keep 5 size-rotated files
```

### No Rotation

Append to a single file without rotation.

```yaml
log:
  level: info
  file:
    enabled: true
    path: /var/log/lazydns/app.log
    rotation:
      type: never
```

## Log Level Precedence

Log level is determined by (in order of priority):

1. **Environment variable `RUST_LOG`** - If set and non-empty, used verbatim
2. **Command line `-v` flags** (when `RUST_LOG` not set):
   - No `-v`: use `warn,lazydns=<config.level>` (external crates stay quiet)
   - `-v`: set lazydns to `debug`
   - `-vv`: set lazydns to `trace`
   - `-vvv` or more: global `trace` (includes external crates)
3. **Config file `level`**

## Environment Variable Overrides

The following environment variables can override config settings:

| Variable | Description | Example |
|----------|-------------|---------|
| `RUST_LOG` | Full log specification | `RUST_LOG=trace` |
| `LOG_LEVEL` | Log level | `LOG_LEVEL=debug` |
| `LOG_FORMAT` | Output format | `LOG_FORMAT=json` |
| `LOG_FILE` | Enable file logging with path | `LOG_FILE=/var/log/app.log` |
| `LOG_CONSOLE` | Console output | `LOG_CONSOLE=false` |

## Complete Configuration Example

```yaml
log:
  # Log level: trace, debug, info, warn, error
  level: info
  
  # Output to console (default: false)
  console: true
  
  # Output format: text or json
  format: text
  
  # File logging configuration
  file:
    # Enable file logging (default: false)
    enabled: true
    
    # Path to log file
    path: /var/log/lazydns/lazydns.log
    
    # Rotation configuration
    rotation:
      # Rotation type: time, size, both, or never
      type: both
      
      # Time period: daily or hourly (for type: time or both)
      period: daily
      
      # Max file size in bytes (for type: size or both)
      max_size: 10M   # 10MB
      
      # Number of rotated files to keep (for type: size or both)
      max_files: 5
      
      # Compress rotated files (future feature)
      compress: false
```

## Time and Format Details

- Timestamps use RFC3339-like format with millisecond precision
- Uses local timezone when available, falls back to UTC
- ANSI colors are disabled when writing to file

## Runtime Examples

```bash
# Override all logs with environment variable
RUST_LOG=trace ./lazydns

# Increase verbosity with CLI flags
./lazydns -v      # lazydns shows debug
./lazydns -vv     # lazydns shows trace
./lazydns -vvv    # all crates show trace

# Combine environment overrides
LOG_LEVEL=debug LOG_FILE=/tmp/debug.log ./lazydns
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| Configured `file` but no logs in file | Ensure `file.enabled: true` and binary built with `log-file` feature |
| Incorrect time in logs | Check host timezone and local offset availability |
| Invalid `RUST_LOG` value | Falls back to `warn,lazydns=<config.level>` with warning |
| No logs shown | Ensure log level not set too high; try lowering level via env or CLI |
| Rotation not working | Check `rotation.type` is not `never` and file permissions are correct |

## Feature Flags

The `log-file` feature must be enabled for file logging support. Without it, file configuration is ignored and a warning is logged.

```toml
# Cargo.toml
[dependencies]
lazydns = { version = "0.2", features = ["log-file"] }
```

## Architecture Notes (for developers)

The logging module is designed to be potentially extractable as a standalone crate (`lazylog`) for reuse in other projects. Key components:

- `RotationTrigger` - Serde-compatible rotation policy enum
- `RotationPeriod` - Time period enum (daily/hourly/never)
- `RotatingWriter` - Implements `std::io::Write` with rotation support
- Integration with `tracing-subscriber` via `tracing_appender::non_blocking`
