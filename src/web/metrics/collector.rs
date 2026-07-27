//! Metrics collector
//!
//! Subscribes to the audit event bus and aggregates metrics for the dashboard.

use super::timeseries::{
    LatencyDistribution, LatencyDistributionSnapshot, TimeSeries, TimeSeriesStats,
};
use super::top_n::{TopN, TopNEntry};
use crate::Result;
use crate::plugins::audit::{QueryLogEntry, event_bus};
use crate::web::config::MetricsConfig;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{info, trace};

/// Metrics collector that subscribes to audit events
pub struct MetricsCollector {
    /// Configuration
    config: MetricsConfig,
    /// Top domains tracker
    top_domains: TopN<String>,
    /// Top clients tracker
    top_clients: TopN<String>,
    /// Query rate (queries per second)
    qps: TimeSeries,
    /// Latency distribution
    latency: LatencyDistribution,
    /// Total queries processed
    total_queries: AtomicU64,
    /// Cache hits
    cache_hits: AtomicU64,
    /// Cache misses
    cache_misses: AtomicU64,
    /// Error responses (SERVFAIL and others)
    error_responses: AtomicU64,
    /// Blocked queries
    blocked_queries: AtomicU64,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new(config: &MetricsConfig) -> Result<Self> {
        let window = Duration::from_secs(config.window_secs);

        Ok(Self {
            config: config.clone(),
            top_domains: TopN::new(config.top_n),
            top_clients: TopN::new(config.top_n),
            qps: TimeSeries::new(window, Duration::from_secs(1)),
            latency: LatencyDistribution::new(),
            total_queries: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            error_responses: AtomicU64::new(0),
            blocked_queries: AtomicU64::new(0),
        })
    }

    /// Run the metrics collector (subscribes to event bus)
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let bus = match event_bus() {
            Some(bus) => bus,
            None => {
                info!("Event bus not initialized, metrics collector not starting");
                return Ok(());
            }
        };

        let mut subscriber = bus.subscribe_queries();
        info!("Metrics collector started");

