//! Metrics collection and Prometheus exporter
//!
//! This module provides Prometheus metrics used by the DNS server. It
//! exposes a small set of well-documented, process-global metrics that are
//! registered on first use and intended to be scraped by Prometheus (or
//! inspected by `gather_metrics()` in tests).
//!
//! Public metrics offered by this module:
//! - `dns_queries_total{protocol,query_type}`: counter of queries received,
//!   labelled by transport protocol (such as `udp`, `tcp`, `tls`, `doh`) and DNS
//!   question type (such as A/AAAA).
//! - `dns_responses_total{protocol,status}`: counter of responses sent,
//!   labelled by protocol and response status (such as `NOERROR`, `NXDOMAIN`).
//! - `dns_query_duration_seconds{protocol}`: histogram of request processing
//!   latency in seconds, labelled by protocol.
//! - `dns_cache_hits_total`, `dns_cache_misses_total`: simple counters for the
//!   cache subsystem.
//! - `dns_cache_size`: current gauge with number of entries in the cache.
//! - `dns_upstream_queries_total{upstream,status}` and
//!   `dns_upstream_duration_seconds{upstream}`: upstream-specific metrics to
//!   observe health and latency of configured upstream resolvers.
//! - `dns_active_connections{protocol}`: gauge of active connections by
//!   protocol.
//! - `dns_plugin_executions_total{plugin,status}`: counter of plugin
//!   execution events, labelled by plugin name and status string.
//! - `lazydns_process_resident_memory_bytes`: gauge of process RSS from /proc.
//! - `lazydns_process_virtual_memory_bytes`: gauge of process VMS from /proc.
//! - `lazydns_process_cgroup_memory_bytes`: gauge of cgroup memory usage (containers).
//! - `lazydns_process_cgroup_memory_limit_bytes`: gauge of cgroup memory limit.
//!
//! Example (incrementing metrics from application code):
//!
//! ```rust
//! use lazydns::metrics::{DNS_QUERIES_TOTAL, CACHE_HITS_TOTAL};
//!
//! // increment query counter for UDP A queries
//! DNS_QUERIES_TOTAL.with_label_values(&["udp", "A"]).inc();
//!
//! // increment cache hit
//! CACHE_HITS_TOTAL.inc();
//! ```
//!
//! Example: render Prometheus text exposition (useful for tests):
//!
//! ```rust
//! use lazydns::metrics::{gather_metrics, DNS_QUERIES_TOTAL, CACHE_HITS_TOTAL};
//! // ensure the metric is registered and has a sample before gathering exposition
//! DNS_QUERIES_TOTAL.with_label_values(&["udp", "A"]).inc();
//! CACHE_HITS_TOTAL.inc();
//! let text = gather_metrics();
//! assert!(text.contains("dns_queries_total"));
//! ```

// Memory metrics submodule
pub mod memory;

use once_cell::sync::Lazy;
use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts,
    Registry,
};
use std::sync::Arc;

/// Global Prometheus `Registry` used to register process-global metrics.
///
/// This registry is created on first access and shared by all metrics in
/// this module. Prefer using the provided metric helpers rather than
/// registering new metrics directly into this registry at runtime.
pub static METRICS_REGISTRY: Lazy<Arc<Registry>> = Lazy::new(|| Arc::new(Registry::new()));

/// Counter of DNS queries grouped by transport `protocol` and `query_type`.
///
/// Labels:
/// - `protocol`: transport/protocol where the query was received (such as `udp`, `tcp`, `doh`).
/// - `query_type`: DNS question type label (such as `A`, `AAAA`, `TXT`).
pub static DNS_QUERIES_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let counter = IntCounterVec::new(
        Opts::new("dns_queries_total", "Total number of DNS queries"),
        &["protocol", "query_type"],
    )
    .expect("Failed to create dns_queries_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_queries_total");
    counter
});

/// Counter of DNS responses grouped by `protocol` and `status`.
///
/// Labels:
/// - `protocol`: transport/protocol used to send the response.
/// - `status`: textual response code (such as `NOERROR`, `NXDOMAIN`).
pub static DNS_RESPONSES_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let counter = IntCounterVec::new(
        Opts::new("dns_responses_total", "Total number of DNS responses"),
        &["protocol", "status"],
    )
    .expect("Failed to create dns_responses_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_responses_total");
    counter
});

/// Histogram of DNS query processing durations (seconds), labelled by `protocol`.
///
/// Use `observe()` with the request handling duration (in seconds) to track
/// latency distributions for each protocol.
pub static QUERY_DURATION_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    let histogram = HistogramVec::new(
        HistogramOpts::new(
            "dns_query_duration_seconds",
            "DNS query processing duration in seconds",
        )
        .buckets(vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
        ]),
        &["protocol"],
    )
    .expect("Failed to create query_duration_seconds metric");
    METRICS_REGISTRY
        .register(Box::new(histogram.clone()))
        .expect("Failed to register query_duration_seconds");
    histogram
});

