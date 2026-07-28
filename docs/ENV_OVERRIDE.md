# Environment Variable Overrides

lazydns supports overriding configuration values via environment variables at runtime. This is useful for containerized deployments and CI/CD pipelines where configuration needs to be adjusted dynamically without modifying the YAML file.

## Overview

The configuration loader will automatically apply environment variable overrides when loading the YAML configuration. Overrides happen **after** YAML deserialization but **before** validation, allowing for flexible configuration management.

## Patterns

### 1. Top-Level Log Configuration

Override top-level logging settings using these environment variables:

| Variable     | Config Field | Example                | Valid Values                              |
| ------------ | ------------ | ---------------------- | ----------------------------------------- |
| `LOG_LEVEL`  | `log.level`  | `debug`                | `trace`, `debug`, `info`, `warn`, `error` |
| `LOG_FORMAT` | `log.format` | `json`                 | `text`, `json`                            |
| `LOG_FILE`   | `log.file`   | `/var/log/lazydns.log` | Any file path                             |
| `LOG_ROTATE` | `log.rotate` | `daily`                | `never`, `daily`, `hourly`, `size:100M`   |

> Note: The `time_format` option (and `LOG_TIME_FORMAT`) has been removed; logs now use local time by default.

### 2. Server Configuration

Override server settings using these environment variables:

| Variable          | Config Field      | Example          | Valid Values                           |
| ----------------- | ----------------- | ---------------- | -------------------------------------- |
| `ADMIN_ENABLED`   | `admin.enabled`   | `true`           | `true`, `false`, `1`, `0`, `yes`, `no` |
| `ADMIN_ADDR`      | `admin.addr`      | `127.0.0.1:8000` | Any valid address:port                 |
| `METRICS_ENABLED` | `metrics.enabled` | `true`           | `true`, `false`, `1`, `0`, `yes`, `no` |
| `METRICS_ADDR`    | `metrics.addr`    | `127.0.0.1:8001` | Any valid address:port                 |

### 3. Plugin Arguments

Override plugin arguments using the pattern:

```
PLUGINS_<TAG>_ARGS_<KEY>=value
```

Where:

- `<TAG>` is the plugin tag name (normalized to lowercase, `_`: `-`)
- `<KEY>` is the argument key (normalized to lowercase, `_`: `-`)
- `value` is parsed as YAML (supports numbers, booleans, strings, arrays)

#### Examples

**Override plugin server URL:**

```bash
export PLUGINS_ADD_GFWLIST_ARGS_SERVER="http://10.100.100.1"
```

**Override cache size:**

```bash
export PLUGINS_CACHE_ARGS_SIZE="2048"
```

**Override cache negative TTL:**

```bash
export PLUGINS_CACHE_ARGS_NEGATIVE_TTL="3600"
```

**Override enable-prefetch flag:**

```bash
export PLUGINS_CACHE_ARGS_ENABLE_PREFETCH="true"
```

## Value Type Conversion

Environment variable values are automatically converted based on their format:

| Input          | Parsed As | Example                         |
| -------------- | --------- | ------------------------------- |
| `123`          | Number    | `2048`: u64                    |
| `true`/`false` | Boolean   | `true`: bool                   |
| `[1,2,3]`      | Array     | `[8.8.8.8, 1.1.1.1]`: Sequence |
| `text`         | String    | `debug`: String                |

## Usage Examples

### Container Deployment

```bash
docker run -e LOG_FORMAT=json \
           -e LOG_LEVEL=info \
           -e ADMIN_ADDR=0.0.0.0:8000 \
           -e METRICS_ENABLED=true \
           -e METRICS_ADDR=0.0.0.0:8001 \
           -e PLUGINS_CACHE_ARGS_SIZE=4096 \
           -v /path/to/config.yaml:/etc/lazydns/config.yaml \
           lazydns:latest
```

### Docker Compose

