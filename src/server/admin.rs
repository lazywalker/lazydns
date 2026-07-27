//! Admin API for runtime management
//!
//! This module provides HTTP endpoints for managing the DNS server at runtime, including:
//! - Cache control and statistics
//! - Configuration reload
//! - Server status monitoring
//!
//! # Architecture
//!
//! The admin server runs as a separate HTTP server alongside the main DNS servers, bound to
//! a configurable address (default: `127.0.0.1:8000`). It allows operators to monitor and
//! manage the server without restarting the entire process.
//!
//! # API Endpoints
//!
//! - `GET /api/cache/stats` - Retrieve cache statistics (size, hits, misses, evictions, hit rate)
//! - `POST /api/cache/control` - Control cache operations (such as clear)
//! - `POST /api/config/reload` - Reload configuration from file
//! - `GET /api/server/stats` - Get current server status and version
//!
//! # Security Considerations
//!
//! The admin API has no built-in authentication. It should only be exposed to trusted networks
//! or protected by network-level access controls (firewall, reverse proxy with auth, and others).
//! Production deployments should:
//! - Bind to localhost or internal network only
//! - Use a reverse proxy with authentication
//! - Restrict network access via firewall rules
//!
//! # Example
//!
//! ```yaml
//! # Configuration example
//! admin:
//!   enabled: true
//!   addr: "127.0.0.1:8000"
//! ```
//!
//! ```bash
//! # Query server status
//! curl http://127.0.0.1:8000/api/server/stats
//!
//! # Get cache statistics
//! curl http://127.0.0.1:8000/api/cache/stats
//!
//! # Clear cache
//! curl -X POST http://127.0.0.1:8000/api/cache/control \
//!   -H "Content-Type: application/json" \
//!   -d '{"action":"clear"}'
//! ```

use crate::config::Config;
use crate::plugin::Registry;
use crate::plugins::CachePlugin;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Cache control request
///
/// Allows clients to perform control operations on the cache system.
///
/// # Fields
///
/// * `action` - The operation to perform. Currently supported actions:
///   - `"clear"` - Clear all entries from the cache
///
/// # Example
///
/// ```json
/// {
///   "action": "clear"
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct CacheControlRequest {
    /// Action to perform: "clear"
    pub action: String,
}

/// Cache statistics response
///
/// Contains detailed statistics about the cache performance and state.
///
/// # Fields
///
/// * `size` - Current number of entries in the cache
/// * `hits` - Total number of cache hits since server start
/// * `misses` - Total number of cache misses since server start
/// * `evictions` - Total number of entries evicted due to LRU policy
/// * `hit_rate` - Cache hit rate as a percentage (0.0-100.0)
///
/// # Example
///
/// ```json
/// {
///   "size": 245,
///   "hits": 5800,
///   "misses": 1200,
///   "evictions": 42,
///   "expirations": 198,
///   "hit_rate": 82.86
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct CacheStatsResponse {
    /// Current cache size
    pub size: usize,
    /// Cache hits
    pub hits: u64,
    /// Cache misses
    pub misses: u64,
    /// Cache evictions
    pub evictions: u64,
    /// Cache expirations
    pub expirations: u64,
    /// Hit rate percentage
    pub hit_rate: f64,
}

/// Config reload request
///
/// Allows clients to trigger configuration reload from a file.
///
/// # Fields
///
/// * `path` - Optional path to the configuration file. If not provided, uses the default
///   location (typically `config.yaml` in the current working directory).
///
/// # Example
///
/// ```json
/// {
///   "path": "/etc/lazydns/config.yaml"
/// }
/// ```
///
/// Or without path (uses default):
///
/// ```json
/// {
///   "path": null
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct ReloadConfigRequest {
    /// Optional path to config file (uses default if not specified)
    pub path: Option<String>,
}

/// Generic success response
///
/// Standard response format for successful operations.
///
/// # Example
///
/// ```json
/// {
///   "message": "Cache cleared successfully"
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct SuccessResponse {
    /// Success message
    pub message: String,
}

/// Generic error response
///
/// Standard response format for error conditions.
///
/// # Example
///
/// ```json
/// {
///   "error": "Cache not configured"
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Error message
    pub error: String,
}

