//! Audit logger implementation
//!
//! Provides async file-based audit logging with buffering and rotation.
//! Uses a unified event bus architecture - all consumers (WebUI, file writers, metrics)
//! subscribe to the same event bus for consistent event distribution.

use super::config::{AuditConfig, QueryLogConfig, SecurityEventConfig};
use super::event::{AuditEvent, QueryLogEntry, SecurityEventType};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace, warn};

/// Global audit logger instance
pub static AUDIT_LOGGER: Lazy<AuditLogger> = Lazy::new(AuditLogger::new);

/// Audit logger with async write support
///
/// Uses a unified event bus architecture:
/// - All events are published once to the event bus
/// - Multiple consumers (WebUI SSE, file writers, metrics) subscribe independently
/// - No duplicate channels - cleaner architecture
pub struct AuditLogger {
    /// Configuration
    config: RwLock<Option<AuditConfig>>,

    /// Whether query log file writing is enabled
    query_log_file_enabled: AtomicBool,

    /// Whether security log file writing is enabled
    security_log_file_enabled: AtomicBool,

    /// Enabled security event types (empty = all)
    enabled_events: RwLock<HashSet<SecurityEventType>>,

    /// Sampling threshold (scaled to u64 range)
    sampling_threshold: AtomicU64,

    /// Background writer task handles, so shutdown can abort them. Without
    /// this the spawned writers blocked forever on subscriber.recv() after
    /// shutdown, leaking as orphan tasks.
    writer_tasks: parking_lot::Mutex<Vec<tokio::task::JoinHandle<()>>>,

    /// Guards against re-init spawning duplicate writers that would each
    /// consume the same events and double-write the log.
    writers_spawned: AtomicBool,

    /// Statistics
    queries_logged: AtomicU64,
    queries_sampled_out: AtomicU64,
    security_events_logged: AtomicU64,
}

impl AuditLogger {
    /// Create a new disabled audit logger
    pub fn new() -> Self {
        Self {
            config: RwLock::new(None),
            query_log_file_enabled: AtomicBool::new(false),
            security_log_file_enabled: AtomicBool::new(false),
            enabled_events: RwLock::new(HashSet::new()),
            sampling_threshold: AtomicU64::new(u64::MAX), // 100% by default
            writer_tasks: parking_lot::Mutex::new(Vec::new()),
            writers_spawned: AtomicBool::new(false),
            queries_logged: AtomicU64::new(0),
            queries_sampled_out: AtomicU64::new(0),
            security_events_logged: AtomicU64::new(0),
        }
    }

    /// Initialize the audit logger with configuration
    pub async fn init(&self, config: AuditConfig) -> crate::Result<()> {
        if !config.enabled {
            info!("Audit logging disabled");
            return Ok(());
        }

        info!("Initializing audit logger");

        // If previously initialized (reload), stop the old writers first so we
        // don't end up with duplicate writers consuming the same events and
        // double-writing the log files.
        if self.writers_spawned.swap(true, Ordering::AcqRel) {
            warn!("Audit logger already initialized; stopping previous writers before re-init");
            self.stop_writers();
        }

        // Set up query logging
        if let Some(ref query_config) = config.query_log {
            self.init_query_log(&config, query_config).await?;
        }

        // Set up security event logging
        if let Some(ref security_config) = config.security_events {
            self.init_security_log(&config, security_config).await?;
        }

        *self.config.write().await = Some(config);
        Ok(())
    }