        loop {
            match subscriber.recv().await {
                Some(entry) => {
                    trace!(qname = %entry.qname, "Processing query log entry");
                    self.process_query(&entry);
                }
                None => {
                    info!("Event bus closed, metrics collector stopping");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Process a query log entry
    fn process_query(&self, entry: &QueryLogEntry) {
        self.total_queries.fetch_add(1, Ordering::Relaxed);

        // Track QPS
        self.qps.increment();

        // Track domain if enabled
        if self.config.track_domains {
            // Normalize domain (lowercase, remove trailing dot)
            let domain = entry.qname.trim_end_matches('.').to_lowercase();
            self.top_domains.increment(domain);
        }

        // Track client if enabled
        if self.config.track_clients
            && let Some(client_ip) = entry.client_ip
        {
            self.top_clients.increment(client_ip.to_string());
        }

        // Track latency if enabled (use microsecond precision)
        if self.config.track_latency {
            // Prefer microsecond precision if available
            let latency_ms = if let Some(us) = entry.response_time_us {
                us as f64 / 1000.0
            } else if let Some(ms) = entry.response_time_ms {
                ms as f64
            } else {
                0.0
            };
            if latency_ms > 0.0
                || entry.response_time_us.is_some()
                || entry.response_time_ms.is_some()
            {
                self.latency.add(latency_ms);
            }
        }

        // Track cache hits/misses
        if let Some(cached) = entry.cached {
            if cached {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
            } else {
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Track error responses
        if let Some(ref rcode) = entry.rcode
            && rcode != "NOERROR"
            && rcode != "NXDOMAIN"
        {
            self.error_responses.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get top domains
    pub fn get_top_domains(&self) -> Vec<TopNEntry<String>> {
        self.top_domains.top_entries()
    }

    /// Get top domains within a time window (seconds ago)
    pub fn get_top_domains_within(&self, window_secs: u64) -> Vec<TopNEntry<String>> {
        self.top_domains.top_entries_within(window_secs)
    }

    /// Get top clients
    pub fn get_top_clients(&self) -> Vec<TopNEntry<String>> {
        self.top_clients.top_entries()
    }

    /// Get top clients within a time window (seconds ago)
    pub fn get_top_clients_within(&self, window_secs: u64) -> Vec<TopNEntry<String>> {
        self.top_clients.top_entries_within(window_secs)
    }

    /// Get QPS history
    pub fn get_qps_history(&self) -> Vec<super::timeseries::TimeSeriesPoint> {
        self.qps.points()
    }

    /// Get current QPS rate
    pub fn get_current_qps(&self) -> f64 {
        self.qps.rate()
    }

    /// Get QPS statistics
    pub fn get_qps_stats(&self) -> TimeSeriesStats {
        self.qps.stats()
    }

    /// Get latency distribution
    pub fn get_latency_distribution(&self) -> LatencyDistributionSnapshot {
        self.latency.distribution()
    }

    /// Get overview statistics
    pub fn get_overview(&self) -> MetricsOverview {
        let total = self.total_queries.load(Ordering::Relaxed);
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let errors = self.error_responses.load(Ordering::Relaxed);
        let blocked = self.blocked_queries.load(Ordering::Relaxed);

        let cache_hit_rate = if hits + misses > 0 {
            (hits as f64 / (hits + misses) as f64) * 100.0
        } else {
            0.0
        };

        MetricsOverview {
            total_queries: total,
            queries_per_second: self.get_current_qps(),
            cache_hit_rate,
            cache_hits: hits,
            cache_misses: misses,
            error_responses: errors,
            blocked_queries: blocked,
            unique_domains: self.top_domains.len() as u64,
            unique_clients: self.top_clients.len() as u64,
        }
    }

    /// Increment blocked queries counter
    pub fn record_blocked(&self) {
        self.blocked_queries.fetch_add(1, Ordering::Relaxed);
    }

    /// Clear all metrics
    pub fn clear(&self) {
        self.top_domains.clear();
        self.top_clients.clear();
        self.qps.clear();
        self.latency.clear();
        self.total_queries.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.error_responses.store(0, Ordering::Relaxed);
        self.blocked_queries.store(0, Ordering::Relaxed);
    }
}

/// Overview of all metrics
#[derive(Debug, Clone, Serialize)]
pub struct MetricsOverview {
    pub total_queries: u64,
    pub queries_per_second: f64,
    pub cache_hit_rate: f64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub error_responses: u64,
    pub blocked_queries: u64,
    pub unique_domains: u64,
    pub unique_clients: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> MetricsConfig {
        MetricsConfig {
            window_secs: 60,
            top_n: 10,
            track_clients: true,
            track_domains: true,
            track_latency: true,
        }
    }

    fn sample_entry() -> QueryLogEntry {
        QueryLogEntry::new(
            1234,
            "udp",
            "example.com".to_string(),
            "A".to_string(),
            "IN".to_string(),
        )
        .with_client_ip("192.168.1.1".parse().unwrap())
        .with_response("NOERROR", 1, 10)
        .with_cached(false)
    }

    #[test]
    fn test_process_query() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();
        let entry = sample_entry();

        collector.process_query(&entry);

        assert_eq!(collector.total_queries.load(Ordering::Relaxed), 1);
        assert_eq!(collector.cache_misses.load(Ordering::Relaxed), 1);

        let top_domains = collector.get_top_domains();
        assert_eq!(top_domains.len(), 1);
        assert_eq!(top_domains[0].key, "example.com");
    }

    #[test]
    fn test_overview() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        for i in 0..100 {
            let mut entry = sample_entry();
            entry.qname = format!("domain{}.com", i % 10);
            entry.cached = Some(i % 3 == 0);
            collector.process_query(&entry);
        }

        let overview = collector.get_overview();
        assert_eq!(overview.total_queries, 100);
        assert!(overview.cache_hit_rate > 0.0);
    }

    #[test]
    fn test_cache_hit_rate_calculation() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        // 75 cache hits
        for _ in 0..75 {
            let mut entry = sample_entry();
            entry.cached = Some(true);
            collector.process_query(&entry);
        }

        // 25 cache misses
        for _ in 0..25 {
            let mut entry = sample_entry();
            entry.cached = Some(false);
            collector.process_query(&entry);
        }

        let overview = collector.get_overview();
        assert_eq!(overview.cache_hits, 75);
        assert_eq!(overview.cache_misses, 25);
        // Cache hit rate should be 75%
        assert!((overview.cache_hit_rate - 75.0).abs() < 0.1);
    }

    #[test]
    fn test_error_response_tracking() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        // NOERROR should not count as error
        let mut entry = sample_entry();
        entry.rcode = Some("NOERROR".to_string());
        collector.process_query(&entry);

        // NXDOMAIN should not count as error
        let mut entry = sample_entry();
        entry.rcode = Some("NXDOMAIN".to_string());
        collector.process_query(&entry);

        // SERVFAIL should count as error
        let mut entry = sample_entry();
        entry.rcode = Some("SERVFAIL".to_string());
        collector.process_query(&entry);

        // REFUSED should count as error
        let mut entry = sample_entry();
        entry.rcode = Some("REFUSED".to_string());
        collector.process_query(&entry);

        let overview = collector.get_overview();
        assert_eq!(overview.error_responses, 2);
    }

    #[test]
    fn test_top_clients_tracking() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        // Generate queries from different clients
        for i in 0..50 {
            let mut entry = sample_entry();
            // Client 1 makes more queries
            entry.client_ip = Some(format!("192.168.1.{}", i % 5).parse().unwrap());
            collector.process_query(&entry);
        }

        let top_clients = collector.get_top_clients();
        assert!(!top_clients.is_empty());
        assert!(top_clients.len() <= 10); // Respects top_n config
    }

    #[test]
    fn test_domain_normalization() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        // Query with trailing dot (FQDN format)
        let mut entry = sample_entry();
        entry.qname = "EXAMPLE.COM.".to_string();
        collector.process_query(&entry);

        // Query without trailing dot
        let mut entry2 = sample_entry();
        entry2.qname = "Example.Com".to_string();
        collector.process_query(&entry2);

        let top_domains = collector.get_top_domains();
        assert_eq!(top_domains.len(), 1);
        assert_eq!(top_domains[0].key, "example.com");
        assert_eq!(top_domains[0].count, 2);
    }

    #[test]
    fn test_latency_tracking_microseconds() {
        let config = MetricsConfig {
            track_latency: true,
            ..sample_config()
        };
        let collector = MetricsCollector::new(&config).unwrap();

        // Test with microsecond precision
        let mut entry = sample_entry();
        entry.response_time_us = Some(5000); // 5ms in microseconds
        collector.process_query(&entry);

        let latency = collector.get_latency_distribution();
        assert_eq!(latency.total, 1);
        assert!(latency.avg_ms > 0.0);
    }

    #[test]
    fn test_clear_metrics() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        // Add some queries
        for _ in 0..50 {
            collector.process_query(&sample_entry());
        }

        assert_eq!(collector.total_queries.load(Ordering::Relaxed), 50);

        // Clear all metrics
        collector.clear();

        let overview = collector.get_overview();
        assert_eq!(overview.total_queries, 0);
        assert_eq!(overview.cache_hits, 0);
        assert_eq!(overview.cache_misses, 0);
        assert_eq!(overview.unique_domains, 0);
        assert_eq!(overview.unique_clients, 0);
    }

