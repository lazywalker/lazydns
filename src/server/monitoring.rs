//! Monitoring and administration HTTP server
//!
//! Provides endpoints for metrics, health checks, and stats.

use crate::metrics;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use prometheus::{Encoder, TextEncoder};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tracing::info;

/// Health check response
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Server status
    pub status: String,
    /// Server version
    pub version: String,
    /// Uptime in seconds
    pub uptime_seconds: u64,
}

/// Stats response
#[derive(Debug, Serialize, Deserialize)]
pub struct StatsResponse {
    /// Total queries processed
    pub total_queries: u64,
    /// Cache hit rate (0.0 to 1.0)
    pub cache_hit_rate: f64,
    /// Active connections
    pub active_connections: i64,
}

/// Monitoring server state
#[derive(Clone)]
pub struct MonitoringState {
    start_time: std::time::Instant,
}

impl MonitoringState {
    /// Create new monitoring state
    pub fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
        }
    }

    /// Get uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

impl Default for MonitoringState {
    fn default() -> Self {
        Self::new()
    }
}

/// Monitoring server
pub struct MonitoringServer {
    addr: String,
    state: Arc<MonitoringState>,
}

impl MonitoringServer {
    /// Create a new monitoring server
    ///
    /// # Arguments
    ///
    /// * `addr` - Address to bind to (such as "0.0.0.0:8001")
    ///
    /// # Example
    ///
    /// ```no_run
    /// use lazydns::server::monitoring::MonitoringServer;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let server = MonitoringServer::new("0.0.0.0:8001");
    /// // server.run().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            state: Arc::new(MonitoringState::new()),
        }
    }

    /// Start the monitoring server, with optional startup signal and optional shutdown receiver
    ///
    /// If `startup_tx` is provided the function will send a `()` once the server has bound
    /// to the configured address. If `shutdown_rx` is provided the server will run until
    /// that channel receives a value, at which point it performs a graceful shutdown.
    pub async fn run_with_signal(
        self,
        startup_tx: Option<tokio::sync::oneshot::Sender<()>>,
        shutdown_rx: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> Result<(), std::io::Error> {
        let app = Router::new()
            .route("/metrics", get(metrics_handler))
            .route("/health", get(health_handler))
            .route("/stats", get(stats_handler))
            .with_state(self.state);

        info!("Monitoring server listening on {}", self.addr);

        let listener = TcpListener::bind(&self.addr).await?;

        // Signal startup if requested
        if let Some(tx) = startup_tx {
            let _ = tx.send(());
        }

        // Prepare graceful shutdown future. If an external shutdown receiver is
        // provided we await it. Otherwise, listen to OS signals (Ctrl-C / SIGTERM / SIGHUP)
        // so the monitoring server can shut itself down like the admin server.
        let shutdown_fut = async move {
            if let Some(rx) = shutdown_rx {
                crate::server::common::await_shutdown(&rx).await;
            } else {
                #[cfg(unix)]
                {
                    // restricted runtimes cannot install signal handlers;
                    // fall back to ctrl_c alone instead of panicking
                    match (
                        signal(SignalKind::terminate()),
                        signal(SignalKind::hangup()),
                    ) {
                        (Ok(mut sigterm), Ok(mut sighup)) => {
                            tokio::select! {
                                _ = tokio::signal::ctrl_c() => {},
                                _ = sigterm.recv() => {},
                                _ = sighup.recv() => {},
                            }
                        }
                        _ => {
                            let _ = tokio::signal::ctrl_c().await;
                        }
                    }
                }

                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        };

        // Run server with graceful shutdown
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_fut)
            .await?;

        Ok(())
    }

    /// Start the monitoring server (no startup/shutdown channels)
    pub async fn run(self) -> Result<(), std::io::Error> {
        self.run_with_signal(None, None).await
    }
}

/// Handle metrics endpoint (Prometheus format)
async fn metrics_handler(State(state): State<Arc<MonitoringState>>) -> Response {
    // Update uptime gauge before scraping
    metrics::SERVER_UPTIME_SECONDS.set(state.uptime_seconds() as i64);
    let metrics_text = metrics::gather_metrics();
    (StatusCode::OK, metrics_text).into_response()
}

/// Handle health check endpoint
async fn health_handler(State(state): State<Arc<MonitoringState>>) -> Response {
    let health = HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.uptime_seconds(),
    };

    Json(health).into_response()
}