    /// Spawn a background writer task, tracking its handle so `shutdown` /
    /// re-init can abort it. Use this instead of bare `tokio::spawn` for all
    /// audit writer tasks.
    fn spawn_writer<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::spawn(future);
        self.writer_tasks.lock().push(handle);
    }

    /// Abort all tracked writer tasks and clear the handles. Used by shutdown
    /// and re-init. Does not touch the shared event bus (other subscribers
    /// such as the WebUI keep running).
    fn stop_writers(&self) {
        let mut tasks = self.writer_tasks.lock();
        for handle in tasks.drain(..) {
            handle.abort();
        }
        self.query_log_file_enabled.store(false, Ordering::Relaxed);
        self.security_log_file_enabled
            .store(false, Ordering::Relaxed);
    }

    /// Initialize query logging
    async fn init_query_log(
        &self,
        audit_config: &AuditConfig,
        config: &QueryLogConfig,
    ) -> crate::Result<()> {
        debug!("Initializing query log with path: {}", config.path);

        // Set sampling threshold
        let threshold = if config.sampling_rate >= 1.0 {
            u64::MAX
        } else if config.sampling_rate <= 0.0 {
            0
        } else {
            (config.sampling_rate * u64::MAX as f64) as u64
        };
        self.sampling_threshold.store(threshold, Ordering::Relaxed);
        debug!("Set sampling threshold: {}", threshold);

        // Check if file writing is enabled (can override global setting)
        let log_to_file = config.log_to_file.unwrap_or(audit_config.log_to_file);
        if !log_to_file {
            debug!("Query log file writing disabled, only event bus publishing active");
            info!(
                path = %config.path,
                sampling_rate = config.sampling_rate,
                format = %config.format,
                "Query logging initialized (event bus only, file writing disabled)"
            );
            return Ok(());
        }

        // Create parent directories if needed
        if let Some(parent) = Path::new(&config.path).parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                crate::Error::Config(format!(
                    "Failed to create query log directory {:?}: {}",
                    parent, e
                ))
            })?;
            debug!("Created query log directory: {:?}", parent);
        }

        // Mark file writing as enabled
        self.query_log_file_enabled.store(true, Ordering::Relaxed);

        // Subscribe to event bus and spawn writer task
        let path = config.path.clone();
        let format = config.format.clone();
        let buffer_size = config.buffer_size.unwrap_or(audit_config.buffer_size);
        let max_file_size = config.max_file_size.unwrap_or(audit_config.max_file_size);
        let max_files = config.max_files.unwrap_or(audit_config.max_files);

        // Get event bus and subscribe
        if let Some(bus) = super::event_bus::event_bus() {
            let subscriber = bus.subscribe_queries();
            debug!("Query log writer subscribed to event bus");

            self.spawn_writer(async move {
                debug!("Query log writer task started");
                query_log_writer(
                    subscriber,
                    path,
                    format,
                    buffer_size,
                    max_file_size,
                    max_files,
                )
                .await;
            });
        } else {
            warn!("Event bus not initialized, query log file writing disabled");
            self.query_log_file_enabled.store(false, Ordering::Relaxed);
        }

        info!(
            path = %config.path,
            sampling_rate = config.sampling_rate,
            format = %config.format,
            "Query logging initialized"
        );

        Ok(())
    }

    /// Initialize security event logging
    async fn init_security_log(
        &self,
        audit_config: &AuditConfig,
        config: &SecurityEventConfig,
    ) -> crate::Result<()> {
        if !config.enabled {
            return Ok(());
        }

        // Parse enabled events
        let mut enabled = HashSet::new();
        for event_str in &config.events {
            if let Some(event_type) = SecurityEventType::parse(event_str) {
                enabled.insert(event_type);
            } else {
                warn!(event = %event_str, "Unknown security event type");
            }
        }
        *self.enabled_events.write().await = enabled;

        // Check if file writing is enabled (can override global setting)
        let log_to_file = config.log_to_file.unwrap_or(audit_config.log_to_file);
        if !log_to_file {
            debug!("Security event file writing disabled, only event bus publishing active");
            info!(
                path = %config.path,
                events = ?config.events,
                "Security event logging initialized (event bus only, file writing disabled)"
            );
            return Ok(());
        }

        // Create parent directories if needed
        if let Some(parent) = Path::new(&config.path).parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                crate::Error::Config(format!(
                    "Failed to create security log directory {:?}: {}",
                    parent, e
                ))
            })?;
            debug!("Created security log directory: {:?}", parent);
        }

        // Mark file writing as enabled
        self.security_log_file_enabled
            .store(true, Ordering::Relaxed);

        // Subscribe to event bus and spawn writer task
        let path = config.path.clone();
        let buffer_size = config.buffer_size.unwrap_or(audit_config.buffer_size);
        let max_file_size = config.max_file_size.unwrap_or(audit_config.max_file_size);
        let max_files = config.max_files.unwrap_or(audit_config.max_files);

        // Get event bus and subscribe
        if let Some(bus) = super::event_bus::event_bus() {
            let subscriber = bus.subscribe_security();
            debug!("Security log writer subscribed to event bus");

            self.spawn_writer(async move {
                security_log_writer(subscriber, path, buffer_size, max_file_size, max_files).await;
            });
        } else {
            warn!("Event bus not initialized, security log file writing disabled");
            self.security_log_file_enabled
                .store(false, Ordering::Relaxed);
        }

        info!(
            path = %config.path,
            events = ?config.events,
            "Security event logging initialized"
        );

        Ok(())
    }

    /// Check if audit logging is enabled
    pub async fn is_enabled(&self) -> bool {
        self.config.read().await.as_ref().is_some_and(|c| c.enabled)
    }

    /// Check if query logging is enabled (event bus is always active when audit is enabled)
    pub async fn is_query_log_enabled(&self) -> bool {
        self.config
            .read()
            .await
            .as_ref()
            .is_some_and(|c| c.enabled && c.query_log.is_some())
    }

    /// Check if query log file writing is enabled
    pub fn is_query_log_file_enabled(&self) -> bool {
        self.query_log_file_enabled.load(Ordering::Relaxed)
    }

    /// Check if a query should be logged (based on sampling)
    pub fn should_sample(&self) -> bool {
        let threshold = self.sampling_threshold.load(Ordering::Relaxed);
        if threshold == u64::MAX {
            return true;
        }
        if threshold == 0 {
            return false;
        }

        // Use random sampling
        let sample: u64 = rand::random::<u64>();
        sample <= threshold
    }

    /// Log a DNS query
    ///
    /// Publishes to the event bus which distributes to all subscribers
    /// (WebUI SSE, file writers, metrics collectors, etc.). The file-writer
    /// subscribers are tracked via `spawn_writer` and aborted on shutdown /
    /// re-init, so events published after shutdown only reach still-live
    /// subscribers (e.g. WebUI) and are not written to disk.
    pub fn log_query(&self, entry: QueryLogEntry) {
        // Check sampling first (fast path)
        if !self.should_sample() {
            self.queries_sampled_out.fetch_add(1, Ordering::Relaxed);
            return;
        }

        // Publish to event bus - all consumers receive from here
        let subscribers = super::event_bus::publish_query(entry);
        if subscribers > 0 {
            self.queries_logged.fetch_add(1, Ordering::Relaxed);
            trace!(subscribers, "Query published to event bus");
        }
    }

    /// Log a security event
    ///
    /// Publishes to the event bus which distributes to all subscribers
    pub async fn log_security(&self, event: AuditEvent) {
        // Check if this event type is enabled
        if let AuditEvent::Security { event_type, .. } = &event {
            let enabled = self.enabled_events.read().await;
            // If enabled set is not empty, check if this event is in it
            // If enabled set is empty, log all events (no filtering)
            if !enabled.is_empty() && !enabled.contains(event_type) {
                return;
            }
        }

        // Publish to event bus - all consumers receive from here
        let subscribers = super::event_bus::publish_security(event);
        if subscribers > 0 {
            self.security_events_logged.fetch_add(1, Ordering::Relaxed);
            trace!(subscribers, "Security event published to event bus");
        }
    }

    /// Log a security event (convenience method)
    pub async fn log_security_event(
        &self,
        event_type: SecurityEventType,
        message: impl Into<String>,
        client_ip: Option<std::net::IpAddr>,
        qname: Option<String>,
    ) {
        // Respect include_client_ip setting from configuration
        let include_ip = {
            if let Some(cfg) = self.config.read().await.as_ref() {
                cfg.query_log
                    .as_ref()
                    .map(|q| q.include_client_ip)
                    .unwrap_or(true)
            } else {
                true
            }
        };

        let ip_to_use = if include_ip { client_ip } else { None };

        let event = AuditEvent::security_with_client(event_type, message, ip_to_use, qname);
        self.log_security(event).await;
    }

    /// Get statistics
    pub fn stats(&self) -> AuditStats {
        // Get event bus stats for dropped events
        let bus_stats = super::event_bus::event_bus()
            .map(|b| b.stats())
            .unwrap_or_default();

        AuditStats {
            queries_logged: self.queries_logged.load(Ordering::Relaxed),
            queries_sampled_out: self.queries_sampled_out.load(Ordering::Relaxed),
            security_events_logged: self.security_events_logged.load(Ordering::Relaxed),
            events_dropped: bus_stats.events_dropped,
            active_subscribers: bus_stats.active_subscribers,
        }
    }

    /// Shutdown the audit logger
    pub async fn shutdown(&self) {
        // Abort the background writer tasks so they don't keep blocking on
        // subscriber.recv() (and don't keep writing) after shutdown.
        self.stop_writers();
        self.writers_spawned.store(false, Ordering::Release);

        *self.config.write().await = None;

        info!(
            queries = self.queries_logged.load(Ordering::Relaxed),
            sampled_out = self.queries_sampled_out.load(Ordering::Relaxed),
            security_events = self.security_events_logged.load(Ordering::Relaxed),
            "Audit logger shutdown"
        );
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

/// Audit statistics
#[derive(Debug, Clone, Default)]
pub struct AuditStats {
    pub queries_logged: u64,
    pub queries_sampled_out: u64,
    pub security_events_logged: u64,
    pub events_dropped: u64,
    pub active_subscribers: usize,
}

/// Background task for writing query logs
/// Subscribes to event bus and writes to file with buffering and rotation
async fn query_log_writer(
    mut subscriber: super::event_bus::QueryLogSubscriber,
    path: String,
    format: String,
    buffer_size: usize,
    max_file_size: u64,
    max_files: u32,
) {
    debug!("Query log writer task started for path: {}", path);

    let mut buffer: Vec<QueryLogEntry> = Vec::with_capacity(buffer_size);
    let use_json = format == "json";
    let mut flush_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            entry = subscriber.recv() => {
                match entry {
                    Some(entry) => {
                        trace!("Received query log entry: {:?}", entry.qname);
                        buffer.push(entry);
                        if buffer.len() >= buffer_size {
                            debug!("Buffer full ({} entries), flushing", buffer.len());
                            flush_query_buffer(&mut buffer, &path, use_json, max_file_size, max_files);
                        }
                    }
                    None => {
                        // Channel closed, flush remaining and exit
                        if !buffer.is_empty() {
                            debug!("Channel closed, flushing {} remaining entries", buffer.len());
                            flush_query_buffer(&mut buffer, &path, use_json, max_file_size, max_files);
                        }
                        break;
                    }
                }
            }
            // Periodic flush
            _ = flush_interval.tick() => {
                if !buffer.is_empty() {
                    debug!("Periodic flush: flushing {} entries", buffer.len());
                    flush_query_buffer(&mut buffer, &path, use_json, max_file_size, max_files);
                }
            }
        }
    }

    debug!("Query log writer stopped");
}

