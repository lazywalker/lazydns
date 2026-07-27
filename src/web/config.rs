//! WebUI server configuration
//!
//! Configuration for the WebUI HTTP server with real-time streaming.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Default WebUI listen address
fn default_listen_addr() -> String {
    "127.0.0.1:8080".to_string()
}

/// Default event bus capacity
fn default_event_bus_capacity() -> usize {
    1024
}

/// Default metrics window in seconds
fn default_metrics_window_secs() -> u64 {
    300 // 5 minutes
}

/// Default top-N count
fn default_top_n() -> usize {
    10
}

/// Default SSE keepalive interval in seconds
fn default_sse_keepalive_secs() -> u64 {
    30
}

/// Default max SSE connections
fn default_max_sse_connections() -> usize {
    100
}

/// Default max WebSocket connections
fn default_max_ws_connections() -> usize {
    50
}

/// WebUI server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    /// Whether WebUI is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Listen address for the WebUI server
    #[serde(default = "default_listen_addr")]
    pub listen: String,

    /// Path to static files directory (for development)
    #[serde(default)]
    pub static_dir: Option<String>,

    /// Event bus capacity (number of events to buffer)
    #[serde(default = "default_event_bus_capacity")]
    pub event_bus_capacity: usize,

    /// Metrics configuration
    #[serde(default)]
    pub metrics: MetricsConfig,

    /// SSE (Server-Sent Events) configuration
    #[serde(default)]
    pub sse: SseConfig,

    /// WebSocket configuration
    #[serde(default)]
    pub websocket: WebSocketConfig,

    /// Alert configuration
    #[serde(default)]
    pub alerts: AlertConfig,

    /// CORS configuration
    #[serde(default)]
    pub cors: CorsConfig,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: default_listen_addr(),
            static_dir: None,
            event_bus_capacity: default_event_bus_capacity(),
            metrics: MetricsConfig::default(),
            sse: SseConfig::default(),
            websocket: WebSocketConfig::default(),
            alerts: AlertConfig::default(),
            cors: CorsConfig::default(),
        }
    }
}

impl WebConfig {
    /// Parse listen address to SocketAddr
    pub fn socket_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        self.listen.parse()
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        // Validate listen address
        self.socket_addr()
            .map_err(|e| format!("Invalid listen address '{}': {}", self.listen, e))?;

        // Validate event bus capacity
        if self.event_bus_capacity == 0 {
            return Err("event_bus_capacity must be greater than 0".to_string());
        }

        // Validate metrics
        self.metrics.validate()?;

        // Validate SSE
        self.sse.validate()?;

        // Validate WebSocket
        self.websocket.validate()?;

        Ok(())
    }
}

/// Metrics collection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Time window for metrics aggregation (seconds)
    #[serde(default = "default_metrics_window_secs")]
    pub window_secs: u64,

    /// Number of top items to track (domains, clients, and others)
    #[serde(default = "default_top_n")]
    pub top_n: usize,

    /// Whether to track per-client statistics
    #[serde(default = "default_true")]
    pub track_clients: bool,

    /// Whether to track per-domain statistics
    #[serde(default = "default_true")]
    pub track_domains: bool,

    /// Whether to track latency distribution
    #[serde(default = "default_true")]
    pub track_latency: bool,
}

fn default_true() -> bool {
    true
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            window_secs: default_metrics_window_secs(),
            top_n: default_top_n(),
            track_clients: true,
            track_domains: true,
            track_latency: true,
        }
    }
}

impl MetricsConfig {
    fn validate(&self) -> Result<(), String> {
        if self.window_secs == 0 {
            return Err("metrics.window_secs must be greater than 0".to_string());
        }
        if self.top_n == 0 {
            return Err("metrics.top_n must be greater than 0".to_string());
        }
        Ok(())
    }
}

/// SSE (Server-Sent Events) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseConfig {
    /// Maximum number of concurrent SSE connections
    #[serde(default = "default_max_sse_connections")]
    pub max_connections: usize,

    /// Keepalive interval in seconds (sends comment to keep connection alive)
    #[serde(default = "default_sse_keepalive_secs")]
    pub keepalive_secs: u64,

    /// Buffer size for SSE events per connection
    #[serde(default = "default_sse_buffer")]
    pub buffer_size: usize,
}

fn default_sse_buffer() -> usize {
    100
}

impl Default for SseConfig {
    fn default() -> Self {
        Self {
            max_connections: default_max_sse_connections(),
            keepalive_secs: default_sse_keepalive_secs(),
            buffer_size: default_sse_buffer(),
        }
    }
}

impl SseConfig {
    fn validate(&self) -> Result<(), String> {
        if self.max_connections == 0 {
            return Err("sse.max_connections must be greater than 0".to_string());
        }
        if self.keepalive_secs == 0 {
            return Err("sse.keepalive_secs must be greater than 0".to_string());
        }
        Ok(())
    }
}