```yaml
services:
  lazydns:
    image: lazydns:latest
    environment:
      LOG_FORMAT: json
      LOG_LEVEL: debug
      ADMIN_ENABLED: true
      ADMIN_ADDR: "0.0.0.0:8000"
      METRICS_ENABLED: true
      METRICS_ADDR: "0.0.0.0:8001"
      PLUGINS_ADD_GFWLIST_ARGS_SERVER: "http://list-server:8000"
      PLUGINS_CACHE_ARGS_SIZE: "2048"
    volumes:
      - ./config.yaml:/etc/lazydns/config.yaml
```

### Kubernetes

```yaml
env:
  - name: LOG_LEVEL
    value: "info"
  - name: LOG_FORMAT
    value: "json"
  - name: ADMIN_ENABLED
    value: "true"
  - name: ADMIN_ADDR
    value: "0.0.0.0:8000"
  - name: METRICS_ENABLED
    value: "true"
  - name: METRICS_ADDR
    value: "0.0.0.0:8001"
  - name: PLUGINS_CACHE_ARGS_SIZE
    value: "4096"
  - name: PLUGINS_ADD_GFWLIST_ARGS_SERVER
    value: "http://list-server.default:8000"
```

### CI/CD Pipeline

```bash
#!/bin/bash
export LOG_FORMAT=json
export LOG_LEVEL=warn
export ADMIN_ENABLED=true
export ADMIN_ADDR=0.0.0.0:8000
export METRICS_ENABLED=true
export METRICS_ADDR=0.0.0.0:8001
export PLUGINS_CACHE_ARGS_NEGATIVE_TTL=7200
./lazydns -c /etc/config.yaml
```

## Configuration File Example

You don't need to modify the YAML file; these are typically set in YAML directly, but can now be overridden:

```yaml
admin:
  enabled: true # Can be overridden by ADMIN_ENABLED
  addr: "127.0.0.1:8000" # Can be overridden by ADMIN_ADDR

metrics:
  enabled: false # Can be overridden by METRICS_ENABLED
  addr: "127.0.0.1:8001" # Can be overridden by METRICS_ADDR

log:
  level: info # Can be overridden by LOG_LEVEL
  format: text # Can be overridden by LOG_FORMAT
  file: null # Can be overridden by LOG_FILE

plugins:
  - tag: cache
    plugin_type: cache
    args:
      size: 1024 # Can be overridden by PLUGINS_CACHE_ARGS_SIZE
      negative_ttl: 300 # Can be overridden by PLUGINS_CACHE_ARGS_NEGATIVE_TTL

  - tag: add-gfwlist
    plugin_type: add-gfwlist
    args:
      server: http://default.com # Can be overridden by PLUGINS_ADD_GFWLIST_ARGS_SERVER
```

## Priority

Environment variable overrides have the highest priority and will:

1. Override values from the YAML configuration file
2. Be applied before configuration validation
3. Be visible in logs (use `LOG_LEVEL=debug` to see applied overrides)

## Testing

Run tests with single thread to avoid environment variable interference:

```bash
cargo test -- --test-threads=1
```

## Troubleshooting

### Override Not Applied

1. **Check the variable name**: Ensure it matches the pattern exactly

   - Plugin tags must use `_` which get converted to `-`
   - Example: `add_gfwlist`: `add-gfwlist`

2. **Check the plugin exists**: If a plugin with the tag doesn't exist, a warning is logged

3. **Check value format**: Numbers should be valid YAML numbers, booleans should be `true`/`false`

4. **Enable debug logging**: Set `LOG_LEVEL=debug` to see which overrides were applied

### Invalid Value Format

If an environment variable value doesn't parse as valid YAML (for non-string types), it will be treated as a string fallback.

Example:

```bash
export PLUGINS_CACHE_ARGS_SIZE="2048"    # Parsed as u64
export PLUGINS_CACHE_ARGS_SIZE="invalid"  # Falls back to string "invalid", may cause validation error
```
