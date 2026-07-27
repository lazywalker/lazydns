//! Metrics collector executable plugin.
//!
//! This plugin provides Prometheus-backed metrics similar to the
//! `metrics_collector` executable plugin in upstream mosdns. It exposes
//! a small, test-friendly constructor `MetricsCollectorPlugin::new` that
//! accepts a shared counter used by tests or higher-level wiring. The
//! executable-style QuickSetup (registering with runtime) can be added
//! separately where needed.

use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, Result};
use async_trait::async_trait;
#[cfg(feature = "metrics")]
use once_cell::sync::Lazy;
#[cfg(feature = "metrics")]
use prometheus;
// Text encoding of metrics used in tests via fully-qualified call
#[cfg(feature = "metrics")]
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "metrics")]
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "metrics")]
/// Type alias for the metrics tuple to reduce type complexity
type MetricsTuple = (
    prometheus::Counter,
    prometheus::Counter,
    prometheus::Gauge,
    prometheus::Histogram,
);

#[cfg(feature = "metrics")]
static METRICS_CACHE: Lazy<Mutex<HashMap<String, MetricsTuple>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Metrics collector plugin: counts queries and accumulates latency.
#[derive(Debug, Clone, RegisterExecPlugin)]
pub struct MetricsCollectorPlugin {
    counter: Arc<AtomicUsize>,
    _start_time: std::time::Instant,
    last_reset: Arc<std::sync::RwLock<std::time::Instant>>,
    total_latency_ms: Arc<AtomicUsize>,
}

impl MetricsCollectorPlugin {
    /// Create a new metrics collector with shared counter.
    pub fn new(counter: Arc<AtomicUsize>) -> Self {
        let now = std::time::Instant::now();
        Self {
            counter,
            _start_time: now,
            last_reset: Arc::new(std::sync::RwLock::new(now)),
            total_latency_ms: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Get the total query count.
    pub fn count(&self) -> usize {
        self.counter.load(Ordering::SeqCst)
    }

    /// Calculate queries per second since last reset.
    pub fn queries_per_second(&self) -> f64 {
        let count = self.count() as f64;
        let last_reset = match self.last_reset.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("MetricsCollectorPlugin last_reset RwLock was poisoned, recovering");
                poisoned.into_inner()
            }
        };
        let duration = last_reset.elapsed().as_secs_f64();

        if duration > 0.0 {
            count / duration
        } else {
            0.0
        }
    }

    /// Get average latency in milliseconds.
    pub fn average_latency_ms(&self) -> f64 {
        let total_latency = self.total_latency_ms.load(Ordering::SeqCst) as f64;
        let count = self.count() as f64;

        if count > 0.0 {
            total_latency / count
        } else {
            0.0
        }
    }

    /// Reset the metrics counters.
    pub fn reset(&self) {
        self.counter.store(0, Ordering::SeqCst);
        self.total_latency_ms.store(0, Ordering::SeqCst);
        let mut last_reset = match self.last_reset.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("MetricsCollectorPlugin last_reset RwLock was poisoned, recovering");
                poisoned.into_inner()
            }
        };
        *last_reset = std::time::Instant::now();
    }

    /// Get time since metrics were last reset.
    pub fn time_since_reset(&self) -> std::time::Duration {
        let last_reset = match self.last_reset.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("MetricsCollectorPlugin last_reset RwLock was poisoned, recovering");
                poisoned.into_inner()
            }
        };
        last_reset.elapsed()
    }
}

#[async_trait]
impl Plugin for MetricsCollectorPlugin {
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        self.counter.fetch_add(1, Ordering::SeqCst);

        // Track latency if available from metadata
        if let Some(latency_ms) = ctx.get_metadata::<f64>("query_latency_ms") {
            self.total_latency_ms
                .fetch_add((*latency_ms) as usize, Ordering::SeqCst);
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "metrics_collector"
    }
}

#[async_trait]
impl ExecPlugin for MetricsCollectorPlugin {
    /// Parse a quick configuration string for metrics collector plugin.
    ///
    /// The exec_str is currently unused and can be empty.
    /// Future versions may support configuration options.
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != "metrics_collector" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'metrics_collector'",
                prefix
            )));
        }

        // For now, exec_str is ignored - could be extended for future options
        let _exec_str = exec_str.trim();

        // Create a new shared counter for this metrics collector instance
        let counter = Arc::new(AtomicUsize::new(0));
        let plugin = MetricsCollectorPlugin::new(counter);

        Ok(Arc::new(plugin))
    }
}

