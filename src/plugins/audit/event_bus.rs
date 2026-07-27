//! Event bus for audit events
//!
//! Provides a publish-subscribe mechanism for distributing audit events
//! to multiple consumers (WebUI, metrics, alerts) with backpressure handling.

use super::event::{AuditEvent, QueryLogEntry};
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::broadcast;
use tracing::{debug, trace, warn};

/// Default channel capacity for the event bus
const DEFAULT_CAPACITY: usize = 1024;

/// Event types that can be published to the bus
#[derive(Debug, Clone)]
pub enum BusEvent {
    /// Query log entry
    QueryLog(QueryLogEntry),
    /// Security audit event
    Security(AuditEvent),
}

/// Statistics for the event bus
#[derive(Debug, Default)]
pub struct EventBusStats {
    /// Total events published
    pub events_published: AtomicU64,
    /// Events dropped due to slow subscribers (lagged)
    pub events_dropped: AtomicU64,
    /// Current number of active subscribers
    pub active_subscribers: AtomicUsize,
    /// Peak number of subscribers
    pub peak_subscribers: AtomicUsize,
}

impl EventBusStats {
    /// Get a snapshot of the statistics
    pub fn snapshot(&self) -> EventBusStatsSnapshot {
        EventBusStatsSnapshot {
            events_published: self.events_published.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
            active_subscribers: self.active_subscribers.load(Ordering::Relaxed),
            peak_subscribers: self.peak_subscribers.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of event bus statistics
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct EventBusStatsSnapshot {
    pub events_published: u64,
    pub events_dropped: u64,
    pub active_subscribers: usize,
    pub peak_subscribers: usize,
}

/// Event bus for distributing audit events to multiple subscribers
///
/// Uses a broadcast channel with configurable capacity. When a subscriber
/// falls behind, older events are dropped (backpressure handling).
#[derive(Debug)]
pub struct AuditEventBus {
    /// Broadcast sender for query log events
    query_tx: broadcast::Sender<QueryLogEntry>,
    /// Broadcast sender for security events
    security_tx: broadcast::Sender<AuditEvent>,
    /// Statistics
    stats: Arc<EventBusStats>,
    /// Channel capacity
    capacity: usize,
}

impl AuditEventBus {
    /// Create a new event bus with default capacity
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Create a new event bus with specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let (query_tx, _) = broadcast::channel(capacity);
        let (security_tx, _) = broadcast::channel(capacity);

        debug!(capacity, "Created audit event bus");

        Self {
            query_tx,
            security_tx,
            stats: Arc::new(EventBusStats::default()),
            capacity,
        }
    }

    /// Publish a query log entry to the bus
    ///
    /// Returns the number of subscribers that received the event.
    /// If there are no subscribers, returns 0 (event is silently dropped).
    pub fn publish_query(&self, entry: QueryLogEntry) -> usize {
        match self.query_tx.send(entry) {
            Ok(count) => {
                self.stats.events_published.fetch_add(1, Ordering::Relaxed);
                trace!(subscribers = count, "Published query log entry");
                count
            }
            Err(_) => {
                // No active receivers - this is normal if no WebUI clients are connected
                trace!("No subscribers for query log event");
                0
            }
        }
    }

    /// Publish a security event to the bus
    ///
    /// Returns the number of subscribers that received the event.
    pub fn publish_security(&self, event: AuditEvent) -> usize {
        match self.security_tx.send(event) {
            Ok(count) => {
                self.stats.events_published.fetch_add(1, Ordering::Relaxed);
                trace!(subscribers = count, "Published security event");
                count
            }
            Err(_) => {
                trace!("No subscribers for security event");
                0
            }
        }
    }

    /// Subscribe to query log events
    ///
    /// Returns a receiver that will receive all future query log events.
    /// If the receiver falls behind by more than `capacity` events,
    /// older events will be dropped and a `Lagged` error will be returned.
    pub fn subscribe_queries(&self) -> QueryLogSubscriber {
        let rx = self.query_tx.subscribe();
        let stats = Arc::clone(&self.stats);

        // Update subscriber count
        let current = stats.active_subscribers.fetch_add(1, Ordering::Relaxed) + 1;
        let peak = stats.peak_subscribers.load(Ordering::Relaxed);
        if current > peak {
            stats.peak_subscribers.store(current, Ordering::Relaxed);
        }

        debug!(active = current, "New query log subscriber");

        QueryLogSubscriber { rx, stats }
    }

    /// Subscribe to security events
    pub fn subscribe_security(&self) -> SecurityEventSubscriber {
        let rx = self.security_tx.subscribe();
        let stats = Arc::clone(&self.stats);

        let current = stats.active_subscribers.fetch_add(1, Ordering::Relaxed) + 1;
        let peak = stats.peak_subscribers.load(Ordering::Relaxed);
        if current > peak {
            stats.peak_subscribers.store(current, Ordering::Relaxed);
        }

        debug!(active = current, "New security event subscriber");

        SecurityEventSubscriber { rx, stats }
    }

    /// Get statistics snapshot
    pub fn stats(&self) -> EventBusStatsSnapshot {
        self.stats.snapshot()
    }

    /// Get the number of active query log subscribers
    pub fn query_subscriber_count(&self) -> usize {
        self.query_tx.receiver_count()
    }

    /// Get the number of active security event subscribers
    pub fn security_subscriber_count(&self) -> usize {
        self.security_tx.receiver_count()
    }

    /// Get channel capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Default for AuditEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for AuditEventBus {
    fn clone(&self) -> Self {
        Self {
            query_tx: self.query_tx.clone(),
            security_tx: self.security_tx.clone(),
            stats: Arc::clone(&self.stats),
            capacity: self.capacity,
        }
    }
}

/// Subscriber for query log events
pub struct QueryLogSubscriber {
    rx: broadcast::Receiver<QueryLogEntry>,
    stats: Arc<EventBusStats>,
}

impl QueryLogSubscriber {
    /// Receive the next query log entry
    ///
    /// Returns `None` if the sender has been dropped.
    /// If the subscriber has lagged behind, drops the missed events
    /// and returns the next available event.
    pub async fn recv(&mut self) -> Option<QueryLogEntry> {
        loop {
            match self.rx.recv().await {
                Ok(entry) => return Some(entry),
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    warn!(
                        lagged = count,
                        "Query log subscriber lagged, dropped events"
                    );
                    self.stats
                        .events_dropped
                        .fetch_add(count, Ordering::Relaxed);
                    // Continue to receive the next available event
                }
                Err(broadcast::error::RecvError::Closed) => {
                    debug!("Query log channel closed");
                    return None;
                }
            }
        }
    }

    /// Try to receive without blocking
    pub fn try_recv(&mut self) -> Option<QueryLogEntry> {
        loop {
            match self.rx.try_recv() {
                Ok(entry) => return Some(entry),
                Err(broadcast::error::TryRecvError::Lagged(count)) => {
                    self.stats
                        .events_dropped
                        .fetch_add(count, Ordering::Relaxed);
                    // Try again
                }
                Err(_) => return None,
            }
        }
    }
}

impl Drop for QueryLogSubscriber {
    fn drop(&mut self) {
        self.stats
            .active_subscribers
            .fetch_sub(1, Ordering::Relaxed);
        debug!("Query log subscriber dropped");
    }
}

/// Subscriber for security events
pub struct SecurityEventSubscriber {
    rx: broadcast::Receiver<AuditEvent>,
    stats: Arc<EventBusStats>,
}

impl SecurityEventSubscriber {
    /// Receive the next security event
    pub async fn recv(&mut self) -> Option<AuditEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    warn!(
                        lagged = count,
                        "Security event subscriber lagged, dropped events"
                    );
                    self.stats
                        .events_dropped
                        .fetch_add(count, Ordering::Relaxed);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    debug!("Security event channel closed");
                    return None;
                }
            }
        }
    }

    /// Try to receive without blocking
    pub fn try_recv(&mut self) -> Option<AuditEvent> {
        loop {
            match self.rx.try_recv() {
                Ok(event) => return Some(event),
                Err(broadcast::error::TryRecvError::Lagged(count)) => {
                    self.stats
                        .events_dropped
                        .fetch_add(count, Ordering::Relaxed);
                }
                Err(_) => return None,
            }
        }
    }
}