/// Admin server state
///
/// Holds shared references needed by admin API handlers to access configuration
/// and plugins at runtime.
///
/// # Fields
///
/// * `config` - Shared configuration that can be updated via the reload endpoint
/// * `registry` - Plugin registry for accessing runtime plugins like cache
///
/// # Thread Safety
///
/// This struct is `Clone` and thread-safe. Configuration is protected by `RwLock`
/// for safe concurrent access.
#[derive(Clone)]
pub struct AdminState {
    /// Shared configuration
    config: Arc<RwLock<Config>>,
    /// Plugin registry reference for accessing plugins like cache
    registry: Arc<Registry>,
    /// Process start time for uptime reporting
    start_time: Instant,
}

impl AdminState {
    /// Create new admin state
    ///
    /// Initializes the admin server state with shared configuration and plugin registry.
    ///
    /// # Arguments
    ///
    /// * `config` - Shared configuration reference (will be updated by reload endpoint)
    /// * `registry` - Plugin registry reference for accessing plugins
    ///
    /// # Returns
    ///
    /// A new `AdminState` instance.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use lazydns::server::admin::AdminState;
    /// use lazydns::config::Config;
    /// use lazydns::plugin::Registry;
    /// use std::sync::Arc;
    /// use tokio::sync::RwLock;
    ///
    /// let config = Arc::new(RwLock::new(Config::new()));
    /// let registry = Arc::new(Registry::new());
    /// let state = AdminState::new(Arc::clone(&config), Arc::clone(&registry));
    /// ```
    pub fn new(config: Arc<RwLock<Config>>, registry: Arc<Registry>) -> Self {
        Self {
            config,
            registry,
            start_time: Instant::now(),
        }
    }
}

/// Admin API server
///
/// Provides administrative HTTP endpoints for runtime management of the DNS server.
///
/// # Overview
///
/// The `AdminServer` runs on a separate port from the main DNS servers, allowing
/// operators to monitor and manage the server without affecting DNS traffic.
///
/// # Endpoints
///
/// - `GET /api/cache/stats` - Cache statistics
/// - `POST /api/cache/control` - Cache control operations
/// - `POST /api/config/reload` - Reload configuration
/// - `GET /api/server/stats` - Server status and version
///
/// # Example
///
/// ```no_run
/// use lazydns::server::admin::{AdminServer, AdminState};
/// use lazydns::config::Config;
/// use lazydns::plugin::Registry;
/// use std::sync::Arc;
/// use tokio::sync::RwLock;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Arc::new(RwLock::new(Config::new()));
/// let registry = Arc::new(Registry::new());
/// let state = AdminState::new(Arc::clone(&config), Arc::clone(&registry));
/// let server = AdminServer::new("127.0.0.1:8000", state);
/// // server.run().await?;
/// # Ok(())
/// # }
/// ```
pub struct AdminServer {
    addr: String,
    state: Arc<AdminState>,
}

impl AdminServer {
    /// Create a new admin server
    ///
    /// # Arguments
    ///
    /// * `addr` - Address to bind to (such as "127.0.0.1:8000")
    /// * `state` - Admin state containing shared config and plugin registry
    ///
    /// # Returns
    ///
    /// A new `AdminServer` instance ready to be run.
    pub fn new(addr: impl Into<String>, state: AdminState) -> Self {
        Self {
            addr: addr.into(),
            state: Arc::new(state),
        }
    }