/// Flush query log buffer to file
fn flush_query_buffer(
    buffer: &mut Vec<QueryLogEntry>,
    path: &str,
    use_json: bool,
    max_file_size: u64,
    max_files: u32,
) {
    debug!("Flushing {} query entries to {}", buffer.len(), path);

    // Ensure parent directory exists (in case it was deleted during runtime)
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        let _ = std::fs::create_dir_all(parent);
    }

    // Check if rotation needed
    if let Ok(metadata) = std::fs::metadata(path)
        && metadata.len() >= max_file_size
    {
        rotate_log_file(path, max_files);
    }

    // Open file for append
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(f) => {
            debug!("Opened query log file: {}", path);
            f
        }
        Err(e) => {
            // Transient I/O failure: keep the buffer so the next flush retries,
            // but cap growth to avoid unbounded memory if the file is
            // permanently unwritable (drop oldest beyond 4x the buffer size).
            error!(path = %path, error = %e, "Failed to open query log file; retaining buffer for retry");
            let cap = buffer.len().max(1024).saturating_mul(4);
            if buffer.len() > cap {
                let drop_n = buffer.len() - cap;
                buffer.drain(..drop_n);
            }
            return;
        }
    };

    let mut writer = std::io::BufWriter::new(file);

    let mut written = 0;
    for entry in buffer.drain(..) {
        let line = if use_json {
            match entry.to_json() {
                Ok(json) => json,
                Err(e) => {
                    error!(error = %e, "Failed to serialize query log entry");
                    continue;
                }
            }
        } else {
            entry.to_text()
        };

        if let Err(e) = writeln!(writer, "{}", line) {
            error!(error = %e, "Failed to write query log entry");
        } else {
            written += 1;
        }
    }

    debug!("Wrote {} entries to buffer", written);

    if let Err(e) = writer.flush() {
        error!(error = %e, "Failed to flush query log");
    } else {
        debug!("Query log buffer flushed successfully");
    }
}