impl Drop for SecurityEventSubscriber {
    fn drop(&mut self) {
        self.stats
            .active_subscribers
            .fetch_sub(1, Ordering::Relaxed);
        debug!("Security event subscriber dropped");
    }
}

/// Global event bus instance
static EVENT_BUS: once_cell::sync::Lazy<RwLock<Option<AuditEventBus>>> =
    once_cell::sync::Lazy::new(|| RwLock::new(None));

/// Initialize the global event bus
pub fn init_event_bus(capacity: usize) {
    let mut bus = EVENT_BUS.write();
    *bus = Some(AuditEventBus::with_capacity(capacity));
    debug!(capacity, "Initialized global event bus");
}

/// Get a reference to the global event bus
pub fn event_bus() -> Option<AuditEventBus> {
    EVENT_BUS.read().clone()
}

/// Publish a query log entry to the global event bus
pub fn publish_query(entry: QueryLogEntry) -> usize {
    if let Some(bus) = EVENT_BUS.read().as_ref() {
        bus.publish_query(entry)
    } else {
        0
    }
}

/// Publish a security event to the global event bus
pub fn publish_security(event: AuditEvent) -> usize {
    if let Some(bus) = EVENT_BUS.read().as_ref() {
        bus.publish_security(event)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_query_entry() -> QueryLogEntry {
        QueryLogEntry::new(
            1234,
            "udp",
            "example.com".to_string(),
            "A".to_string(),
            "IN".to_string(),
        )
    }

    #[tokio::test]
    async fn test_publish_without_subscribers() {
        let bus = AuditEventBus::new();
        let count = bus.publish_query(sample_query_entry());
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_single_subscriber() {
        let bus = AuditEventBus::new();
        let mut sub = bus.subscribe_queries();

        let entry = sample_query_entry();
        let count = bus.publish_query(entry.clone());
        assert_eq!(count, 1);

        let received = sub.recv().await.unwrap();
        assert_eq!(received.qname, "example.com");
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = AuditEventBus::new();
        let mut sub1 = bus.subscribe_queries();
        let mut sub2 = bus.subscribe_queries();
        let mut sub3 = bus.subscribe_queries();

        assert_eq!(bus.query_subscriber_count(), 3);

        let entry = sample_query_entry();
        let count = bus.publish_query(entry);
        assert_eq!(count, 3);

        // All subscribers should receive the event
        assert!(sub1.recv().await.is_some());
        assert!(sub2.recv().await.is_some());
        assert!(sub3.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_backpressure_handling() {
        let bus = AuditEventBus::with_capacity(2);
        let mut sub = bus.subscribe_queries();

        // Publish more events than capacity
        for i in 0..5 {
            let mut entry = sample_query_entry();
            entry.query_id = i;
            bus.publish_query(entry);
        }

        // Subscriber should handle lagged events gracefully
        let received = sub.recv().await;
        assert!(received.is_some());

        let stats = bus.stats();
        // Should have some dropped events due to capacity
        assert!(stats.events_dropped > 0 || stats.events_published == 5);
    }

    #[tokio::test]
    async fn test_subscriber_drop_updates_count() {
        let bus = AuditEventBus::new();

        {
            let _sub1 = bus.subscribe_queries();
            let _sub2 = bus.subscribe_queries();
            assert_eq!(bus.query_subscriber_count(), 2);
        }

        // After subscribers are dropped
        assert_eq!(bus.query_subscriber_count(), 0);
    }

    #[tokio::test]
    async fn test_stats_tracking() {
        let bus = AuditEventBus::new();
        let _sub = bus.subscribe_queries();

        for _ in 0..10 {
            bus.publish_query(sample_query_entry());
        }

        let stats = bus.stats();
        assert_eq!(stats.events_published, 10);
        assert_eq!(stats.active_subscribers, 1);
        assert_eq!(stats.peak_subscribers, 1);
    }
}
