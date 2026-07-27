//! Audit logger: publishes query logs and security events to the event bus.
//!
//! This is the in-memory data source for the WebUI real-time streams and the
//! alert engine. File-based logging has been removed; all consumers subscribe
//! to the event bus.

use super::event::{AuditEvent, QueryLogEntry, SecurityEventType};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{info, trace};

/// Global audit logger instance.
///
/// Lazily initialized on first access. No explicit `init()` is required;
/// the event bus is initialized at startup when the `web` feature is on.
pub static AUDIT_LOGGER: Lazy<AuditLogger> = Lazy::new(AuditLogger::new);

/// Audit logger that publishes events to the shared event bus.
pub struct AuditLogger {
    /// Enabled security event types (empty = all events published).
    enabled_events: RwLock<HashSet<SecurityEventType>>,

    /// Statistics counters.
    queries_logged: AtomicU64,
    queries_sampled_out: AtomicU64,
    security_events_logged: AtomicU64,
}

impl AuditLogger {
    /// Create a new audit logger (all events enabled by default).
    pub fn new() -> Self {
        Self {
            enabled_events: RwLock::new(HashSet::new()),
            queries_logged: AtomicU64::new(0),
            queries_sampled_out: AtomicU64::new(0),
            security_events_logged: AtomicU64::new(0),
        }
    }

    /// Log a DNS query by publishing it to the event bus.
    ///
    /// Sampling is currently 100% (no sampling configured). If sampling is
    /// needed in the future it can be re-added here.
    pub fn log_query(&self, entry: QueryLogEntry) {
        let subscribers = super::event_bus::publish_query(entry);
        if subscribers > 0 {
            self.queries_logged.fetch_add(1, Ordering::Relaxed);
            trace!(subscribers, "Query published to event bus");
        }
    }

    /// Log a security event by publishing it to the event bus.
    pub async fn log_security(&self, event: AuditEvent) {
        // Filter by enabled event types if the set is non-empty.
        if let AuditEvent::Security { event_type, .. } = &event {
            let enabled = self.enabled_events.read().await;
            if !enabled.is_empty() && !enabled.contains(event_type) {
                return;
            }
        }

        let subscribers = super::event_bus::publish_security(event);
        if subscribers > 0 {
            self.security_events_logged.fetch_add(1, Ordering::Relaxed);
            trace!(subscribers, "Security event published to event bus");
        }
    }

    /// Log a security event (convenience wrapper).
    pub async fn log_security_event(
        &self,
        event_type: SecurityEventType,
        message: impl Into<String>,
        client_ip: Option<std::net::IpAddr>,
        qname: Option<String>,
    ) {
        let event = AuditEvent::security_with_client(event_type, message, client_ip, qname);
        self.log_security(event).await;
    }

    /// Get audit statistics (combined logger + event bus stats).
    pub fn stats(&self) -> AuditStats {
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

    /// Shutdown the audit logger.
    pub async fn shutdown(&self) {
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

/// Audit statistics.
#[derive(Debug, Clone, Default)]
pub struct AuditStats {
    pub queries_logged: u64,
    pub queries_sampled_out: u64,
    pub security_events_logged: u64,
    pub events_dropped: u64,
    pub active_subscribers: usize,
}