/// Background task for writing security logs
async fn security_log_writer(
    mut subscriber: super::event_bus::SecurityEventSubscriber,
    path: String,
    buffer_size: usize,
    max_file_size: u64,
    max_files: u32,
) {
    let mut buffer: Vec<String> = Vec::with_capacity(buffer_size);
    let mut accumulated_size: u64 = 0;
    let mut flush_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

    // Aggregation: map of (signature -> (json, occurrence_count, first_timestamp))
    // This tracks events that haven't been written yet, allowing aggregation within window
    let mut pending_events: std::collections::HashMap<String, (String, u64, u64)> =
        std::collections::HashMap::with_capacity(20);

    loop {
        tokio::select! {
            event = subscriber.recv() => {
                match event {
                    Some(event) => {
                        if let Ok(json) = event.to_json() {
                            // Extract event signature for aggregation
                            let signature = extract_event_signature(&event);
                            let now_secs = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();

                            // Check if this signature is already pending (within 10s window)
                            if let Some((_, count, _)) = pending_events.get_mut(&signature) {
                                // Same event within aggregation window, just increment count
                                *count += 1;
                                trace!(signature = %signature, count = %count, "Event aggregated (pending)");
                            } else {
                                // New event or window expired, add to pending
                                pending_events.insert(signature.clone(), (json.clone(), 1, now_secs));
                                trace!(signature = %signature, "New event added (pending)");
                            }

                            // Check if we should flush (pending events accumulated)
                            if pending_events.len() >= buffer_size {
                                flush_security_buffer_aggregated(
                                    &path,
                                    &mut pending_events,
                                    &mut buffer,
                                    max_file_size,
                                    max_files,
                                    &mut accumulated_size,
                                    now_secs,
                                );
                            }
                        } else {
                            error!("Failed to serialize security event");
                        }
                    }
                    None => {
                        // Channel closed, flush all pending and exit
                        if !pending_events.is_empty() {
                            let now_secs = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            flush_security_buffer_aggregated(
                                &path,
                                &mut pending_events,
                                &mut buffer,
                                max_file_size,
                                max_files,
                                &mut accumulated_size,
                                now_secs,
                            );
                        }
                        if !buffer.is_empty() {
                            flush_security_buffer(&path, &mut buffer, max_file_size, max_files, &mut accumulated_size);
                        }
                        break;
                    }
                }
            }
            _ = flush_interval.tick() => {
                // Periodic flush of pending events
                if !pending_events.is_empty() {
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    flush_security_buffer_aggregated(
                        &path,
                        &mut pending_events,
                        &mut buffer,
                        max_file_size,
                        max_files,
                        &mut accumulated_size,
                        now_secs,
                    );
                }
                if !buffer.is_empty() {
                    flush_security_buffer(&path, &mut buffer, max_file_size, max_files, &mut accumulated_size);
                }
            }
        }
    }

    debug!("Security log writer stopped");
}