/// Counter of cache hits observed by the DNS cache subsystem.
pub static CACHE_HITS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new("dns_cache_hits_total", "Total number of cache hits")
        .expect("Failed to create cache_hits_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register cache_hits_total");
    counter
});

/// Counter of cache misses observed by the DNS cache subsystem.
pub static CACHE_MISSES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new("dns_cache_misses_total", "Total number of cache misses")
        .expect("Failed to create cache_misses_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register cache_misses_total");
    counter
});

/// Gauge exposing the current number of entries in the DNS cache.
pub static CACHE_SIZE: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::new("dns_cache_size", "Current number of entries in cache")
        .expect("Failed to create cache_size metric");
    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register cache_size");
    gauge
});

/// Gauge exposing process uptime in seconds (set from monitoring server)
pub static SERVER_UPTIME_SECONDS: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::new("dns_uptime_seconds", "Process uptime in seconds")
        .expect("Failed to create dns_uptime_seconds metric");
    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register dns_uptime_seconds");
    gauge
});

/// Counter of queries sent to upstream resolvers, labelled by `upstream` and `status`.
///
/// Labels:
/// - `upstream`: identifier or address of the upstream resolver.
/// - `status`: outcome of the upstream query (such as `success`, `timeout`, `error`).
pub static UPSTREAM_QUERIES_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let counter = IntCounterVec::new(
        Opts::new(
            "dns_upstream_queries_total",
            "Total number of upstream queries",
        ),
        &["upstream", "status"],
    )
    .expect("Failed to create upstream_queries_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register upstream_queries_total");
    counter
});

/// Histogram of upstream query durations (seconds), labelled by `upstream`.
pub static UPSTREAM_DURATION_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    let histogram = HistogramVec::new(
        HistogramOpts::new(
            "dns_upstream_duration_seconds",
            "Upstream query duration in seconds",
        )
        .buckets(vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
        ]),
        &["upstream"],
    )
    .expect("Failed to create upstream_duration_seconds metric");
    METRICS_REGISTRY
        .register(Box::new(histogram.clone()))
        .expect("Failed to register upstream_duration_seconds");
    histogram
});

/// Gauge of active connections by `protocol` (such as `udp`, `tcp`).
pub static ACTIVE_CONNECTIONS: Lazy<IntGaugeVec> = Lazy::new(|| {
    let gauge = IntGaugeVec::new(
        Opts::new("dns_active_connections", "Number of active connections"),
        &["protocol"],
    )
    .expect("Failed to create active_connections metric");
    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register active_connections");
    gauge
});

/// Counter of domain validation events, labelled by `result`.
///
/// Labels:
/// - `result`: validation outcome (such as `valid`, `invalid_chars`, `blacklisted`).
pub static DNS_DOMAIN_VALIDATION_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let counter = IntCounterVec::new(
        Opts::new(
            "dns_domain_validation_total",
            "Total number of domain validations",
        ),
        &["result"],
    )
    .expect("Failed to create dns_domain_validation_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_domain_validation_total");
    counter
});

/// Histogram of domain validation durations (seconds).
pub static DNS_DOMAIN_VALIDATION_DURATION_SECONDS: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            "dns_domain_validation_duration_seconds",
            "Domain validation duration in seconds",
        )
        .buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01]),
    )
    .expect("Failed to create dns_domain_validation_duration_seconds metric");
    METRICS_REGISTRY
        .register(Box::new(histogram.clone()))
        .expect("Failed to register dns_domain_validation_duration_seconds");
    histogram
});

/// Counter of domain validation cache hits.
pub static DNS_DOMAIN_VALIDATION_CACHE_HITS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        "dns_domain_validation_cache_hits_total",
        "Total number of domain validation cache hits",
    )
    .expect("Failed to create dns_domain_validation_cache_hits_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_domain_validation_cache_hits_total");
    counter
});

/// Gauge exposing the current number of entries in the domain validation cache.
/// This is useful for monitoring cache pressure and ensuring the cache size
/// behaves as expected (growth, evictions, or unexpected shrinkage).
pub static DNS_DOMAIN_VALIDATION_CACHE_SIZE: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::new(
        "dns_domain_validation_cache_size",
        "Number of entries in domain validation cache",
    )
    .expect("Failed to create dns_domain_validation_cache_size metric");
    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register dns_domain_validation_cache_size");
    gauge
});

/// Counter for number of evictions from all caches (such as response cache, domain validation cache).
/// Incremented when inserting a new entry causes an existing entry to be evicted.
pub static DNS_CACHE_EVICTIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        "dns_cache_evictions_total",
        "Total number of evictions from caches",
    )
    .expect("Failed to create dns_cache_evictions_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_cache_evictions_total");
    counter
});