    #[test]
    fn test_blocked_queries_counter() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        collector.record_blocked();
        collector.record_blocked();
        collector.record_blocked();

        let overview = collector.get_overview();
        assert_eq!(overview.blocked_queries, 3);
    }

    #[test]
    fn test_disabled_tracking_options() {
        let config = MetricsConfig {
            window_secs: 60,
            top_n: 10,
            track_clients: false,
            track_domains: false,
            track_latency: false,
        };
        let collector = MetricsCollector::new(&config).unwrap();

        let mut entry = sample_entry();
        entry.response_time_ms = Some(100);
        collector.process_query(&entry);

        // Domains and clients should not be tracked
        assert!(collector.get_top_domains().is_empty());
        assert!(collector.get_top_clients().is_empty());
    }

    #[test]
    fn test_qps_rate() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        // Add multiple queries
        for _ in 0..100 {
            collector.process_query(&sample_entry());
        }

        let qps = collector.get_current_qps();
        // QPS should be positive
        assert!(qps >= 0.0);

        let stats = collector.get_qps_stats();
        assert_eq!(stats.count, 100);
        assert!(stats.sum >= 100.0);
    }

    #[test]
    fn test_overview_empty_collector() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        let overview = collector.get_overview();
        assert_eq!(overview.total_queries, 0);
        assert_eq!(overview.cache_hit_rate, 0.0);
        assert_eq!(overview.queries_per_second, 0.0);
        assert_eq!(overview.unique_domains, 0);
        assert_eq!(overview.unique_clients, 0);
    }

    #[test]
    fn test_multiple_domains_ranking() {
        let collector = MetricsCollector::new(&sample_config()).unwrap();

        // Domain A: 100 queries
        for _ in 0..100 {
            let mut entry = sample_entry();
            entry.qname = "a.com".to_string();
            collector.process_query(&entry);
        }

        // Domain B: 50 queries
        for _ in 0..50 {
            let mut entry = sample_entry();
            entry.qname = "b.com".to_string();
            collector.process_query(&entry);
        }

        // Domain C: 25 queries
        for _ in 0..25 {
            let mut entry = sample_entry();
            entry.qname = "c.com".to_string();
            collector.process_query(&entry);
        }

        let top_domains = collector.get_top_domains();
        assert!(top_domains.len() >= 3);
        assert_eq!(top_domains[0].key, "a.com");
        assert_eq!(top_domains[0].count, 100);
        assert_eq!(top_domains[1].key, "b.com");
        assert_eq!(top_domains[1].count, 50);
        assert_eq!(top_domains[2].key, "c.com");
        assert_eq!(top_domains[2].count, 25);
    }
}