/// Flush aggregated pending events to the write buffer
/// Expires old signatures (> 10s) and converts pending events to aggregated JSON
fn flush_security_buffer_aggregated(
    _path: &str,
    pending_events: &mut std::collections::HashMap<String, (String, u64, u64)>,
    buffer: &mut Vec<String>,
    _max_file_size: u64,
    _max_files: u32,
    accumulated_size: &mut u64,
    now_secs: u64,
) {
    let aggregation_window = 10u64; // 10 seconds

    // Move pending events to buffer, filtering out expired ones and applying aggregation
    for (signature, (json, count, first_ts)) in pending_events.drain() {
        // Only write events within aggregation window or those that haven't been expired
        if now_secs.saturating_sub(first_ts) <= aggregation_window {
            // Format with aggregation count (count > 1 means it was aggregated)
            let aggregated_json = if count > 1 {
                format_aggregated_event(&json, count)
            } else {
                json
            };
            let event_size = aggregated_json.len() as u64 + 1;
            buffer.push(aggregated_json);
            *accumulated_size += event_size;
            debug!(
                signature = %signature,
                count = count,
                "Event flushed to buffer with aggregation"
            );
        } else {
            debug!(
                signature = %signature,
                age = now_secs.saturating_sub(first_ts),
                "Event expired, not writing"
            );
        }
    }
}