/// Counter for number of expirations from all caches (entries removed due to TTL expiry)
pub static DNS_CACHE_EXPIRATIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        "dns_cache_expirations_total",
        "Total number of expirations from caches",
    )
    .expect("Failed to create dns_cache_expirations_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_cache_expirations_total");
    counter
});

/// Domain-specific counter for evictions from the domain validation cache.
/// This helps distinguish evictions originating from the domain validator
/// versus other caches.
pub static DNS_DOMAIN_VALIDATION_CACHE_EVICTIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        "dns_domain_validation_cache_evictions_total",
        "Total number of evictions from domain validation cache",
    )
    .expect("Failed to create dns_domain_validation_cache_evictions_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_domain_validation_cache_evictions_total");
    counter
});

/// Counter for misses in the domain validation cache (cache.get() returned None).
/// Helps diagnose cache performance: hits_total + misses_total should approximate
/// the total number of validations performed.
pub static DNS_DOMAIN_VALIDATION_CACHE_MISSES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        "dns_domain_validation_cache_misses_total",
        "Total number of misses in domain validation cache",
    )
    .expect("Failed to create dns_domain_validation_cache_misses_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register dns_domain_validation_cache_misses_total");
    counter
});

/// Counter of plugin execution events, labelled by `plugin` and `status`.
///
/// Typical `status` labels are `ok`, `skipped`, `error`, depending on
/// how the plugin reports its execution result.
pub static PLUGIN_EXECUTIONS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    let counter = IntCounterVec::new(
        Opts::new(
            "dns_plugin_executions_total",
            "Total number of plugin executions",
        ),
        &["plugin", "status"],
    )
    .expect("Failed to create plugin_executions_total metric");
    METRICS_REGISTRY
        .register(Box::new(counter.clone()))
        .expect("Failed to register plugin_executions_total");
    counter
});

/// Gather the current registry metrics and return the Prometheus text exposition.
///
/// This function is primarily useful for tests and health endpoints that want
/// to render the current metrics as a human- and Prometheus-readable string.
pub fn gather_metrics() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let metric_families = METRICS_REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registry() {
        // Just verify metrics are initialized
        let _ = &*METRICS_REGISTRY;
        let _ = &*DNS_QUERIES_TOTAL;
        let _ = &*CACHE_HITS_TOTAL;
    }

    #[test]
    fn test_gather_metrics() {
        // Increment some metrics
        DNS_QUERIES_TOTAL.with_label_values(&["udp", "A"]).inc();
        CACHE_HITS_TOTAL.inc();

        // Gather metrics
        let metrics_text = gather_metrics();
        assert!(metrics_text.contains("dns_queries_total"));
        assert!(metrics_text.contains("dns_cache_hits_total"));
    }

    #[test]
    fn test_query_duration_histogram() {
        QUERY_DURATION_SECONDS
            .with_label_values(&["udp"])
            .observe(0.015);
        let metrics = gather_metrics();
        assert!(metrics.contains("dns_query_duration_seconds"));
    }

    #[test]
    fn test_responses_and_upstream_metrics() {
        DNS_RESPONSES_TOTAL
            .with_label_values(&["udp", "NOERROR"])
            .inc();
        UPSTREAM_QUERIES_TOTAL
            .with_label_values(&["8.8.8.8", "success"]) // label values are strings
            .inc();
        UPSTREAM_DURATION_SECONDS
            .with_label_values(&["8.8.8.8"])
            .observe(0.05);

        let metrics = gather_metrics();
        assert!(metrics.contains("dns_responses_total"));
        assert!(metrics.contains("dns_upstream_queries_total"));
        assert!(metrics.contains("dns_upstream_duration_seconds"));
    }

    #[test]
    fn cache_size_active_connections_plugin_exec() {
        // Set gauge and counters and verify exposition contains values and labels
        CACHE_SIZE.set(42);
        ACTIVE_CONNECTIONS.with_label_values(&["udp"]).set(3);
        PLUGIN_EXECUTIONS_TOTAL
            .with_label_values(&["cache", "ok"])
            .inc();

        let metrics = gather_metrics();
        assert!(metrics.contains("dns_cache_size"));
        assert!(metrics.contains("dns_active_connections"));
        assert!(metrics.contains("dns_plugin_executions_total"));

        // check specific labelled value appears for active connections
        assert!(metrics.contains("dns_active_connections{protocol=\"udp\"} 3"));
    }

    #[test]
    fn test_uptime_metric_exposed() {
        SERVER_UPTIME_SECONDS.set(123);
        let metrics = gather_metrics();
        assert!(metrics.contains("dns_uptime_seconds"));
        assert!(metrics.contains("dns_uptime_seconds 123"));
    }
}