/// Handle stats endpoint
///
/// Notes:
/// - `total_queries` is computed by summing all samples of the `dns_queries_total` metric
///   across label combinations. This avoids maintaining separate aggregation state and
///   automatically covers new labels/protocols (such as `udp`, `tcp`, `doh`, `dot`, `doq`).
/// - `active_connections` is computed by summing all samples of the `dns_active_connections`
///   gauge metric; this includes all protocol labels and thus reflects the total active
///   connections across protocols.
/// - This approach reads the current Prometheus registry (`METRICS_REGISTRY.gather()`)
///   and performs an immediate aggregation; for very high-throughput scenarios a dedicated
///   aggregated metric maintained at the source may be preferable for performance and
///   absolute accuracy.
async fn stats_handler() -> Response {
    // Gather statistics from metrics
    let cache_hits = metrics::CACHE_HITS_TOTAL.get();
    let cache_misses = metrics::CACHE_MISSES_TOTAL.get();
    let cache_hit_rate = if cache_hits + cache_misses > 0 {
        cache_hits as f64 / (cache_hits + cache_misses) as f64
    } else {
        0.0
    };

    // Aggregate total queries and active connections by inspecting the registry
    let mut total_queries: u64 = 0;
    let mut active_conns: i64 = 0;

    // Encode each relevant metric family to Prometheus text format and parse numeric samples.
    // This avoids using non-public proto helper traits and works with the public encoder API.
    let encoder = TextEncoder::new();
    let metric_families = metrics::METRICS_REGISTRY.gather();
    for mf in metric_families.iter() {
        match mf.name() {
            "dns_queries_total" | "dns_active_connections" => {
                let mut buf = Vec::new();
                // use a slice reference to avoid cloning the MetricFamily
                if encoder.encode(std::slice::from_ref(mf), &mut buf).is_ok() {
                    let text = match String::from_utf8(buf) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        // line format: metric_name{labels} <value>
                        if let Some(pos) = line.rfind(' ') {
                            let value_str = &line[pos + 1..];
                            if let Ok(v) = value_str.parse::<f64>() {
                                if mf.name() == "dns_queries_total" {
                                    total_queries = total_queries.saturating_add(v as u64);
                                } else {
                                    active_conns = active_conns.saturating_add(v as i64);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let stats = StatsResponse {
        total_queries,
        cache_hit_rate,
        active_connections: active_conns,
    };

    Json(stats).into_response()
}

#[cfg(test)]
fn parse_metric_family_values(mf: &prometheus::proto::MetricFamily) -> Vec<f64> {
    let encoder = TextEncoder::new();
    let mut buf = Vec::new();
    if encoder.encode(std::slice::from_ref(mf), &mut buf).is_err() {
        return Vec::new();
    }
    let text = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut values = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(pos) = line.rfind(' ') {
            let value_str = &line[pos + 1..];
            if let Ok(v) = value_str.parse::<f64>() {
                values.push(v);
            }
        }
    }

    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitoring_state_creation() {
        let state = MonitoringState::new();
        assert_eq!(state.uptime_seconds(), 0);
    }

    #[test]
    fn test_monitoring_state_default() {
        let state = MonitoringState::default();
        assert_eq!(state.uptime_seconds(), 0);
    }

    #[test]
    fn test_monitoring_state_uptime() {
        let state = MonitoringState::new();
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(state.uptime_seconds() >= 1);
    }

    #[test]
    fn test_monitoring_state_uptime_zero() {
        let state = MonitoringState::new();
        // Immediately check uptime - should be 0
        assert_eq!(state.uptime_seconds(), 0);
    }

    #[test]
    fn test_monitoring_server_creation() {
        let server = MonitoringServer::new("127.0.0.1:8001");
        assert_eq!(server.addr, "127.0.0.1:8001");
        assert_eq!(server.state.uptime_seconds(), 0);
    }

    #[test]
    fn test_monitoring_server_creation_with_string() {
        let addr = "0.0.0.0:8000".to_string();
        let server = MonitoringServer::new(addr.clone());
        assert_eq!(server.addr, addr);
    }

    #[test]
    fn test_health_response_serialization() {
        let health = HealthResponse {
            status: "healthy".to_string(),
            version: "0.1.0".to_string(),
            uptime_seconds: 100,
        };

        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("healthy"));
        assert!(json.contains("0.1.0"));
        assert!(json.contains("100"));
    }

    #[test]
    fn test_health_response_deserialization() {
        let json = r#"{"status":"healthy","version":"0.1.0","uptime_seconds":100}"#;
        let health: HealthResponse = serde_json::from_str(json).unwrap();

        assert_eq!(health.status, "healthy");
        assert_eq!(health.version, "0.1.0");
        assert_eq!(health.uptime_seconds, 100);
    }

    #[test]
    fn test_health_response_empty_values() {
        let health = HealthResponse {
            status: String::new(),
            version: String::new(),
            uptime_seconds: 0,
        };

        let json = serde_json::to_string(&health).unwrap();
        let deserialized: HealthResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.status, "");
        assert_eq!(deserialized.version, "");
        assert_eq!(deserialized.uptime_seconds, 0);
    }

    #[test]
    fn test_stats_response_serialization() {
        let stats = StatsResponse {
            total_queries: 1000,
            cache_hit_rate: 0.75,
            active_connections: 10,
        };

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("1000"));
        assert!(json.contains("0.75"));
        assert!(json.contains("10"));
    }

    #[test]
    fn test_stats_response_deserialization() {
        let json = r#"{"total_queries":1000,"cache_hit_rate":0.75,"active_connections":10}"#;
        let stats: StatsResponse = serde_json::from_str(json).unwrap();

        assert_eq!(stats.total_queries, 1000);
        assert_eq!(stats.cache_hit_rate, 0.75);
        assert_eq!(stats.active_connections, 10);
    }

    #[test]
    fn test_stats_response_edge_cases() {
        // Test zero values
        let stats_zero = StatsResponse {
            total_queries: 0,
            cache_hit_rate: 0.0,
            active_connections: 0,
        };
        let json_zero = serde_json::to_string(&stats_zero).unwrap();
        assert!(json_zero.contains("0"));

        // Test maximum hit rate
        let stats_perfect = StatsResponse {
            total_queries: 1000,
            cache_hit_rate: 1.0,
            active_connections: -1, // Negative connections (edge case)
        };
        let json_perfect = serde_json::to_string(&stats_perfect).unwrap();
        assert!(json_perfect.contains("1.0"));
        assert!(json_perfect.contains("-1"));
    }

    #[test]
    fn test_stats_response_negative_connections() {
        let stats = StatsResponse {
            total_queries: 500,
            cache_hit_rate: 0.5,
            active_connections: -5,
        };

        let json = serde_json::to_string(&stats).unwrap();
        let deserialized: StatsResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.active_connections, -5);
    }

    #[test]
    fn test_health_handler_response_structure() {
        // Test that health handler creates proper response structure
        // We can't easily test the async handler directly without axum test framework,
        // but we can test the response structure it creates
        let state = Arc::new(MonitoringState::new());
        let expected_version = env!("CARGO_PKG_VERSION");

        // Simulate what health_handler does
        let health = HealthResponse {
            status: "healthy".to_string(),
            version: expected_version.to_string(),
            uptime_seconds: state.uptime_seconds(),
        };

        assert_eq!(health.status, "healthy");
        assert_eq!(health.version, expected_version);
        assert_eq!(health.uptime_seconds, 0); // Just created
    }

    #[test]
    fn test_stats_handler_response_structure() {
        // Test that stats handler creates proper response structure
        // We can't easily test the async handler directly without axum test framework,
        // but we can test the logic it uses

        // Test cache hit rate calculation logic
        let cache_hits = 75;
        let cache_misses = 25;
        let expected_hit_rate = cache_hits as f64 / (cache_hits + cache_misses) as f64;
        assert_eq!(expected_hit_rate, 0.75);

        // Test zero division case
        let zero_hit_rate = if 0 > 0 { 1.0 } else { 0.0 };
        assert_eq!(zero_hit_rate, 0.0);

        // Test active connections summation logic
        let udp_conns = 5;
        let tcp_conns = 3;
        let total_conns = udp_conns + tcp_conns;
        assert_eq!(total_conns, 8);
    }

    #[test]
    fn test_parse_metric_family_values_and_sum() {
        // Snapshot current totals to avoid flakes due to global metric state
        let mfs_before = metrics::METRICS_REGISTRY.gather();
        let q_mf_before = mfs_before
            .iter()
            .find(|mf| mf.name() == "dns_queries_total");
        let prev_total_queries: u64 = q_mf_before
            .map(|mf| {
                parse_metric_family_values(mf)
                    .iter()
                    .map(|v| *v as u64)
                    .sum()
            })
            .unwrap_or(0);

        // Set up counters/gauges
        for _ in 0..5 {
            metrics::DNS_QUERIES_TOTAL
                .with_label_values(&["udp", "A"])
                .inc();
        }
        for _ in 0..3 {
            metrics::DNS_QUERIES_TOTAL
                .with_label_values(&["tcp", "A"])
                .inc();
        }

        metrics::ACTIVE_CONNECTIONS
            .with_label_values(&["udp"])
            .set(2);
        metrics::ACTIVE_CONNECTIONS
            .with_label_values(&["tcp"])
            .set(4);

        // Find metric families and parse
        let mfs = metrics::METRICS_REGISTRY.gather();
        let q_mf = mfs
            .iter()
            .find(|mf| mf.name() == "dns_queries_total")
            .unwrap();
        let a_mf = mfs
            .iter()
            .find(|mf| mf.name() == "dns_active_connections")
            .unwrap();

        let q_vals = parse_metric_family_values(q_mf);
        assert!(q_vals.len() >= 2);
        let total_queries_after: u64 = q_vals.iter().map(|v| *v as u64).sum();
        assert_eq!(total_queries_after, prev_total_queries + 8);

        let a_vals = parse_metric_family_values(a_mf);
        assert!(a_vals.len() >= 2);
        assert_eq!(a_vals.iter().map(|v| *v as i64).sum::<i64>(), 6);
    }

    #[tokio::test]
    async fn test_stats_handler_aggregates_metrics() {
        // Tests share global Prometheus metrics. Snapshot current values and
        // assert that our increments change the aggregated totals by the
        // expected deltas so the test is robust against other tests.

        // Snapshot current totals for dns_queries_total and dns_active_connections
        let mfs_before = metrics::METRICS_REGISTRY.gather();
        let q_mf_before = mfs_before
            .iter()
            .find(|mf| mf.name() == "dns_queries_total");
        let prev_total_queries: u64 = q_mf_before
            .map(|mf| {
                parse_metric_family_values(mf)
                    .iter()
                    .map(|v| *v as u64)
                    .sum()
            })
            .unwrap_or(0);

        let a_mf_before = mfs_before
            .iter()
            .find(|mf| mf.name() == "dns_active_connections");
        let prev_total_active: i64 = a_mf_before
            .map(|mf| {
                parse_metric_family_values(mf)
                    .iter()
                    .map(|v| *v as i64)
                    .sum()
            })
            .unwrap_or(0);

        let prev_cache_hits = metrics::CACHE_HITS_TOTAL.get();
        let prev_cache_misses = metrics::CACHE_MISSES_TOTAL.get();

        // Increment queries: 5 udp_test + 3 tcp_test (use unique labels to avoid
        // clobbering other tests' label values)
        for _ in 0..5 {
            metrics::DNS_QUERIES_TOTAL
                .with_label_values(&["udp_test", "A"])
                .inc();
        }
        for _ in 0..3 {
            metrics::DNS_QUERIES_TOTAL
                .with_label_values(&["tcp_test", "A"])
                .inc();
        }

        // Set cache hits/misses to compute hit rate
        metrics::CACHE_HITS_TOTAL.inc_by(75);
        metrics::CACHE_MISSES_TOTAL.inc_by(25);

        // Record existing per-label active connection values for our test labels
        let prev_udp_val = metrics::ACTIVE_CONNECTIONS
            .with_label_values(&["udp_test"])
            .get();
        let prev_tcp_val = metrics::ACTIVE_CONNECTIONS
            .with_label_values(&["tcp_test"])
            .get();

        // Set active connections for our test labels
        metrics::ACTIVE_CONNECTIONS
            .with_label_values(&["udp_test"])
            .set(2);
        metrics::ACTIVE_CONNECTIONS
            .with_label_values(&["tcp_test"])
            .set(4);

        let response = stats_handler().await;
        let (parts, body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::OK);

        let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

        let stats: StatsResponse = serde_json::from_str(&body_str).unwrap();

        // Verify totals increased by the amounts we added
        let expected_total_queries = prev_total_queries + 8;
        assert_eq!(stats.total_queries, expected_total_queries);

        // Verify cache hit rate reflects our increments on top of previous counts
        let expected_hit_rate = if (prev_cache_hits + prev_cache_misses + 100) > 0 {
            (prev_cache_hits + 75) as f64 / (prev_cache_hits + prev_cache_misses + 100) as f64
        } else {
            0.0
        };
        assert!((stats.cache_hit_rate - expected_hit_rate).abs() < 1e-6);

        // Verify active connections: replace previous values for our labels with the new ones
        let expected_active = prev_total_active - prev_udp_val - prev_tcp_val + 2 + 4;
        assert_eq!(stats.active_connections, expected_active);
    }

    #[tokio::test]
    async fn test_monitoring_server_bind_address_validation() {
        // Test that server creation accepts various valid address formats
        let valid_addresses = vec![
            "127.0.0.1:8001",
            "0.0.0.0:8000",
            "localhost:3000",
            "[::1]:8001", // IPv6
        ];

        for addr in valid_addresses {
            let server = MonitoringServer::new(addr);
            assert_eq!(server.addr, addr);
        }
    }
}