/// Flush security event buffer to file
fn flush_security_buffer(
    path: &str,
    buffer: &mut Vec<String>,
    max_file_size: u64,
    max_files: u32,
    accumulated_size: &mut u64,
) {
    if buffer.is_empty() {
        return;
    }

    // Check file size before writing
    if let Ok(metadata) = std::fs::metadata(path) {
        let current_size = metadata.len();
        if current_size + *accumulated_size > max_file_size && current_size > 0 {
            // Need to rotate
            rotate_log_file(path, max_files);
            *accumulated_size = 0;
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        let _ = std::fs::create_dir_all(parent);
    }

    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) => {
            // Transient I/O failure: keep the buffer for the next retry, but
            // cap growth to avoid unbounded memory if the file is permanently
            // unwritable.
            error!(path = %path, error = %e, "Failed to open security log file; retaining buffer for retry");
            let cap = buffer.len().max(1024).saturating_mul(4);
            if buffer.len() > cap {
                let drop_n = buffer.len() - cap;
                buffer.drain(..drop_n);
            }
            return;
        }
    };

    let mut writer = std::io::BufWriter::new(file);
    for json in buffer.drain(..) {
        if let Err(e) = writeln!(writer, "{}", json) {
            error!(error = %e, "Failed to write security event");
        }
    }

    if let Err(e) = writer.flush() {
        error!(error = %e, "Failed to flush security log");
    }

    *accumulated_size = 0;
}

/// Rotate log files (simple numbered rotation)
fn rotate_log_file(path: &str, max_files: u32) {
    // Delete oldest file if at limit
    let oldest = format!("{}.{}", path, max_files);
    let _ = std::fs::remove_file(&oldest);

    // Shift existing files
    for i in (1..max_files).rev() {
        let from = format!("{}.{}", path, i);
        let to = format!("{}.{}", path, i + 1);
        let _ = std::fs::rename(&from, &to);
    }

    // Rename current file to .1
    let first = format!("{}.1", path);
    if let Err(e) = std::fs::rename(path, &first) {
        warn!(path = %path, error = %e, "Failed to rotate log file");
    } else {
        info!(path = %path, "Rotated log file");
    }
}

/// Extract event signature for aggregation purposes
/// Combines event_type and key fields to create a unique signature
fn extract_event_signature(event: &AuditEvent) -> String {
    match event {
        AuditEvent::Security {
            event_type,
            client_ip,
            qname,
            ..
        } => {
            // Signature: "event_type:ip:domain" - capture the essentials
            format!(
                "{}:{}:{}",
                event_type.as_str(),
                client_ip.map(|ip| ip.to_string()).unwrap_or_default(),
                qname.as_deref().unwrap_or("")
            )
        }
        AuditEvent::Query(_) => "query".to_string(),
    }
}