    /// Start the admin server, with a channel to signal when startup is complete
    ///
    /// This method binds the HTTP server to the configured address and starts accepting
    /// connections. It runs until the server is shut down externally.
    ///
    /// # Arguments
    ///
    /// * `startup_tx` - Optional channel to signal when the server starts listening.
    ///   This is useful for synchronizing with other startup operations.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the server exits normally. Returns an `Err` if binding
    /// to the address fails or other I/O errors occur.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The address cannot be parsed or is invalid
    /// - The port is already in use
    /// - Insufficient permissions to bind to the port
    /// - Other network I/O errors occur
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use lazydns::server::admin::{AdminServer, AdminState};
    /// # use lazydns::config::Config;
    /// # use lazydns::plugin::Registry;
    /// # use std::sync::Arc;
    /// # use tokio::sync::RwLock;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = Arc::new(RwLock::new(Config::new()));
    /// let registry = Arc::new(Registry::new());
    /// let state = AdminState::new(config, registry);
    /// let server = AdminServer::new("127.0.0.1:8000", state);
    ///
    /// let (tx, rx) = tokio::sync::oneshot::channel();
    /// tokio::spawn(async move {
    ///     server.run_with_signal(Some(tx), None).await.ok();
    /// });
    ///
    /// // Wait for startup
    /// rx.await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run_with_signal(
        self,
        startup_tx: Option<tokio::sync::oneshot::Sender<()>>,
        mut shutdown_rx: Option<tokio::sync::oneshot::Receiver<()>>,
    ) -> Result<(), std::io::Error> {
        let app = Router::new()
            .route("/api/cache/control", post(cache_control))
            .route("/api/cache/stats", get(cache_stats))
            .route("/api/config/reload", post(reload_config))
            .route("/api/server/stats", get(server_stats))
            .with_state(Arc::clone(&self.state));

        let listener = TcpListener::bind(&self.addr).await?;
        info!("Admin API server listening on {}", self.addr);

        // Signal that startup is complete
        if let Some(tx) = startup_tx {
            let _ = tx.send(());
        }

        // Prepare graceful shutdown future. If an external shutdown receiver is
        // provided we await it. Otherwise, listen to OS signals so the admin
        // server can shut itself down similarly to the monitoring server.
        let shutdown_fut = async move {
            if let Some(rx) = shutdown_rx.as_mut() {
                let _ = rx.await;
            } else {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{SignalKind, signal};
                    let mut sigterm = signal(SignalKind::terminate()).unwrap();
                    let mut sighup = signal(SignalKind::hangup()).unwrap();

                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {},
                        _ = sigterm.recv() => {},
                        _ = sighup.recv() => {},
                    }
                }

                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        };

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_fut)
            .await?;

        Ok(())
    }

    /// Start the admin server
    ///
    /// Simple wrapper around [`Self::run_with_signal`] for cases where you don't need
    /// a startup signal.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the server exits normally. Returns an `Err` if binding
    /// to the address fails or other I/O errors occur.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The address cannot be parsed or is invalid
    /// - The port is already in use
    /// - Insufficient permissions to bind to the port
    /// - Other network I/O errors occur
    pub async fn run(self) -> Result<(), std::io::Error> {
        self.run_with_signal(None, None).await
    }
}

/// Helper function to get cache plugin from registry
///
/// Attempts to retrieve and downcast the cache plugin from the registry.
///
/// # Returns
///
/// - `Ok(Arc<dyn Plugin>)` if the cache plugin is found and accessible
/// - `Err(Response)` with appropriate HTTP error response if not found or inaccessible
#[allow(clippy::result_large_err)]
fn get_cache_plugin(registry: &Arc<Registry>) -> Result<Arc<dyn crate::plugin::Plugin>, Response> {
    let cache = registry.get("cache").ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Cache not configured".to_string(),
            }),
        )
            .into_response()
    })?;

    // Verify it's actually a CachePlugin before returning
    cache
        .as_ref()
        .as_any()
        .downcast_ref::<CachePlugin>()
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Cache plugin found but failed to access".to_string(),
                }),
            )
                .into_response()
        })?;

    Ok(cache)
}

/// Handle cache control requests
///
/// Processes control operations on the cache system.
///
/// # Supported Actions
///
/// - `"clear"` - Clear all entries from the cache
///
/// # Responses
///
/// - `200 OK` - Operation completed successfully
/// - `400 Bad Request` - Unknown action requested
/// - `404 Not Found` - Cache not configured or not accessible
/// - `500 Internal Server Error` - Plugin downcast failed
///
/// # Example Request
///
/// ```json
/// {
///   "action": "clear"
/// }
/// ```
async fn cache_control(
    State(state): State<Arc<AdminState>>,
    Json(request): Json<CacheControlRequest>,
) -> Response {
    let cache = match get_cache_plugin(&state.registry) {
        Ok(c) => c,
        Err(e) => return e,
    };

    match request.action.as_str() {
        "clear" => {
            if let Some(cache_plugin) = cache.as_ref().as_any().downcast_ref::<CachePlugin>() {
                cache_plugin.clear();
                info!("Cache cleared via admin API");
                (
                    StatusCode::OK,
                    Json(SuccessResponse {
                        message: "Cache cleared successfully".to_string(),
                    }),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to downcast cache plugin".to_string(),
                    }),
                )
                    .into_response()
            }
        }
        _ => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Unknown action: {}", request.action),
            }),
        )
            .into_response(),
    }
}