#[cfg(feature = "metrics")]
#[derive(crate::RegisterExecPlugin)]
/// Prometheus-backed metrics collector plugin.
///
/// This plugin integrates with Prometheus to collect and expose DNS query metrics.
/// It provides counters for total queries, errors, gauges for active threads,
/// and histograms for response latency.
#[derive(Debug, Clone)]
pub struct PromMetricsCollectorPlugin {
    query_total: prometheus::Counter,
    err_total: prometheus::Counter,
    thread: prometheus::Gauge,
    response_latency: prometheus::Histogram,
}

#[cfg(feature = "metrics")]
impl PromMetricsCollectorPlugin {
    /// Create a new Prometheus metrics collector with the given registry and name.
    /// If no registry is provided, uses the global METRICS_REGISTRY.
    pub fn new(registry: Option<&prometheus::Registry>, name: &str) -> Result<Self> {
        let registry = registry.unwrap_or(&*crate::metrics::METRICS_REGISTRY);

        // Generate unique metric names using the collector name to avoid conflicts
        let query_total_name = format!("dns_query_total_{}", name.replace("-", "_"));
        let err_total_name = format!("dns_err_total_{}", name.replace("-", "_"));
        let thread_name = format!("dns_thread_active_{}", name.replace("-", "_"));
        let latency_name = format!(
            "dns_response_latency_millisecond_{}",
            name.replace("-", "_")
        );

        // Check if metrics already exist in cache
        let cache_key = name.to_string();
        let (query_total, err_total, thread, response_latency) = {
            let mut cache = METRICS_CACHE.lock().unwrap();
            if let Some(metrics) = cache.get(&cache_key) {
                // Use existing metrics from cache
                metrics.clone()
            } else {
                // Create new metrics
                let q_opts = prometheus::Opts::new(
                    &query_total_name,
                    "The total number of DNS queries processed",
                )
                .const_label("collector", name.to_string());
                let query_total = prometheus::Counter::with_opts(q_opts).map_err(|e| {
                    crate::Error::Other(format!("prometheus counter opts error: {}", e))
                })?;

                let e_opts = prometheus::Opts::new(
                    &err_total_name,
                    "The total number of DNS queries that failed",
                )
                .const_label("collector", name.to_string());
                let err_total = prometheus::Counter::with_opts(e_opts).map_err(|e| {
                    crate::Error::Other(format!("prometheus counter opts error: {}", e))
                })?;

                let t_opts = prometheus::Opts::new(
                    &thread_name,
                    "The number of threads that are currently being processed",
                )
                .const_label("collector", name.to_string());
                let thread = prometheus::Gauge::with_opts(t_opts).map_err(|e| {
                    crate::Error::Other(format!("prometheus gauge opts error: {}", e))
                })?;

                let h_opts = prometheus::HistogramOpts::new(
                    &latency_name,
                    "The response latency in millisecond",
                )
                .const_label("collector", name.to_string());
                let response_latency = prometheus::Histogram::with_opts(h_opts).map_err(|e| {
                    crate::Error::Other(format!("prometheus histogram opts error: {}", e))
                })?;

                // Register metrics to provided registry.
                registry
                    .register(Box::new(query_total.clone()))
                    .map_err(|e| {
                        crate::Error::Other(format!("failed to register query_total: {}", e))
                    })?;
                registry
                    .register(Box::new(err_total.clone()))
                    .map_err(|e| {
                        crate::Error::Other(format!("failed to register err_total: {}", e))
                    })?;
                registry.register(Box::new(thread.clone())).map_err(|e| {
                    crate::Error::Other(format!("failed to register thread: {}", e))
                })?;
                registry
                    .register(Box::new(response_latency.clone()))
                    .map_err(|e| {
                        crate::Error::Other(format!("failed to register response_latency: {}", e))
                    })?;

                // Cache the metrics
                let metrics_tuple = (
                    query_total.clone(),
                    err_total.clone(),
                    thread.clone(),
                    response_latency.clone(),
                );
                cache.insert(cache_key, metrics_tuple);

                (query_total, err_total, thread, response_latency)
            }
        };

        Ok(Self {
            query_total,
            err_total,
            thread,
            response_latency,
        })
    }

    /// Create a new Prometheus metrics collector using the global registry.
    pub fn with_global_registry(name: &str) -> Result<Self> {
        Self::new(None, name)
    }
}

#[cfg(feature = "metrics")]
#[async_trait]
impl Plugin for PromMetricsCollectorPlugin {
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        self.query_total.inc();