/// Format event JSON with aggregation count
fn format_aggregated_event(original_json: &str, count: u64) -> String {
    // Preserve original JSON key order by appending occurrence_count at the end
    // Remove the closing brace and add the field before it
    let trimmed = original_json.trim_end();
    if let Some(last_brace) = trimmed.rfind('}') {
        let json_body = &trimmed[..last_brace];
        format!(
            "{}{}\"occurrence_count\":{}}}",
            json_body,
            if json_body.trim_end().ends_with(',') || json_body.trim_end().ends_with('{') {
                ""
            } else {
                ","
            },
            count
        )
    } else {
        // Not a valid JSON object, return with occurrence_count appended anyway
        format!(
            "{},\"occurrence_count\":{}}}",
            trimmed.trim_end_matches('}'),
            count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Helper to initialize event bus for tests
    fn init_test_event_bus() {
        super::super::event_bus::init_event_bus(1024);
    }

    #[test]
    fn test_audit_logger_new() {
        let logger = AuditLogger::new();
        let stats = logger.stats();
        assert_eq!(stats.queries_logged, 0);
        assert_eq!(stats.security_events_logged, 0);
    }

    #[test]
    fn test_should_sample_100_percent() {
        let logger = AuditLogger::new();
        // Default is 100% sampling
        for _ in 0..100 {
            assert!(logger.should_sample());
        }
    }

    #[test]
    fn test_should_sample_0_percent() {
        let logger = AuditLogger::new();
        logger.sampling_threshold.store(0, Ordering::Relaxed);
        for _ in 0..100 {
            assert!(!logger.should_sample());
        }
    }

    #[tokio::test]
    async fn test_audit_logger_disabled() {
        let logger = AuditLogger::new();
        let config = AuditConfig::default();
        logger.init(config).await.unwrap();
        assert!(!logger.is_enabled().await);
    }

    #[tokio::test]
    async fn test_audit_logger_query_log() {
        init_test_event_bus();
        let dir = tempdir().unwrap();
        let path = dir.path().join("queries.log");

        let logger = AuditLogger::new();
        let config = AuditConfig {
            enabled: true,
            log_to_file: true,
            buffer_size: 100,
            max_file_size: 100 * 1024 * 1024,
            max_files: 10,
            query_log: Some(QueryLogConfig {
                log_to_file: None,
                path: path.to_string_lossy().to_string(),
                format: "json".into(),
                sampling_rate: 1.0,
                ..Default::default()
            }),
            security_events: None,
        };

        logger.init(config).await.unwrap();
        assert!(logger.is_query_log_enabled().await);
        assert!(logger.is_query_log_file_enabled());

        // Log a query
        let entry = QueryLogEntry::new(1234, "udp", "example.com".into(), "A".into(), "IN".into());
        logger.log_query(entry);

        // Give writer time to receive and flush
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let stats = logger.stats();
        assert_eq!(stats.queries_logged, 1);

        logger.shutdown().await;
    }

    #[tokio::test]
    async fn test_audit_logger_security_event() {
        init_test_event_bus();
        let dir = tempdir().unwrap();
        let path = dir.path().join("security.log");

        let logger = AuditLogger::new();
        let config = AuditConfig {
            enabled: true,
            log_to_file: true,
            buffer_size: 100,
            max_file_size: 100 * 1024 * 1024,
            max_files: 10,
            query_log: None,
            security_events: Some(SecurityEventConfig {
                enabled: true,
                log_to_file: None,
                path: path.to_string_lossy().to_string(),
                events: vec![], // all events
                ..Default::default()
            }),
        };

        logger.init(config).await.unwrap();

        // Log a security event
        logger
            .log_security_event(
                SecurityEventType::RateLimitExceeded,
                "Test rate limit",
                None,
                Some("example.com".into()),
            )
            .await;

        // Give writer time
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let stats = logger.stats();
        assert_eq!(stats.security_events_logged, 1);

        logger.shutdown().await;
    }

    #[tokio::test]
    async fn test_audit_logger_filtered_events() {
        init_test_event_bus();
        let dir = tempdir().unwrap();
        let path = dir.path().join("security.log");

        let logger = AuditLogger::new();
        let config = AuditConfig {
            enabled: true,
            log_to_file: true,
            buffer_size: 100,
            max_file_size: 100 * 1024 * 1024,
            max_files: 10,
            query_log: None,
            security_events: Some(SecurityEventConfig {
                enabled: true,
                log_to_file: None,
                path: path.to_string_lossy().to_string(),
                events: vec!["rate_limit_exceeded".into()], // only this event
                ..Default::default()
            }),
        };

        logger.init(config).await.unwrap();

        // Log allowed event
        logger
            .log_security_event(SecurityEventType::RateLimitExceeded, "Test", None, None)
            .await;

        // Log filtered event (should be ignored)
        logger
            .log_security_event(SecurityEventType::BlockedDomainQuery, "Test", None, None)
            .await;

        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let stats = logger.stats();
        assert_eq!(stats.security_events_logged, 1);

        logger.shutdown().await;
    }

    #[tokio::test]
    async fn test_event_bus_only_mode() {
        init_test_event_bus();

        let logger = AuditLogger::new();
        let config = AuditConfig {
            enabled: true,
            log_to_file: false, // Disable file writing
            buffer_size: 100,
            max_file_size: 100 * 1024 * 1024,
            max_files: 10,
            query_log: Some(QueryLogConfig {
                log_to_file: None,
                path: "/tmp/queries.log".into(),
                format: "json".into(),
                sampling_rate: 1.0,
                ..Default::default()
            }),
            security_events: None,
        };

        logger.init(config).await.unwrap();
        assert!(logger.is_query_log_enabled().await);
        assert!(!logger.is_query_log_file_enabled()); // File writing disabled

        // Log should still work (publishes to event bus)
        let entry = QueryLogEntry::new(1234, "udp", "example.com".into(), "A".into(), "IN".into());
        logger.log_query(entry);

        // Stats should reflect the logged query (event bus publishing)
        let stats = logger.stats();
        // Note: queries_logged only increments if there are subscribers
        // In test mode without subscribers, it won't increment
        assert_eq!(stats.queries_sampled_out, 0);

        logger.shutdown().await;
    }
}