/// WebSocket configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    /// Maximum number of concurrent WebSocket connections
    #[serde(default = "default_max_ws_connections")]
    pub max_connections: usize,

    /// Heartbeat interval in seconds
    #[serde(default = "default_ws_heartbeat")]
    pub heartbeat_secs: u64,

    /// Connection timeout in seconds (no heartbeat response)
    #[serde(default = "default_ws_timeout")]
    pub timeout_secs: u64,

    /// Maximum message size in bytes
    #[serde(default = "default_ws_max_message_size")]
    pub max_message_size: usize,
}

fn default_ws_heartbeat() -> u64 {
    30
}

fn default_ws_timeout() -> u64 {
    60
}

fn default_ws_max_message_size() -> usize {
    64 * 1024 // 64KB
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            max_connections: default_max_ws_connections(),
            heartbeat_secs: default_ws_heartbeat(),
            timeout_secs: default_ws_timeout(),
            max_message_size: default_ws_max_message_size(),
        }
    }
}

impl WebSocketConfig {
    fn validate(&self) -> Result<(), String> {
        if self.max_connections == 0 {
            return Err("websocket.max_connections must be greater than 0".to_string());
        }
        if self.heartbeat_secs == 0 {
            return Err("websocket.heartbeat_secs must be greater than 0".to_string());
        }
        if self.timeout_secs <= self.heartbeat_secs {
            return Err("websocket.timeout_secs must be greater than heartbeat_secs".to_string());
        }
        Ok(())
    }
}

/// Alert engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    /// Whether alerting is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Alert rules
    #[serde(default)]
    pub rules: Vec<AlertRule>,

    /// Deduplication window in seconds
    #[serde(default = "default_dedup_window")]
    pub dedup_window_secs: u64,

    /// Maximum alerts to keep in memory
    #[serde(default = "default_max_alerts")]
    pub max_alerts: usize,

    /// Webhook configuration
    #[serde(default)]
    pub webhook: Option<WebhookConfig>,
}

fn default_dedup_window() -> u64 {
    300 // 5 minutes
}

fn default_max_alerts() -> usize {
    1000
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rules: Vec::new(),
            dedup_window_secs: default_dedup_window(),
            max_alerts: default_max_alerts(),
            webhook: None,
        }
    }
}

/// Alert rule definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    /// Rule name
    pub name: String,

    /// Condition type
    pub condition: AlertCondition,

    /// Severity level
    #[serde(default)]
    pub severity: AlertSeverity,

    /// Optional message template
    #[serde(default)]
    pub message: Option<String>,
}

/// Alert condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlertCondition {
    /// Trigger on security event type
    SecurityEvent { event_type: String },
    /// Trigger when rate exceeds threshold
    RateThreshold {
        metric: String,
        threshold: f64,
        window_secs: u64,
    },
    /// Trigger on upstream health change
    UpstreamHealth { status: String },
    /// Trigger when error rate exceeds threshold
    ErrorRate { threshold: f64, window_secs: u64 },
}

/// Alert severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    #[default]
    Warning,
    Error,
    Critical,
}

/// Webhook notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Webhook URL
    pub url: String,

    /// Optional authorization header
    #[serde(default)]
    pub auth_header: Option<String>,

    /// Timeout in seconds
    #[serde(default = "default_webhook_timeout")]
    pub timeout_secs: u64,

    /// Retry count on failure
    #[serde(default = "default_webhook_retries")]
    pub retries: u32,
}

fn default_webhook_timeout() -> u64 {
    10
}

fn default_webhook_retries() -> u32 {
    3
}

/// CORS configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorsConfig {
    /// Allowed origins (empty = allow all)
    #[serde(default)]
    pub allowed_origins: Vec<String>,

    /// Whether to allow credentials
    #[serde(default)]
    pub allow_credentials: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WebConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_parse_socket_addr() {
        let config = WebConfig::default();
        let addr = config.socket_addr().unwrap();
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn test_invalid_listen_addr() {
        let config = WebConfig {
            listen: "invalid".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_metrics_config() {
        let mut config = WebConfig::default();
        config.metrics.top_n = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_websocket_timeout_validation() {
        let mut config = WebConfig::default();
        config.websocket.timeout_secs = 10;
        config.websocket.heartbeat_secs = 20;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_alert_rule_deserialization() {
        let yaml = r#"
            name: high_rate_limit
            condition:
              type: security_event
              event_type: rate_limit_exceeded
            severity: warning
        "#;
        let rule: AlertRule = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(rule.name, "high_rate_limit");
        assert_eq!(rule.severity, AlertSeverity::Warning);
    }
}
