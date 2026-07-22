//! WebUI server module
//!
//! Provides a real-time dashboard and API for monitoring DNS server operations.
//!
//! # Features
//!
//! - **REST API**: Query metrics, alerts, and configuration
//! - **SSE Streaming**: Real-time query logs and security events
//! - **WebSocket**: Live metrics updates
//! - **Alert Engine**: Configurable alerting with webhook support
//! - **Static File Serving**: Development mode for WebUI frontend
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                    WebUI Server                      │
//! ├─────────────────────────────────────────────────────┤
//! │  Routes:                                             │
//! │  ├── /api/dashboard/overview      GET               │
//! │  ├── /api/metrics/top-domains     GET               │
//! │  ├── /api/metrics/top-clients     GET               │
//! │  ├── /api/metrics/upstream-health GET               │
//! │  ├── /api/alerts/recent           GET               │
//! │  ├── /api/audit/query-logs/stream GET (SSE)         │
//! │  ├── /api/audit/security/stream   GET (SSE)         │
//! │  └── /ws/metrics                  WebSocket         │
//! └─────────────────────────────────────────────────────┘
//! ```

pub mod alerts;
pub mod config;
pub mod metrics;
pub mod routes;
pub mod state;
pub mod upstream_registry;
pub mod websocket;

#[cfg(feature = "web-embed")]
pub mod embedded;

use crate::Result;
use crate::config::Config;
use crate::plugin::Registry;
use crate::plugins::audit::init_event_bus;
#[cfg(feature = "admin")]
use axum::routing::post;
use axum::{Json, Router, routing::get};
use config::WebConfig;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{error, info};

// Re-export upstream registry for easy access
pub use upstream_registry::{
    UpstreamHealthData, UpstreamHealthSnapshot, UpstreamRegistry, get_all_upstream_health,
    register_upstream, unregister_upstream, upstream_registry,
};

pub use state::WebState;

/// WebUI Server
pub struct WebServer {
    config: WebConfig,
    state: Arc<WebState>,
}

impl WebServer {
    /// Create a new WebUI server
    pub async fn new(config: WebConfig) -> Result<Self> {
        // Validate configuration
        config.validate().map_err(crate::Error::Config)?;

        // Initialize event bus if not already initialized (normally done in main.rs)
        // This is a fallback for cases where WebServer is created directly
        if crate::plugins::audit::event_bus().is_none() {
            init_event_bus(config.event_bus_capacity);
        }

        // Create shared state
        let state = Arc::new(WebState::new(&config).await?);

        Ok(Self { config, state })
    }

    /// Create a new WebUI server with admin capabilities
    pub async fn with_admin(
        config: WebConfig,
        registry: Arc<Registry>,
        global_config: Arc<RwLock<Config>>,
    ) -> Result<Self> {
        // Validate configuration
        config.validate().map_err(crate::Error::Config)?;

        // Initialize event bus if not already initialized
        if crate::plugins::audit::event_bus().is_none() {
            init_event_bus(config.event_bus_capacity);
        }

        // Create shared state with admin capabilities
        let mut state = WebState::new(&config).await?;
        state.set_registry(registry);
        state.set_config_arc(global_config);

        Ok(Self {
            config,
            state: Arc::new(state),
        })
    }

    /// Build the router with all routes
    fn build_router(&self) -> Router {
        let state = Arc::clone(&self.state);

        // Build CORS layer
        let cors = if self.config.cors.allowed_origins.is_empty() {
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any)
        } else {
            let origins: Vec<_> = self
                .config
                .cors
                .allowed_origins
                .iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods(Any)
                .allow_headers(Any)
        };

        // Request tracing/logging layer for debugging
        use tower_http::LatencyUnit;
        use tower_http::trace::{DefaultOnRequest, DefaultOnResponse};