/// Get cache statistics
///
/// Retrieves detailed statistics about cache performance.
///
/// # Response
///
/// Returns a `CacheStatsResponse` with the following metrics:
/// - `size`: Current number of cached entries
/// - `hits`: Total cache hits since server start
/// - `misses`: Total cache misses since server start
/// - `evictions`: Entries removed due to LRU eviction
/// - `hit_rate`: Cache hit rate as a percentage
///
/// # HTTP Status Codes
///
/// - `200 OK` - Statistics retrieved successfully
/// - `404 Not Found` - Cache not configured or not accessible
/// - `500 Internal Server Error` - Plugin downcast failed
///
/// # Example Response
///
/// ```json
/// {
///   "size": 245,
///   "hits": 5800,
///   "misses": 1200,
///   "evictions": 42,
///   "expirations": 198,
///   "hit_rate": 82.86
/// }
/// ```
async fn cache_stats(State(state): State<Arc<AdminState>>) -> Response {
    let cache = match get_cache_plugin(&state.registry) {
        Ok(c) => c,
        Err(e) => return e,
    };

    // Now downcast to access CachePlugin methods
    if let Some(cache_plugin) = cache.as_ref().as_any().downcast_ref::<CachePlugin>() {
        let stats = cache_plugin.stats();
        let hits = stats.hits();
        let misses = stats.misses();
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let response = CacheStatsResponse {
            size: cache_plugin.size(),
            hits,
            misses,
            evictions: stats.evictions(),
            expirations: stats.expirations(),
            hit_rate,
        };

        (StatusCode::OK, Json(response)).into_response()
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to downcast cache plugin".to_string(),
            }),
        )
            .into_response()
    }
}

/// Reload configuration
///
/// Reloads the configuration from a file, validates it, and updates the runtime configuration.
///
/// # Request
///
/// Provide an optional path to the configuration file. If not provided, the default
/// location (`config.yaml`) will be used.
///
/// # Behavior
///
/// 1. Loads the configuration file from the specified path
/// 2. Validates the configuration
/// 3. Updates the in-memory configuration if validation passes
/// 4. Does not restart the server or plugins (hot-reload)
///
/// # Response
///
/// - `200 OK` - Configuration reloaded successfully
/// - `400 Bad Request` - Configuration validation failed
/// - `500 Internal Server Error` - Failed to load configuration file
///
/// # Example Requests
///
/// With explicit path:
/// ```json
/// {
///   "path": "/etc/lazydns/config.yaml"
/// }
/// ```
///
/// Using default path:
/// ```json
/// {
///   "path": null
/// }
/// ```
///
/// # Notes
///
/// - The endpoint validates the configuration but does not apply it to running plugins
/// - To fully apply new configuration, restart the server
/// - Configuration changes take effect on next DNS query in some cases (such as timeout changes)
async fn reload_config(
    State(state): State<Arc<AdminState>>,
    Json(request): Json<ReloadConfigRequest>,
) -> Response {
    let path = request.path.unwrap_or_else(|| "config.yaml".to_string());

    match crate::config::loader::load_from_file(&path) {
        Ok(new_config) => match new_config.validate() {
            Ok(_) => {
                let mut config = state.config.write().await;
                *config = new_config;
                info!("Configuration reloaded from {} via admin API", path);

                (
                    StatusCode::OK,
                    Json(SuccessResponse {
                        message: format!("Configuration reloaded from {}", path),
                    }),
                )
                    .into_response()
            }
            Err(e) => {
                warn!("Configuration validation failed: {}", e);
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Configuration validation failed: {}", e),
                    }),
                )
                    .into_response()
            }
        },
        Err(e) => {
            warn!("Failed to load configuration: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to load configuration: {}", e),
                }),
            )
                .into_response()
        }
    }
}

/// Get server status
///
/// Provides basic status information about the server including version and operational status.
///
/// # Response
///
/// Returns a JSON object with:
/// - `status`: Current operational status (always `"running"`)
/// - `version`: Server version from the package manifest
/// - `uptime`: Human readable uptime (such as "21d 15:05:37")
///
/// # HTTP Status Codes
///
/// - `200 OK` - Always returns successfully
///
/// # Example Response
///
/// ```json
/// {
///   "status": "running",
///   "version": "0.2.8",
///   "uptime": "3d 04:12:32"
/// }
/// ```
///
/// # Notes
///
/// This is a lightweight endpoint suitable for health checks. More detailed
/// metrics are available from the monitoring server at `/metrics`.
async fn server_stats(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    #[derive(Serialize)]
    struct StatusResponse {
        status: String,
        version: String,
        uptime: String,
    }

    let elapsed = state.start_time.elapsed();
    let uptime = format_duration(elapsed);

    let response = StatusResponse {
        status: "running".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime,
    };

    (StatusCode::OK, Json(response))
}