        // Track response latency if available
        if let Some(latency_ms) = ctx.get_metadata::<f64>("query_latency_ms") {
            self.response_latency.observe(*latency_ms);
        }

        // Check for errors and increment error counter if needed
        if let Some(error) = ctx.get_metadata::<bool>("query_error")
            && *error
        {
            self.err_total.inc();
        }

        // Update thread gauge (simplified - in real implementation this would track active threads)
        // For now, we'll just set it to 1 to indicate activity
        self.thread.set(1.0);

        Ok(())
    }

    fn name(&self) -> &str {
        "prom_metrics_collector"
    }
}

#[cfg(feature = "metrics")]
#[async_trait]
impl ExecPlugin for PromMetricsCollectorPlugin {
    /// Parse a quick configuration string for Prometheus metrics collector plugin.
    ///
    /// The exec_str should be in the format: "name=<metric_name>"
    /// Example: "name=my_dns_server"
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != "prom_metrics_collector" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'prom_metrics_collector'",
                prefix
            )));
        }

        // Parse exec_str for name parameter
        let name = if exec_str.trim().is_empty() {
            "default".to_string()
        } else if let Some(name_part) = exec_str.trim().strip_prefix("name=") {
            name_part.to_string()
        } else {
            return Err(crate::Error::Config(format!(
                "Invalid prometheus_metrics_collector configuration: '{}'. Expected format: 'name=<metric_name>'",
                exec_str
            )));
        };

        // Use global registry for metrics to be exposed via /metrics endpoint
        let plugin = Self::with_global_registry(&name)?;

        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;
    use crate::plugin::Context;
    #[cfg(feature = "metrics")]
    use prometheus::Registry;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn test_metrics_collector_increments() {
        let counter = Arc::new(AtomicUsize::new(0));
        let plugin = MetricsCollectorPlugin::new(counter.clone());
        let mut ctx = Context::new(Message::new());
        plugin.execute(&mut ctx).await.unwrap();
        plugin.execute(&mut ctx).await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_metrics_collector_with_latency() {
        let counter = Arc::new(AtomicUsize::new(0));
        let plugin = MetricsCollectorPlugin::new(counter.clone());

        // First query with latency
        let mut ctx1 = Context::new(Message::new());
        ctx1.set_metadata("query_latency_ms", 50.0);
        plugin.execute(&mut ctx1).await.unwrap();

        // Second query with latency
        let mut ctx2 = Context::new(Message::new());
        ctx2.set_metadata("query_latency_ms", 100.0);
        plugin.execute(&mut ctx2).await.unwrap();

        assert_eq!(plugin.count(), 2);
        assert_eq!(plugin.average_latency_ms(), 75.0);
    }

    #[tokio::test]
    async fn test_metrics_collector_queries_per_second() {
        let counter = Arc::new(AtomicUsize::new(0));
        let plugin = MetricsCollectorPlugin::new(counter.clone());

        // Add some queries
        for _ in 0..10 {
            let mut ctx = Context::new(Message::new());
            plugin.execute(&mut ctx).await.unwrap();
        }

        assert_eq!(plugin.count(), 10);
        // QPS should be > 0 since some time has passed
        assert!(plugin.queries_per_second() > 0.0);
    }

    #[tokio::test]
    async fn test_metrics_collector_reset() {
        let counter = Arc::new(AtomicUsize::new(0));
        let plugin = MetricsCollectorPlugin::new(counter.clone());

        // Add some queries with latency
        for i in 0..5 {
            let mut ctx = Context::new(Message::new());
            ctx.set_metadata("query_latency_ms", (i * 20) as f64);
            plugin.execute(&mut ctx).await.unwrap();
        }

        assert_eq!(plugin.count(), 5);
        assert_eq!(plugin.average_latency_ms(), 40.0);

        // Reset metrics
        plugin.reset();

        assert_eq!(plugin.count(), 0);
        assert_eq!(plugin.average_latency_ms(), 0.0);
    }

    #[tokio::test]
    async fn test_metrics_collector_no_latency() {
        let counter = Arc::new(AtomicUsize::new(0));
        let plugin = MetricsCollectorPlugin::new(counter.clone());

        let mut ctx = Context::new(Message::new());
        plugin.execute(&mut ctx).await.unwrap();

        assert_eq!(plugin.count(), 1);
        assert_eq!(plugin.average_latency_ms(), 0.0); // No latency data
    }

    #[test]
    fn test_metrics_collector_quick_setup() {
        let plugin = MetricsCollectorPlugin::quick_setup("metrics_collector", "").unwrap();
        assert_eq!(plugin.name(), "metrics_collector");
    }

    #[test]
    fn test_metrics_collector_quick_setup_wrong_prefix() {
        let result = MetricsCollectorPlugin::quick_setup("wrong_prefix", "");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unsupported prefix")
        );
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_prom_metrics_collector_basic() {
        let registry = Registry::new();
        let plugin = PromMetricsCollectorPlugin::new(Some(&registry), "test_collector").unwrap();

        let mut ctx = Context::new(Message::new());
        plugin.execute(&mut ctx).await.unwrap();

        // Check that metrics were registered
        let metric_families = registry.gather();
        assert!(!metric_families.is_empty());

        // Verify via text encoder to avoid relying on proto internals
        let mut output = String::new();
        let _ = prometheus::TextEncoder::new().encode_utf8(&metric_families, &mut output);
        assert!(output.contains("dns_query_total_test_collector{collector=\"test_collector\"} 1"));
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_prom_metrics_collector_with_latency() {
        let registry = Registry::new();
        let plugin = PromMetricsCollectorPlugin::new(Some(&registry), "test_latency").unwrap();

        let mut ctx = Context::new(Message::new());
        ctx.set_metadata("query_latency_ms", 42.5);
        plugin.execute(&mut ctx).await.unwrap();

        let metric_families = registry.gather();

        // Check latency histogram
        let mut output = String::new();
        let _ = prometheus::TextEncoder::new().encode_utf8(&metric_families, &mut output);
        assert!(output.contains(
            "dns_response_latency_millisecond_test_latency_count{collector=\"test_latency\"} 1"
        ));
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_prom_metrics_collector_with_error() {
        let registry = Registry::new();
        let plugin = PromMetricsCollectorPlugin::new(Some(&registry), "test_error").unwrap();

        let mut ctx = Context::new(Message::new());
        ctx.set_metadata("query_error", true);
        plugin.execute(&mut ctx).await.unwrap();

        let metric_families = registry.gather();

        // Check error counter
        let mut output = String::new();
        let _ = prometheus::TextEncoder::new().encode_utf8(&metric_families, &mut output);
        assert!(output.contains("dns_err_total_test_error{collector=\"test_error\"} 1"));
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_prom_metrics_collector_cache() {
        // First creation
        let registry1 = Registry::new();
        let plugin1 = PromMetricsCollectorPlugin::new(Some(&registry1), "cached_test").unwrap();

        // Second creation with same name should reuse cached metrics from the same registry
        let plugin2 = PromMetricsCollectorPlugin::new(Some(&registry1), "cached_test").unwrap();

        // Both plugins should reference the same metrics from the cache
        let mut ctx = Context::new(Message::new());
        plugin1.execute(&mut ctx).await.unwrap();
        plugin2.execute(&mut ctx).await.unwrap();

        let metrics1 = registry1.gather();

        // Registry should have the metrics
        assert!(!metrics1.is_empty());

        // Check that the counter was incremented twice (both plugins use same metrics)
        let mut output = String::new();
        let _ = prometheus::TextEncoder::new().encode_utf8(&metrics1, &mut output);
        assert!(output.contains("dns_query_total_cached_test{collector=\"cached_test\"} 2"));
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn test_prom_metrics_collector_quick_setup() {
        let plugin =
            PromMetricsCollectorPlugin::quick_setup("prom_metrics_collector", "name=test_quick")
                .unwrap();
        assert_eq!(plugin.name(), "prom_metrics_collector");
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn test_prom_metrics_collector_quick_setup_default_name() {
        let plugin = PromMetricsCollectorPlugin::quick_setup("prom_metrics_collector", "").unwrap();
        assert_eq!(plugin.name(), "prom_metrics_collector");
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn test_prom_metrics_collector_quick_setup_invalid_format() {
        let result =
            PromMetricsCollectorPlugin::quick_setup("prom_metrics_collector", "invalid_format");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid prometheus_metrics_collector configuration")
        );
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn test_prom_metrics_collector_quick_setup_wrong_prefix() {
        let result = PromMetricsCollectorPlugin::quick_setup("wrong_prefix", "name=test");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unsupported prefix")
        );
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn test_prom_metrics_collector_with_global_registry() {
        let plugin = PromMetricsCollectorPlugin::with_global_registry("global_test").unwrap();
        assert_eq!(plugin.name(), "prom_metrics_collector");
    }
}
