# Plugin-Based File Downloader

## Overview

implements a lightweight, plugin-based approach for automatically downloading and updating DNS rule files. This approach combines the **Downloader Plugin** with the **Cron Plugin** to provide scheduled file updates without requiring an HTTP server or Admin API.

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│ Schedule: Daily at 02:05 UTC                                    │
│ (Cron Expression: "0 5 2 * * *")                                │
└─────────────────────────────┬───────────────────────────────────┘
                              │
                              ▼
                    ┌──────────────────┐
                    │ CronPlugin       │
                    │ (Scheduler)      │
                    └────────┬─────────┘
                             │
                    invoke_plugin action
                             │
                    ┌────────▼─────────┐
                    │ DownloaderPlugin │
                    │ (Download Files) │
                    └────────┬─────────┘
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
        ▼                    ▼                    ▼
    gfw.txt          reject-list.txt         hosts.txt
        │                    │                    │
        └────────────────────┼────────────────────┘
                             │
                    ┌────────▼────────┐
                    │ File Watcher    │
                    │ (auto_reload)   │
                    └────────┬────────┘
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
        ▼                    ▼                    ▼
   DomainSet           IPSet                  Hosts
   (auto reload)   (auto reload)           (auto reload)
```

## Key Components

### 1. DownloaderPlugin

Handles the actual file downloads with robust error handling and atomic updates.

**Configuration Options:**

```yaml
- tag: file_downloader
  type: downloader
  args:
    files:
      - url: "https://example.com/gfw.txt"
        path: "gfw.txt"
      - url: "https://example.com/reject.txt"
        path: "reject.txt"

    # Download timeout per file (seconds)
    timeout_secs: 30

    # Concurrent download strategy
    concurrent: false
```

**Features:**

- Downloads multiple files with configurable timeout
- Supports both sequential and concurrent download modes
- Atomic file operations (temp, then rename) prevents partial updates
- Built-in retry mechanism for network errors
- Detailed logging of download progress and results

### 2. CronPlugin

Triggers the downloader on a specified schedule.

**Configuration Options:**

```yaml
- tag: auto_update_scheduler
  type: cron
  args:
    # Cron expression: second minute hour day month day-of-week
    cron: "0 5 2 * * *" # Daily at 02:05 UTC

    # Action type
    action: invoke_plugin

    # Target plugin
    args:
      plugin: "file_downloader"
```

**Cron Expression Examples:**

- `0 5 2 * * *` - Every day at 02:05:00 UTC
- `0 0 */6 * * *` - Every 6 hours at 00:00 minutes
- `0 0 0 1 * *` - First day of every month at 00:00 UTC
- `0 0 0 * * 0` - Every Sunday at 00:00 UTC (weekly)

### 3. Auto-Reload Mechanism

DomainSet, IPSet, and Hosts plugins automatically reload files when they change:

```yaml
- tag: domain_gfw
  type: domain_set
  args:
    files:
      - "gfw.txt"
    auto_reload: true # Key: enables automatic reload on file change
```

## Complete Workflow

### 1. Initialization (Server Start)

1. **Plugin Registration**: DownloaderPlugin and CronPlugin register themselves
2. **Data Loading**: DomainSet, IPSet, Hosts load initial rule files
3. **Scheduler Setup**: CronPlugin starts background scheduler thread
4. **File Watching**: auto_reload mechanism starts watching file changes

### 2. Scheduled Event (02:05 UTC)

1. **Cron Trigger**: CronPlugin detects scheduled time
2. **Plugin Invocation**: CronPlugin calls DownloaderPlugin via `invoke_plugin` action
3. **Download Phase**: DownloaderPlugin downloads all configured files
4. **Atomic Update**: Files are written to temp locations, then atomically renamed
5. **File Change Detection**: File watcher detects modification
6. **Automatic Reload**: DomainSet/IPSet/Hosts reload updated files
7. **DNS Updates**: Subsequent DNS queries use new rules

### 3. DNS Query Processing

Rules from reloaded files are immediately available for DNS queries:

```
DNS Query (e.g., facebook.com)
    │
    ▼
Check domain_gfw matcher (uses updated gfw.txt)
    │
    ├─ Found: forward to upstream_proxy
    │
    └─ Not found: forward to upstream_direct
```

## Configuration Examples

### Simple Daily Update (02:05 UTC)

```yaml
plugins:
  # Download files
  - tag: file_downloader
    type: downloader
    args:
      files:
        - url: "https://example.com/gfw.txt"
          path: "gfw.txt"
      timeout_secs: 30
      concurrent: false

  # Schedule the download
  - tag: scheduler
    type: cron
    args:
      cron: "0 5 2 * * *"
      action: invoke_plugin
      args:
        plugin: "file_downloader"

  # Load with auto-reload
  - tag: domain_list
    type: domain_set
    args:
      files: ["gfw.txt"]
      auto_reload: true
```

### Concurrent Downloads (Off-Peak Hours)

```yaml
- tag: file_downloader
  type: downloader
  args:
    files:
      - url: "https://example.com/gfw.txt"
        path: "gfw.txt"
      - url: "https://example.com/reject.txt"
        path: "reject.txt"
      - url: "https://example.com/hosts.txt"
        path: "hosts.txt"
    timeout_secs: 60
    concurrent: true # Download 3 files simultaneously
```

### Weekly Update (Sunday 03:00 UTC)

```yaml
- tag: scheduler
  type: cron
  args:
    cron: "0 0 3 * * 0" # 0 = Sunday
    action: invoke_plugin
    args:
      plugin: "file_downloader"
```

### Multiple Update Schedules

```yaml
# Fast daily update (lightweight lists)
- tag: daily_downloader
  type: downloader
  args:
    files:
      - url: "https://example.com/gfw.txt"
        path: "gfw.txt"
    timeout_secs: 20
    concurrent: false