/// Format a duration in Linux uptime style: "21d 15:05:37" or "15:05:37"
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    if days > 0 {
        format!("{}d {:02}:{:02}:{:02}", days, hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Struct Serialization Tests

    #[test]
    fn test_cache_control_request_serialization() {
        let req = CacheControlRequest {
            action: "clear".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("clear"));

        // Test deserialization
        let deserialized: CacheControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.action, "clear");
    }

    #[test]
    fn test_cache_stats_response_serialization() {
        let resp = CacheStatsResponse {
            size: 100,
            hits: 80,
            misses: 20,
            evictions: 5,
            expirations: 15,
            hit_rate: 80.0,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("100"));
        assert!(json.contains("80.0"));

        // Test deserialization
        let deserialized: CacheStatsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.size, 100);
        assert_eq!(deserialized.hits, 80);
        assert_eq!(deserialized.hit_rate, 80.0);
    }

    #[test]
    fn test_reload_config_request_with_path() {
        let req = ReloadConfigRequest {
            path: Some("/etc/config.yaml".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("/etc/config.yaml"));

        let deserialized: ReloadConfigRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.path, Some("/etc/config.yaml".to_string()));
    }

    #[test]
    fn test_reload_config_request_without_path() {
        let req = ReloadConfigRequest { path: None };
        let json = serde_json::to_string(&req).unwrap();

        let deserialized: ReloadConfigRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.path, None);
    }

    #[test]
    fn test_success_response_serialization() {
        let resp = SuccessResponse {
            message: "Operation successful".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("Operation successful"));

        let deserialized: SuccessResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.message, "Operation successful");
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse {
            error: "An error occurred".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("An error occurred"));

        let deserialized: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.error, "An error occurred");
    }

    // State Creation Tests

    #[test]
    fn test_admin_state_creation() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let _state = AdminState::new(Arc::clone(&config), Arc::clone(&registry));

        // Verify state was created successfully
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_admin_state_is_clone() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = AdminState::new(Arc::clone(&config), Arc::clone(&registry));

        // Should be cloneable
        let _cloned = state.clone();
    }

    // AdminServer Creation Tests

    #[test]
    fn test_admin_server_creation() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = AdminState::new(Arc::clone(&config), Arc::clone(&registry));

        let _server = AdminServer::new("127.0.0.1:9999", state);
        // Server creation should succeed
    }

    #[test]
    fn test_admin_server_creation_with_shorthand() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = AdminState::new(Arc::clone(&config), Arc::clone(&registry));

        let _server = AdminServer::new(":9999", state);
    }

    // Handler Tests

    #[tokio::test]
    async fn test_server_stats_endpoint() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = Arc::new(AdminState::new(Arc::clone(&config), Arc::clone(&registry)));

        let response = server_stats(State(state)).await.into_response();
        let (parts, body) = response.into_parts();

        assert_eq!(parts.status, StatusCode::OK);

        // Verify body contains version and uptime
        let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(body_str.contains("running"));
        assert!(body_str.contains("version"));
        assert!(body_str.contains("uptime"));
    }

    #[test]
    fn test_format_duration_uptime_style() {
        // Less than a day -> HH:MM:SS
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00:00");
        assert_eq!(format_duration(Duration::from_secs(65)), "00:01:05");
        assert_eq!(format_duration(Duration::from_secs(3661)), "01:01:01");

        // One day -> "1d HH:MM:SS"
        let one_day_one_hour = Duration::from_secs(86400 + 3600);
        assert_eq!(format_duration(one_day_one_hour), "1d 01:00:00");

        // Multiple days -> "Nd HH:MM:SS"
        let days = Duration::from_secs(8 * 86400 + 22 * 3600 + 21 * 60);
        assert_eq!(format_duration(days), "8d 22:21:00");
    }

    #[tokio::test]
    async fn test_cache_stats_with_no_cache_plugin() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = Arc::new(AdminState::new(Arc::clone(&config), Arc::clone(&registry)));

        let response = cache_stats(State(state)).await;
        let (parts, _body) = response.into_parts();

        // Should return 404 when cache not configured
        assert_eq!(parts.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_cache_control_with_no_cache_plugin() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = Arc::new(AdminState::new(Arc::clone(&config), Arc::clone(&registry)));

        let request = CacheControlRequest {
            action: "clear".to_string(),
        };

        let response = cache_control(State(state), Json(request)).await;
        let (parts, _body) = response.into_parts();

        // Should return 404 when cache not configured
        assert_eq!(parts.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_cache_control_with_unknown_action() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = Arc::new(AdminState::new(Arc::clone(&config), Arc::clone(&registry)));

        let request = CacheControlRequest {
            action: "unknown_action".to_string(),
        };

        let response = cache_control(State(state), Json(request)).await;
        let (parts, _body) = response.into_parts();

        // When cache is not configured, returns 404 (takes precedence over bad action)
        // With cache configured, would return 400 for unknown action
        assert_eq!(parts.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_reload_config_with_invalid_path() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = Arc::new(AdminState::new(Arc::clone(&config), Arc::clone(&registry)));

        let request = ReloadConfigRequest {
            path: Some("/nonexistent/path/config.yaml".to_string()),
        };

        let response = reload_config(State(state), Json(request)).await;
        let (parts, _body) = response.into_parts();

        // Should return 500 when file doesn't exist
        assert_eq!(parts.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_reload_config_with_default_path() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(Registry::new());
        let state = Arc::new(AdminState::new(Arc::clone(&config), Arc::clone(&registry)));

        let request = ReloadConfigRequest { path: None };

        let response = reload_config(State(state), Json(request)).await;
        let (parts, _body) = response.into_parts();

        // Should try to load config.yaml (will fail if it doesn't exist)
        // Either 500 (file not found) or 200/400 (file exists)
        assert!(
            parts.status == StatusCode::INTERNAL_SERVER_ERROR
                || parts.status == StatusCode::OK
                || parts.status == StatusCode::BAD_REQUEST
        );
    }

    // Hit Rate Calculation Tests

    #[test]
    fn test_hit_rate_calculation_with_all_hits() {
        let hits = 100u64;
        let misses = 0u64;
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        assert_eq!(hit_rate, 100.0);
    }

    #[test]
    fn test_hit_rate_calculation_with_all_misses() {
        let hits = 0u64;
        let misses = 100u64;
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        assert_eq!(hit_rate, 0.0);
    }

    #[test]
    fn test_hit_rate_calculation_mixed() {
        let hits = 80u64;
        let misses = 20u64;
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        assert!((hit_rate - 80.0).abs() < 0.01);
    }

    #[test]
    fn test_hit_rate_calculation_zero_queries() {
        let hits = 0u64;
        let misses = 0u64;
        let total = hits + misses;
        let hit_rate = if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        assert_eq!(hit_rate, 0.0);
    }

    // Response Type Tests

    #[test]
    fn test_cache_stats_response_with_large_numbers() {
        let resp = CacheStatsResponse {
            size: 1_000_000,
            hits: 10_000_000,
            misses: 2_000_000,
            evictions: 50_000,
            expirations: 100_000,
            hit_rate: 83.33,
        };

        assert_eq!(resp.size, 1_000_000);
        assert_eq!(resp.hits, 10_000_000);
        assert_eq!(resp.misses, 2_000_000);
        assert_eq!(resp.evictions, 50_000);
        assert!((resp.hit_rate - 83.33).abs() < 0.01);
    }

    #[test]
    fn test_cache_stats_response_zero_values() {
        let resp = CacheStatsResponse {
            size: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            expirations: 0,
            hit_rate: 0.0,
        };

        assert_eq!(resp.size, 0);
        assert_eq!(resp.hits, 0);
        assert_eq!(resp.misses, 0);
        assert_eq!(resp.evictions, 0);
        assert_eq!(resp.hit_rate, 0.0);
    }

    // Edge Cases

    #[test]
    fn test_cache_control_request_with_empty_action() {
        let req = CacheControlRequest {
            action: String::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: CacheControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.action, "");
    }

    #[test]
    fn test_cache_control_request_with_special_characters() {
        let req = CacheControlRequest {
            action: "clear-with-dashes_and_underscores".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deserialized: CacheControlRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.action, "clear-with-dashes_and_underscores");
    }

    #[test]
    fn test_success_response_with_long_message() {
        let long_msg = "a".repeat(1000);
        let resp = SuccessResponse {
            message: long_msg.clone(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: SuccessResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.message, long_msg);
    }

    #[test]
    fn test_error_response_with_unicode() {
        let resp = ErrorResponse {
            error: "Error: 无法访问缓存".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.error, "Error: 无法访问缓存");
    }
}