        let trace_layer = TraceLayer::new_for_http()
            .make_span_with(|req: &axum::http::Request<axum::body::Body>| {
                tracing::span!(tracing::Level::TRACE, "http_request",
                    method = %req.method(),
                    uri = %req.uri()
                )
            })
            .on_request(DefaultOnRequest::new().level(tracing::Level::TRACE))
            .on_response(
                DefaultOnResponse::new()
                    .level(tracing::Level::TRACE)
                    .latency_unit(LatencyUnit::Millis),
            );

        // API routes
        #[cfg(feature = "admin")]
        let mut api_router = Router::new()
            // Health check endpoint
            .route("/health", get(routes::dashboard::overview))
            // Server features detection
            .route("/features", get(server_features))
            // Dashboard
            .route("/dashboard/overview", get(routes::dashboard::overview))
            .route(
                "/dashboard/cache/stats",
                get(routes::dashboard::cache_stats),
            )
            .route(
                "/dashboard/server/info",
                get(routes::dashboard::server_info),
            )
            .route("/alerts/recent", get(routes::dashboard::recent_alerts))
            // Metrics
            .route("/metrics/top-domains", get(routes::metrics::top_domains))
            .route("/metrics/top-clients", get(routes::metrics::top_clients))
            .route(
                "/metrics/upstream-health",
                get(routes::metrics::upstream_health),
            )
            .route(
                "/metrics/latency",
                get(routes::metrics::latency_distribution),
            )
            .route("/metrics/qps", get(routes::metrics::qps_history))
            // Config dump (read-only)
            .route("/config/dump", get(routes::config_dump::dump))
            // SSE streams
            .route(
                "/audit/query-logs/stream",
                get(routes::audit::query_logs_stream),
            )
            .route(
                "/audit/security-events/stream",
                get(routes::audit::security_events_stream),
            );

        #[cfg(not(feature = "admin"))]
        let api_router = Router::new()
            // Health check endpoint
            .route("/health", get(routes::dashboard::overview))
            // Server features detection
            .route("/features", get(server_features))
            // Dashboard
            .route("/dashboard/overview", get(routes::dashboard::overview))
            .route(
                "/dashboard/cache/stats",
                get(routes::dashboard::cache_stats),
            )
            .route(
                "/dashboard/server/info",
                get(routes::dashboard::server_info),
            )
            .route("/alerts/recent", get(routes::dashboard::recent_alerts))
            // Metrics
            .route("/metrics/top-domains", get(routes::metrics::top_domains))
            .route("/metrics/top-clients", get(routes::metrics::top_clients))
            .route(
                "/metrics/upstream-health",
                get(routes::metrics::upstream_health),
            )
            .route(
                "/metrics/latency",
                get(routes::metrics::latency_distribution),
            )
            .route("/metrics/qps", get(routes::metrics::qps_history))
            // Config dump (read-only)
            .route("/config/dump", get(routes::config_dump::dump))
            // SSE streams
            .route(
                "/audit/query-logs/stream",
                get(routes::audit::query_logs_stream),
            )
            .route(
                "/audit/security-events/stream",
                get(routes::audit::security_events_stream),
            );

        // Admin routes (only when admin feature is enabled)
        #[cfg(feature = "admin")]
        {
            api_router = api_router
                .route("/admin/cache/clear", post(routes::admin::clear_cache))
                .route("/admin/config/reload", post(routes::admin::reload_config))
                // Alert management
                .route(
                    "/admin/alerts/acknowledge-all",
                    post(routes::admin::acknowledge_all_alerts),
                )
                .route(
                    "/admin/alerts/acknowledge/{id}",
                    post(routes::admin::acknowledge_alert),
                )
                .route("/admin/alerts/clear", post(routes::admin::clear_alerts))
                // Log export
                .route("/admin/logs/export", post(routes::admin::export_logs));
        }

        let api_router = api_router
            .layer(trace_layer.clone())
            .with_state(state.clone());

        // WebSocket routes
        let ws_router = Router::new()
            .route("/metrics", get(websocket::handler::metrics_ws))
            .layer(trace_layer.clone())
            .with_state(state.clone());

        // Main router
        let mut router = Router::new()
            .nest("/api", api_router)
            .nest("/ws", ws_router)
            .layer(cors)
            .layer(trace_layer);