- tag: daily_scheduler
  type: cron
  args:
    cron: "0 2 2 * * *"
    action: invoke_plugin
    args:
      plugin: "daily_downloader"

# Weekly comprehensive update (all lists)
- tag: weekly_downloader
  type: downloader
  args:
    files:
      - url: "https://example.com/gfw.txt"
        path: "gfw.txt"
      - url: "https://example.com/reject.txt"
        path: "reject.txt"
      - url: "https://example.com/hosts.txt"
        path: "hosts.txt"
    timeout_secs: 60
    concurrent: true

- tag: weekly_scheduler
  type: cron
  args:
    cron: "0 0 3 * * 0" # Weekly
    action: invoke_plugin
    args:
      plugin: "weekly_downloader"
```

## Performance Characteristics

### Download Performance

**Sequential Mode (concurrent: false):**

- 6 files, average 500KB each: ~10-15 seconds
- Bandwidth usage: 1 file at a time (~500KB/s network)
- CPU usage: Minimal
- Memory usage: Minimal (streaming mode)

**Concurrent Mode (concurrent: true):**

- 6 files, average 500KB each: ~2-3 seconds
- Bandwidth usage: All files simultaneously (~3MB/s total)
- CPU usage: Slightly higher (tokio tasks)
- Memory usage: Buffered content for all files

### Reload Performance

**File Replace + Reload:**

- Temp file write: ~100-500ms (depends on file size)
- Atomic rename: <1ms (filesystem operation)
- File watch detection: <100ms (inotify/FSEvents)
- Reload execution: ~50-200ms (parsing + memory update)
- **Total: <1 second typical**

### No Impact on DNS Queries

- Downloads happen in background
- File reload is atomic (no partial updates visible)
- DNS queries continue seamlessly
- No server restart required

## Logging and Monitoring

### Key Log Lines

```bash
# Plugin initialization
INFO downloader: Downloader plugin initialized

# Cron trigger
INFO cron: Cron scheduler started

# Download start
INFO downloader: Starting file downloads count=3 concurrent=false

# File downloaded
INFO downloader: File downloaded url="https://..." path="gfw.txt" size_bytes=524288 duration_ms=2500

# All downloads complete
INFO downloader: All files downloaded successfully count=3 duration_ms=5200

# File reload
INFO domain_set: Reloading file "gfw.txt" from file system

# Reload complete
INFO domain_set: Domain set loaded: 10234 entries
```

### Monitoring Downloads

```bash
# Watch for failed downloads
tail -f logs/lazydns.log | grep "download failed"

# Monitor file sizes after update
ls -lh *.txt

# Check last update time
stat gfw.txt

# Verify log rotation
ls -la logs/lazydns.log*
```

## Troubleshooting

### No Downloads Happening

**Check 1: Cron scheduler running**

```bash
grep "Cron scheduler started" logs/lazydns.log
```

**Check 2: Cron expression correct**

```bash
# Test cron expression with cronexpr tool or online calculator
# "0 5 2 * * *" should trigger at 02:05:00
```

**Check 3: Plugin registered**

```bash
grep "Downloader plugin initialized" logs/lazydns.log
```

### Downloads Timing Out

**Issue**: Files too large or network too slow

**Solution**: Increase timeout_secs

```yaml
- tag: file_downloader
  type: downloader
  args:
    files: [...]
    timeout_secs: 60 # Increased from 30
```

### Files Not Reloading

**Issue**: auto_reload not enabled

**Solution**: Enable auto_reload on data plugins

```yaml
- tag: domain_list
  type: domain_set
  args:
    files: ["gfw.txt"]
    auto_reload: true # Must be explicitly true
```

### Concurrent Downloads Slow

**Issue**: Network bottleneck or overloaded disk

**Solution**: Use sequential mode

```yaml
- tag: file_downloader
  type: downloader
  args:
    files: [...]
    concurrent: false # Download one at a time
```

## Implementation Details

### Atomic File Operations

```rust
// 1. Download to temporary file
let temp_path = format!("{}.tmp", spec.path);
write_to_file(&temp_path, &content)?;

// 2. Atomic rename (OS-level guarantee)
fs::rename(&temp_path, &spec.path)?;

// 3. File watcher detects change
// 4. auto_reload triggers reload
```

**Benefits:**

- No partial files visible to the system
- Process crash during download doesn't corrupt files
- File watchers get clean "file modified" event

### Cron to Plugin Invocation

**invoke_plugin Action Flow:**

1. CronPlugin detects schedule match
2. Looks up target plugin by tag: `plugin: "file_downloader"`
3. Calls plugin's `execute()` method
4. Plugin processes download task asynchronously
5. Cron continues without waiting for completion

```rust
// Pseudo-code
if let Some(plugin_tag) = args.get("plugin") {
    if let Some(plugin) = plugin_registry.get(plugin_tag) {
        // Spawn async task
        tokio::spawn(async move {
            let _ = plugin.execute(&mut ctx).await;
        });
    }
}
```

## Advanced Configuration

### Custom File Processing

For formats requiring transformation (such as dnsmasq → POSIX domain list):

```yaml
# Download raw dnsmasq format
- tag: downloader
  type: downloader
  args:
    files:
      - url: "https://example.com/dnsmasq.conf"
        path: "dnsmasq-raw.conf"
    timeout_secs: 30
# Post-process with external command (requires Plan A elements)
# TODO: Add exec_plugin to run conversion script
```

### Fallback Domains

Ensure service availability if downloads fail:

```yaml
- tag: domain_gfw
  type: domain_set
  args:
    files:
      - "gfw.txt" # Updated by downloader
      - "gfw-fallback.txt" # Static fallback
    auto_reload: true
```