        // Static file serving
        // Priority: static_dir config > embedded assets (web-embed feature) > root handler
        if let Some(ref static_dir) = self.config.static_dir {
            info!(path = %static_dir, "Serving static files from directory");
            router = router.fallback_service(
                tower_http::services::ServeDir::new(static_dir)
                    .append_index_html_on_directories(true),
            );
        } else {
            #[cfg(feature = "web-embed")]
            {
                if embedded::has_embedded_assets() {
                    info!("Serving embedded WebUI assets");
                    router = router.merge(embedded::embedded_assets_router());
                } else {
                    info!("No embedded assets found, WebUI will not be served");
                    // Add root handler only if no embedded assets
                    router = router.route("/", get(root_handler));
                }
            }

            #[cfg(not(feature = "web-embed"))]
            {
                info!("No static_dir configured and web-embed feature not enabled");
                // Add root handler when web-embed is not available
                router = router.route("/", get(root_handler));
            }
        }

        router
    }

    /// Run the WebUI server
    pub async fn run(self) -> Result<()> {
        let addr: SocketAddr = self
            .config
            .socket_addr()
            .map_err(|e| crate::Error::Config(format!("Invalid address: {}", e)))?;

        let router = self.build_router();

        info!(address = %addr, "Starting WebUI server");

        // Start metrics collector
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            if let Err(e) = state.metrics_collector().run().await {
                error!(error = %e, "Metrics collector failed");
            }
        });

        // Start alert engine if enabled
        if self.config.alerts.enabled {
            let state = Arc::clone(&self.state);
            tokio::spawn(async move {
                if let Err(e) = state.alert_engine().run().await {
                    error!(error = %e, "Alert engine failed");
                }
            });
        }

        // Run the server
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(crate::Error::Io)?;

        axum::serve(listener, router)
            .await
            .map_err(crate::Error::Io)?;

        Ok(())
    }

    /// Get a reference to the shared state
    pub fn state(&self) -> Arc<WebState> {
        Arc::clone(&self.state)
    }
}

/// Server features detection endpoint
///
/// GET /api/features
async fn server_features() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "admin": cfg!(feature = "admin"),
        "metrics": cfg!(feature = "metrics"),
        "audit": cfg!(feature = "web"),
    }))
}

/// Root handler - returns a simple info page or redirects to dashboard
async fn root_handler() -> axum::response::Html<&'static str> {
    axum::response::Html(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>LazyDNS WebUI</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
            max-width: 600px;
            margin: 50px auto;
            padding: 20px;
            background: #f5f5f5;
        }
        .container {
            background: white;
            padding: 30px;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }
        h1 {
            color: #333;
            margin: 0 0 10px 0;
        }
        .info {
            color: #666;
            margin: 20px 0;
        }
        .links {
            margin: 20px 0;
        }
        a {
            display: inline-block;
            padding: 10px 15px;
            margin: 5px 0;
            background: #007bff;
            color: white;
            text-decoration: none;
            border-radius: 4px;
        }
        a:hover {
            background: #0056b3;
        }
        .divider {
            border-top: 1px solid #eee;
            margin: 20px 0;
        }
        .status {
            background: #d4edda;
            color: #155724;
            padding: 10px;
            border-radius: 4px;
            text-align: center;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>🚀 LazyDNS WebUI</h1>
        <div class="status">✓ Server is running</div>
        <p class="info">A light and fast DNS server implementation in Rust</p>
        
        <div class="divider"></div>
        
        <h3>API Endpoints:</h3>
        <div class="links">
            <a href="/api/dashboard/overview">Dashboard Overview</a>
            <a href="/api/dashboard/cache/stats">Cache Stats</a>
            <a href="/api/dashboard/server/info">Server Info</a>
        </div>
        
        <div class="divider"></div>
        
        <p style="color: #999; font-size: 12px;">
            For detailed API documentation, see the REST API at /api/...
        </p>
    </div>
</body>
</html>"#,
    )
}
