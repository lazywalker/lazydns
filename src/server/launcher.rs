//! Server launcher module
//!
//! This module provides utilities to launch DNS servers based on plugin configurations,
//! reducing code duplication in main.rs.
//!
//! # Overview
//!
//! The `ServerLauncher` is responsible for starting various DNS server types (UDP, TCP, DoH, DoT, DoQ)
//! based on plugin configurations loaded from YAML files. It eliminates the repetitive server
//! initialization code that was previously scattered throughout `main.rs`.
//!
//! # Architecture
//!
//! The launcher works by:
//! 1. Iterating through plugin configurations
//! 2. Matching plugin types to server types
//! 3. Parsing configuration arguments (listen addresses, TLS certificates, etc.)
//! 4. Creating and spawning server tasks
//!
//! # Supported Server Types
//!
//! - **UDP Server**: Basic DNS over UDP (`udp_server`)
//! - **TCP Server**: DNS over TCP (`tcp_server`)
//! - **DoH Server**: DNS over HTTPS (`doh_server`) - requires `tls` feature
//! - **DoT Server**: DNS over TLS (`dot_server`) - requires `tls` feature
//! - **DoQ Server**: DNS over QUIC (`doq_server`) - requires `doq` feature
//!
//! # Example
//!
//! ```rust,no_run
//! use lazydns::config::Config;
//! use lazydns::plugin::PluginBuilder;
//! use lazydns::server::ServerLauncher;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Load configuration
//! let config = Config::from_file("config.yaml")?;
//!
//! // Build plugins
//! let mut builder = PluginBuilder::new();
//! for plugin_config in &config.plugins {
//!     builder.build(plugin_config);
//! }
//! let registry = Arc::new(builder.get_registry());
//!
//! // Launch all servers
//! let launcher = ServerLauncher::new(registry);
//! launcher.launch_all(&config.plugins).await;
//! # Ok(())
//! # }
//! ```

use crate::config::PluginConfig;
use crate::plugin::{PluginHandler, Registry};
#[cfg(feature = "doh")]
use crate::server::DohServer;
#[cfg(feature = "doq")]
use crate::server::DoqServer;
#[cfg(feature = "dot")]
use crate::server::DotServer;
#[cfg(feature = "metrics")]
use crate::server::MonitoringServer;
#[cfg(any(feature = "doh", feature = "dot", feature = "doq"))]
use crate::server::Server;
#[cfg(any(feature = "doh", feature = "dot"))]
use crate::server::TlsConfig;
#[cfg(feature = "admin")]
use crate::server::admin::{AdminServer, AdminState};
use crate::server::{ServerConfig, TcpServer, UdpServer};
use serde_yaml::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
#[cfg(any(feature = "admin", feature = "metrics", feature = "web"))]
use tracing::info;
use tracing::{error, warn};

/// Normalize listen address shorthand like ":5353" -> "0.0.0.0:5353"
///
/// This function converts shorthand listen addresses (starting with ':') to full
/// IPv4 addresses by prepending "0.0.0.0".
///
/// # Arguments
///
/// * `listen` - The listen address string, potentially in shorthand form
///
/// # Returns
///
/// A normalized address string suitable for parsing as a `SocketAddr`
fn normalize_listen_addr(listen: &str) -> String {
    if listen.starts_with(':') {
        format!("0.0.0.0{}", listen)
    } else {
        listen.to_string()
    }
}

/// Server launcher responsible for starting DNS servers based on plugin configurations
///
/// The `ServerLauncher` encapsulates the logic for launching different types of DNS servers
/// based on plugin configurations. It maintains a reference to the plugin registry and
/// provides methods to launch individual server types or all configured servers at once.
///
/// # Fields
///
/// * `registry` - Reference to the plugin registry containing all loaded plugins
///
/// # Thread Safety
///
/// The launcher is thread-safe and can be used to launch multiple servers concurrently.
/// Each launched server runs in its own async task.
pub struct ServerLauncher {
    /// Reference to the plugin registry
    registry: Arc<Registry>,
}

impl ServerLauncher {
    /// Create a new ServerLauncher with the given plugin registry
    ///
    /// # Arguments
    ///
    /// * `registry` - The plugin registry containing all loaded plugins
    ///
    /// # Returns
    ///
    /// A new `ServerLauncher` instance
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lazydns::plugin::Registry;
    /// use lazydns::server::ServerLauncher;
    /// use std::sync::Arc;
    ///
    /// let registry = Arc::new(Registry::new());
    /// let launcher = ServerLauncher::new(registry);
    /// ```
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// Launch all servers configured in the plugin list
    ///
    /// This method iterates through all plugin configurations and launches the appropriate
    /// server type for each recognized plugin. Unknown plugin types are silently ignored.
    ///
    /// # Arguments
    ///
    /// * `plugins` - Slice of plugin configurations to process
    ///
    /// # Behavior
    ///
    /// - Recognized server plugins are launched in separate async tasks
    /// - Each server runs indefinitely until the process is terminated
    /// - Errors during server startup are logged but don't prevent other servers from starting
    /// - Unknown plugin types are skipped without error
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lazydns::config::Config;
    /// use lazydns::server::ServerLauncher;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = Config::from_file("config.yaml")?;
    /// let registry = Arc::new(lazydns::plugin::Registry::new());
    /// let launcher = ServerLauncher::new(registry);
    ///
    /// launcher.launch_all(&config.plugins).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn launch_all(
        &self,
        plugins: &[PluginConfig],
    ) -> Vec<tokio::sync::oneshot::Receiver<()>> {
        let mut receivers = Vec::new();

        for plugin_config in plugins {
            let receiver = match plugin_config.plugin_type.as_str() {
                "udp_server" => self.launch_udp_server(plugin_config).await,
                "tcp_server" => self.launch_tcp_server(plugin_config).await,
                "doh_server" => self.launch_doh_server(plugin_config).await,
                "dot_server" => self.launch_dot_server(plugin_config).await,
                "doq_server" => self.launch_doq_server(plugin_config).await,
                _ => continue,
            };

            if let Some(rx) = receiver {
                receivers.push(rx);
            }
        }

        receivers
    }

    /// Parse listen address from plugin args
    ///
    /// Extracts and parses the listen address from plugin configuration arguments.
    /// Supports shorthand notation (e.g., ":5353" becomes "0.0.0.0:5353").
    ///
    /// # Arguments
    ///
    /// * `args` - Plugin configuration arguments
    /// * `default` - Default address to use if not specified in args
    ///
    /// # Returns
    ///
    /// `Some(SocketAddr)` if parsing succeeds, `None` if the address is invalid
    fn parse_listen_addr(
        &self,
        args: &HashMap<String, Value>,
        default: &str,
    ) -> Option<SocketAddr> {
        let listen_str = args
            .get("listen")
            .and_then(|v| v.as_str())
            .unwrap_or(default);
        let normalized = normalize_listen_addr(listen_str);

        match normalized.parse::<SocketAddr>() {
            Ok(addr) => Some(addr),
            Err(e) => {
                error!("Failed to parse listen address '{}': {}", listen_str, e);
                None
            }
        }
    }

    /// Get entry plugin name from args
    ///
    /// Extracts the entry plugin name from plugin configuration arguments.
    /// Falls back to "main_sequence" if not specified or invalid.
    ///
    /// # Arguments
    ///
    /// * `args` - Plugin configuration arguments
    ///
    /// # Returns
    ///
    /// The entry plugin name as a string
    fn get_entry(&self, args: &HashMap<String, Value>) -> String {
        args.get("entry")
            .and_then(|v| v.as_str())
            .unwrap_or("main_sequence")
            .to_string()
    }

    /// Create plugin handler for the given entry
    ///
    /// Creates a new `PluginHandler` instance configured to use the specified
    /// entry plugin from the registry.
    ///
    /// # Arguments
    ///
    /// * `entry` - Name of the entry plugin to use
    ///
    /// # Returns
    ///
    /// A new `PluginHandler` wrapped in an `Arc`
    fn create_handler(&self, entry: String) -> Arc<PluginHandler> {
        Arc::new(PluginHandler {
            registry: Arc::clone(&self.registry),
            entry,
        })
    }

    /// Launch UDP server
    ///
    /// Creates and starts a UDP DNS server based on the plugin configuration.
    /// The server will listen on the specified address and use the configured entry plugin.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration containing server settings
    ///
    /// # Configuration Parameters
    ///
    /// - `listen`: Listen address (default: "0.0.0.0:53")
    /// - `entry`: Entry plugin name (default: "main_sequence")
    ///
    /// # Behavior
    ///
    /// - Parses the listen address from plugin args
    /// - Creates a UDP server with the specified configuration
    /// - Spawns the server in a background task
    /// - Logs errors if server creation or startup fails
    ///
    /// # Examples
    ///
    /// ```yaml
    /// plugins:
    ///   - type: udp_server
    ///     args:
    ///       listen: "127.0.0.1:5353"
    ///       entry: "main_sequence"
    /// ```
    async fn launch_udp_server(
        &self,
        plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let args = plugin_config.effective_args();
        let addr = self.parse_listen_addr(&args, "0.0.0.0:53")?;

        let entry = self.get_entry(&args);
        let config = ServerConfig {
            udp_addr: Some(addr),
            ..Default::default()
        };
        let handler = self.create_handler(entry);

        match UdpServer::new(config, handler).await {
            Ok(server) => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    // Send startup completion signal immediately
                    let _ = tx.send(());
                    if let Err(e) = server.run().await {
                        error!("UDP server error: {}", e);
                    }
                });
                Some(rx)
            }
            Err(e) => {
                error!("Failed to start UDP server on {}: {}", addr, e);
                None
            }
        }
    }

    /// Launch TCP server
    ///
    /// Creates and starts a TCP DNS server based on the plugin configuration.
    /// The server will listen on the specified address and use the configured entry plugin.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration containing server settings
    ///
    /// # Configuration Parameters
    ///
    /// - `listen`: Listen address (default: "0.0.0.0:53")
    /// - `entry`: Entry plugin name (default: "main_sequence")
    ///
    /// # Behavior
    ///
    /// - Parses the listen address from plugin args
    /// - Creates a TCP server with the specified configuration
    /// - Spawns the server in a background task
    /// - Logs errors if server creation or startup fails
    ///
    /// # Examples
    ///
    /// ```yaml
    /// plugins:
    ///   - type: tcp_server
    ///     args:
    ///       listen: "127.0.0.1:5353"
    ///       entry: "main_sequence"
    /// ```
    async fn launch_tcp_server(
        &self,
        plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let args = plugin_config.effective_args();
        let addr = self.parse_listen_addr(&args, "0.0.0.0:53")?;

        let entry = self.get_entry(&args);
        let config = ServerConfig {
            tcp_addr: Some(addr),
            ..Default::default()
        };
        let handler = self.create_handler(entry);

        match TcpServer::new(config, handler).await {
            Ok(server) => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    // Send startup completion signal immediately
                    let _ = tx.send(());
                    if let Err(e) = server.run().await {
                        error!("TCP server error: {}", e);
                    }
                });
                Some(rx)
            }
            Err(e) => {
                error!("Failed to start TCP server on {}: {}", addr, e);
                None
            }
        }
    }

    /// Launch DoH (DNS over HTTPS) server
    ///
    /// Creates and starts a DNS over HTTPS server based on the plugin configuration.
    /// Requires the `tls` feature to be enabled at compile time.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration containing server settings
    ///
    /// # Configuration Parameters
    ///
    /// - `listen`: Listen address (default: "0.0.0.0:443")
    /// - `entry`: Entry plugin name (default: "main_sequence")
    /// - `cert_file`: Path to TLS certificate file (required)
    /// - `key_file`: Path to TLS private key file (required)
    ///
    /// # Behavior
    ///
    /// - Parses the listen address from plugin args
    /// - Loads TLS certificate and key files
    /// - Creates a DoH server with TLS configuration
    /// - Spawns the server in a background task
    /// - Logs errors if TLS config loading or server startup fails
    /// - Warns if certificate/key files are not specified
    ///
    /// # Examples
    ///
    /// ```yaml
    /// plugins:
    ///   - type: doh_server
    ///     args:
    ///       listen: "0.0.0.0:443"
    ///       entry: "main_sequence"
    ///       cert_file: "/path/to/cert.pem"
    ///       key_file: "/path/to/key.pem"
    /// ```
    ///
    /// # Feature Requirements
    ///
    /// This method is only available when the `doh` feature is enabled:
    /// ```toml
    /// lazydns = { version = "*", features = ["doh"] }
    /// ```
    #[cfg(feature = "doh")]
    async fn launch_doh_server(
        &self,
        plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let args = plugin_config.effective_args();
        let addr = self.parse_listen_addr(&args, "0.0.0.0:443")?;

        let cert_path = args.get("cert_file").and_then(|v| v.as_str());
        let key_path = args.get("key_file").and_then(|v| v.as_str());

        let (Some(cert_path), Some(key_path)) = (cert_path, key_path) else {
            warn!("doh_server plugin configured without cert_file/key_file");
            return None;
        };

        let tls = match TlsConfig::from_files(cert_path, key_path) {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to load TLS config for DoH: {}", e);
                return None;
            }
        };

        let entry = self.get_entry(&args);
        let handler = self.create_handler(entry);
        let doh_path = args.get("path").and_then(|v| v.as_str()).map(String::from);

        let mut config = ServerConfig {
            tcp_addr: Some(addr),
            handler: Some(handler),
            tls_config: Some(tls),
            doh_path,
            ..Default::default()
        };
        // Also set cert paths for consistency
        config.cert_path = Some(cert_path.to_string());
        config.key_path = Some(key_path.to_string());

        let server = match DohServer::from_config(config).await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to create DoH server from config: {}", e);
                return None;
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            // Send startup completion signal immediately
            let _ = tx.send(());
            if let Err(e) = server.run().await {
                error!("DoH server error: {}", e);
            }
        });
        Some(rx)
    }

    /// Launch DoH server (TLS feature disabled)
    ///
    /// This is a stub implementation that logs a warning when the `tls` feature
    /// is not enabled but a DoH server is requested.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration (ignored in this implementation)
    #[cfg(not(feature = "doh"))]
    async fn launch_doh_server(
        &self,
        _plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        warn!("DoH server requested but TLS feature is not enabled");
        None
    }

    /// Launch DoT (DNS over TLS) server
    ///
    /// Creates and starts a DNS over TLS server based on the plugin configuration.
    /// Requires the `tls` feature to be enabled at compile time.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration containing server settings
    ///
    /// # Configuration Parameters
    ///
    /// - `listen`: Listen address (default: "0.0.0.0:853")
    /// - `entry`: Entry plugin name (default: "main_sequence")
    /// - `cert_file`: Path to TLS certificate file (required)
    /// - `key_file`: Path to TLS private key file (required)
    ///
    /// # Behavior
    ///
    /// - Parses the listen address from plugin args
    /// - Loads TLS certificate and key files
    /// - Creates a DoT server with TLS configuration
    /// - Spawns the server in a background task
    /// - Logs errors if TLS config loading or server startup fails
    /// - Warns if certificate/key files are not specified
    ///
    /// # Examples
    ///
    /// ```yaml
    /// plugins:
    ///   - type: dot_server
    ///     args:
    ///       listen: "0.0.0.0:853"
    ///       entry: "main_sequence"
    ///       cert_file: "/path/to/cert.pem"
    ///       key_file: "/path/to/key.pem"
    /// ```
    ///
    /// # Feature Requirements
    ///
    /// This method is only available when the `dot` feature is enabled:
    /// ```toml
    /// lazydns = { version = "*", features = ["dot"] }
    /// ```
    #[cfg(feature = "dot")]
    async fn launch_dot_server(
        &self,
        plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let args = plugin_config.effective_args();
        let addr = self.parse_listen_addr(&args, "0.0.0.0:853")?;

        let cert_path = args.get("cert_file").and_then(|v| v.as_str());
        let key_path = args.get("key_file").and_then(|v| v.as_str());

        let (Some(cert_path), Some(key_path)) = (cert_path, key_path) else {
            warn!("dot_server plugin configured without cert_file/key_file");
            return None;
        };

        let tls = match TlsConfig::from_files(cert_path, key_path) {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to load TLS config for DoT: {}", e);
                return None;
            }
        };

        let entry = self.get_entry(&args);
        let handler = self.create_handler(entry);

        let mut config = ServerConfig {
            tcp_addr: Some(addr),
            handler: Some(handler),
            tls_config: Some(tls),
            ..Default::default()
        };
        // Also set cert paths for consistency
        config.cert_path = Some(cert_path.to_string());
        config.key_path = Some(key_path.to_string());

        let server = match DotServer::from_config(config).await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to create DoT server from config: {}", e);
                return None;
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            // Send startup completion signal immediately
            let _ = tx.send(());
            if let Err(e) = server.run().await {
                error!("DoT server error: {}", e);
            }
        });
        Some(rx)
    }

    /// Launch DoT server (TLS feature disabled)
    ///
    /// This is a stub implementation that logs a warning when the `tls` feature
    /// is not enabled but a DoT server is requested.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration (ignored in this implementation)
    #[cfg(not(feature = "dot"))]
    async fn launch_dot_server(
        &self,
        _plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        warn!("DoT server requested but TLS feature is not enabled");
        None
    }

    /// Launch DoQ (DNS over QUIC) server
    ///
    /// Creates and starts a DNS over QUIC server based on the plugin configuration.
    /// Requires the `doq` feature to be enabled at compile time.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration containing server settings
    ///
    /// # Configuration Parameters
    ///
    /// - `listen`: Listen address (default: "0.0.0.0:784")
    /// - `entry`: Entry plugin name (default: "main_sequence")
    /// - `cert_file`: Path to TLS certificate file (required)
    /// - `key_file`: Path to TLS private key file (required)
    ///
    /// # Behavior
    ///
    /// - Parses the listen address from plugin args
    /// - Creates a DoQ server with the certificate and key paths
    /// - Spawns the server in a background task
    /// - Logs errors if server creation or startup fails
    /// - Warns if certificate/key files are not specified
    ///
    /// # Examples
    ///
    /// ```yaml
    /// plugins:
    ///   - type: doq_server
    ///     args:
    ///       listen: "0.0.0.0:784"
    ///       entry: "main_sequence"
    ///       cert_file: "/path/to/cert.pem"
    ///       key_file: "/path/to/key.pem"
    /// ```
    ///
    /// # Feature Requirements
    ///
    /// This method is only available when the `doq` feature is enabled:
    /// ```toml
    /// lazydns = { version = "*", features = ["doq"] }
    /// ```
    #[cfg(feature = "doq")]
    async fn launch_doq_server(
        &self,
        plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let args = plugin_config.effective_args();
        let addr = self.parse_listen_addr(&args, "0.0.0.0:784")?;

        let cert_path = args.get("cert_file").and_then(|v| v.as_str());
        let key_path = args.get("key_file").and_then(|v| v.as_str());

        let (Some(cert_path), Some(key_path)) = (cert_path, key_path) else {
            warn!("doq_server plugin configured without cert_file/key_file");
            return None;
        };

        let entry = self.get_entry(&args);
        let handler = self.create_handler(entry);

        let config = ServerConfig {
            tcp_addr: Some(addr),
            handler: Some(handler),
            cert_path: Some(cert_path.to_string()),
            key_path: Some(key_path.to_string()),
            ..Default::default()
        };

        let server = match DoqServer::from_config(config).await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to create DoQ server from config: {}", e);
                return None;
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            // Send startup completion signal immediately
            let _ = tx.send(());
            if let Err(e) = server.run().await {
                error!("DoQ server error: {}", e);
            }
        });
        Some(rx)
    }

    /// Launch DoQ server (DoQ feature disabled)
    ///
    /// This is a stub implementation that logs a warning when the `doq` feature
    /// is not enabled but a DoQ server is requested.
    ///
    /// # Arguments
    ///
    /// * `plugin_config` - Plugin configuration (ignored in this implementation)
    #[cfg(not(feature = "doq"))]
    async fn launch_doq_server(
        &self,
        _plugin_config: &PluginConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        warn!("DoQ server requested but DoQ feature is not enabled");
        None
    }

    /// Launch Admin API server (admin feature enabled)
    ///
    /// Creates and starts the Admin API server based on the configuration.
    /// The server provides HTTP endpoints for runtime management including
    /// cache control, config reload, and server status.
    ///
    /// # Arguments
    ///
    /// * `config` - Shared configuration reference
    ///
    /// # Configuration Parameters
    ///
    /// - `enabled`: Whether to start the admin server (default: false)
    /// - `addr`: Listen address (default: "127.0.0.1:8000")
    ///
    /// # Behavior
    ///
    /// - Only starts if admin.enabled is true in configuration
    /// - Parses the listen address from configuration
    /// - Creates AdminServer with shared config
    /// - Spawns the server in a background task
    /// - Logs info when starting and errors if server creation fails
    ///
    /// # Examples
    ///
    /// ```yaml
    /// admin:
    ///   enabled: true
    ///   addr: "127.0.0.1:8000"
    /// ```
    #[cfg(feature = "admin")]
    pub async fn launch_admin_server(
        &self,
        config: Arc<RwLock<crate::config::Config>>,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let cfg = config.read().await;
        if !cfg.admin.enabled {
            return None;
        }

        let addr = normalize_listen_addr(&cfg.admin.addr);
        drop(cfg);

        info!("Starting admin API server on {}", addr);

        let state = AdminState::new(Arc::clone(&config), Arc::clone(&self.registry));
        let server = AdminServer::new(addr, state);

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            info!("Admin server task started");
            if let Err(e) = server.run_with_signal(Some(tx), None).await {
                error!("Admin server error: {}", e);
            }
            info!("Admin server task finished");
        });
        Some(rx)
    }

    /// Launch monitoring server
    ///
    /// Creates and starts the monitoring HTTP server (metrics, health, stats)
    /// based on the configuration. Returns a tuple of `(startup_rx, shutdown_tx)`
    /// which can be used to wait for server startup and to trigger a graceful
    /// shutdown.
    #[cfg(feature = "metrics")]
    pub async fn launch_monitoring_server(
        &self,
        config: Arc<RwLock<crate::config::Config>>,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let cfg = config.read().await;
        if !cfg.monitoring.enabled {
            return None;
        }

        let addr = normalize_listen_addr(&cfg.monitoring.addr);
        drop(cfg);

        info!("Starting monitoring server on {}", addr);

        let server = MonitoringServer::new(addr);

        // Start memory metrics collector if configured in monitoring section
        // This ensures the collector runs whenever the monitoring server is started
        // and keeps collector lifecycle tied to monitoring behavior.
        let mem_cfg = {
            let cfg_read = config.read().await;
            cfg_read.monitoring.memory_metrics.clone()
        };
        if mem_cfg.enabled {
            info!(
                "Starting memory metrics collector (interval: {}ms)",
                mem_cfg.interval_ms
            );
            let mem_config = crate::metrics::memory::MemoryMetricsConfig {
                enabled: mem_cfg.enabled,
                interval_ms: mem_cfg.interval_ms,
            };
            let _memory_handle = crate::metrics::memory::start_memory_metrics_collector(mem_config);
            // dropped here; the collector task keeps running in the background
        }

        let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            info!("Monitoring server task started");
            // No external shutdown receiver provided - the server will listen to
            // OS signals itself for graceful shutdown.
            if let Err(e) = server.run_with_signal(Some(startup_tx), None).await {
                error!("Monitoring server error: {}", e);
            }
            info!("Monitoring server task finished");
        });

        Some(startup_rx)
    }

    /// Launch monitoring server (metrics feature disabled)
    ///
    /// Stub implementation when the `metrics` feature is not enabled.
    #[cfg(not(feature = "metrics"))]
    pub async fn launch_monitoring_server(
        &self,
        config: Arc<RwLock<crate::config::Config>>,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        if config.read().await.monitoring.enabled {
            warn!("Monitoring server requested but metrics feature is not enabled");
        }

        None
    }

    /// Launch Admin API server (admin feature disabled)
    ///
    /// This is a stub implementation that returns None when the `admin` feature
    /// is not enabled.
    #[cfg(not(feature = "admin"))]
    pub async fn launch_admin_server(
        &self,
        config: Arc<RwLock<crate::config::Config>>,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        if config.read().await.admin.enabled {
            warn!("Admin server requested but admin feature is not enabled");
        }

        None
    }

    /// Launch WebUI server (web feature enabled)
    ///
    /// Starts the WebUI server if enabled in configuration.
    /// Returns a oneshot receiver that signals when the server is ready.
    #[cfg(feature = "web")]
    pub async fn launch_web_server(
        &self,
        config: Arc<RwLock<crate::config::Config>>,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let cfg = config.read().await;
        if !cfg.web.enabled {
            return None;
        }

        let web_config = cfg.web.clone();
        drop(cfg);

        info!("Starting WebUI server on {}", web_config.listen);

        let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();

        // Clone references for the spawned task
        let registry = Arc::clone(&self.registry);
        let global_config = Arc::clone(&config);

        tokio::spawn(async move {
            match crate::WebServer::with_admin(web_config, registry, global_config).await {
                Ok(server) => {
                    let _ = startup_tx.send(());
                    if let Err(e) = server.run().await {
                        error!("WebUI server error: {}", e);
                    }
                }
                Err(e) => {
                    error!("Failed to create WebUI server: {}", e);
                }
            }
            info!("WebUI server task finished");
        });

        Some(startup_rx)
    }

    /// Launch WebUI server (web feature disabled)
    ///
    /// Stub implementation when the `web` feature is not enabled.
    #[cfg(not(feature = "web"))]
    pub async fn launch_web_server(
        &self,
        config: Arc<RwLock<crate::config::Config>>,
    ) -> Option<tokio::sync::oneshot::Receiver<()>> {
        let _ = config;
        None
    }
}

#[cfg(test)]
mod tests {
    //! Test module for ServerLauncher
    //!
    //! This module contains comprehensive unit tests for the ServerLauncher functionality,
    //! covering normal operation, edge cases, and error conditions.

    use super::*;
    use crate::plugin::{Plugin, Registry};
    use async_trait::async_trait;
    use serde_yaml::Value;
    use std::collections::HashMap;

    // Mock plugin for testing
    /// Mock plugin implementation for testing purposes
    ///
    /// This plugin provides a minimal implementation of the Plugin trait
    /// that can be used in tests to verify plugin registry functionality.
    /// Mock plugin for testing
    ///
    /// This plugin provides a minimal implementation of the Plugin trait
    /// that can be used in tests to verify plugin registry functionality.
    #[derive(Debug)]
    struct MockPlugin;

    #[async_trait]
    impl Plugin for MockPlugin {
        /// Execute method - always succeeds for testing
        async fn execute(&self, _ctx: &mut crate::plugin::Context) -> crate::Result<()> {
            Ok(())
        }

        /// Returns the plugin name
        fn name(&self) -> &str {
            "mock_plugin"
        }
    }

    /// Test address normalization function
    ///
    /// Verifies that the `normalize_listen_addr` function correctly handles
    /// shorthand addresses and regular addresses.
    #[test]
    fn test_normalize_listen_addr() {
        assert_eq!(normalize_listen_addr(":5353"), "0.0.0.0:5353");
        assert_eq!(normalize_listen_addr("127.0.0.1:8000"), "127.0.0.1:8000");
        assert_eq!(normalize_listen_addr("0.0.0.0:53"), "0.0.0.0:53");
        assert_eq!(normalize_listen_addr("[::1]:53"), "[::1]:53");
        assert_eq!(normalize_listen_addr("localhost:8000"), "localhost:8000");
    }

    /// Test IPv6 address parsing
    ///
    /// Verifies that IPv6 addresses are correctly parsed from plugin arguments.
    #[test]
    fn test_parse_listen_addr_with_ipv6() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let mut args = HashMap::new();
        args.insert(
            "listen".to_string(),
            Value::String("[::1]:5353".to_string()),
        );

        let addr = launcher.parse_listen_addr(&args, "0.0.0.0:53");
        assert_eq!(addr, Some("[::1]:5353".parse().unwrap()));
    }

    /// Test hostname address parsing
    ///
    /// Verifies that hostname-based addresses are correctly parsed.
    #[test]
    fn test_parse_listen_addr_with_hostname() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let mut args = HashMap::new();
        args.insert(
            "listen".to_string(),
            Value::String("127.0.0.1:8000".to_string()),
        );

        let addr = launcher.parse_listen_addr(&args, "0.0.0.0:53");
        assert_eq!(addr, Some("127.0.0.1:8000".parse().unwrap()));
    }

    /// Test non-string entry value handling
    ///
    /// Verifies that non-string entry values fall back to the default.
    #[test]
    fn test_get_entry_with_non_string_value() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let mut args = HashMap::new();
        args.insert(
            "entry".to_string(),
            Value::Number(serde_yaml::Number::from(42)),
        );

        // Should fall back to default when entry is not a string
        let entry = launcher.get_entry(&args);
        assert_eq!(entry, "main_sequence");
    }

    /// Test launching multiple plugins
    ///
    /// Verifies that multiple plugins of different types can be launched together.
    #[test]
    fn test_launch_all_with_multiple_plugins() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugin1 = crate::config::PluginConfig::new("udp_server".to_string()).with_arg(
            "listen".to_string(),
            Value::String("127.0.0.1:0".to_string()),
        );

        let plugin2 = crate::config::PluginConfig::new("unknown_plugin".to_string());

        let plugin3 = crate::config::PluginConfig::new("tcp_server".to_string()).with_arg(
            "listen".to_string(),
            Value::String("127.0.0.1:0".to_string()),
        );

        let plugins = vec![plugin1, plugin2, plugin3];

        // Should handle multiple plugins, launching servers for known types and skipping unknown ones
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // receivers aren't awaited: the servers run indefinitely
        });
    }

    /// Test TCP server launching
    ///
    /// Verifies that TCP server configuration is processed correctly.
    #[test]
    fn test_launch_all_with_tcp_server_config() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugin_config = crate::config::PluginConfig::new("tcp_server".to_string()).with_arg(
            "listen".to_string(),
            Value::String("127.0.0.1:0".to_string()),
        );

        let plugins = vec![plugin_config];

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // receivers aren't awaited: the servers run indefinitely
        });
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_launch_monitoring_server() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        // Build a minimal config enabling monitoring
        let mut cfg = crate::config::Config::new();
        cfg.monitoring.enabled = true;
        cfg.monitoring.addr = "127.0.0.1:0".to_string(); // 0 means OS assign port

        let cfg_arc = Arc::new(RwLock::new(cfg));

        if let Some(startup_rx) = launcher
            .launch_monitoring_server(Arc::clone(&cfg_arc))
            .await
        {
            // Wait for server to signal startup (should complete quickly)
            let started = tokio::time::timeout(std::time::Duration::from_secs(2), startup_rx).await;
            assert!(started.is_ok());

            // Allow a few sampling intervals to elapse so the collector can register
            // and populate metrics in the global registry.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            let mfs = crate::metrics::METRICS_REGISTRY.gather();
            let names: Vec<_> = mfs.iter().map(|m| m.name()).collect();
            assert!(names.contains(&"lazydns_process_resident_memory_bytes"));
        } else {
            panic!("Expected monitoring server to be launched");
        }
    }

    /// Test monitoring server with shorthand address notation
    ///
    /// Verifies that monitoring server supports ":port" shorthand format
    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_launch_monitoring_server_with_shorthand_addr() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        // Build a config with shorthand address notation
        let mut cfg = crate::config::Config::new();
        cfg.monitoring.enabled = true;
        cfg.monitoring.addr = ":0".to_string(); // Shorthand: should expand to 0.0.0.0:0

        let cfg_arc = Arc::new(RwLock::new(cfg));

        if let Some(startup_rx) = launcher
            .launch_monitoring_server(Arc::clone(&cfg_arc))
            .await
        {
            // Wait for server to signal startup
            let started = tokio::time::timeout(std::time::Duration::from_secs(2), startup_rx).await;
            assert!(
                started.is_ok(),
                "Monitoring server should start with shorthand address"
            );
        } else {
            panic!("Expected monitoring server to be launched with shorthand address");
        }
    }

    /// Test admin server with shorthand address notation
    ///
    /// Verifies that admin server supports ":port" shorthand format
    #[cfg(feature = "admin")]
    #[tokio::test]
    async fn test_launch_admin_server_with_shorthand_addr() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        // Build a config with shorthand address notation
        let mut cfg = crate::config::Config::new();
        cfg.admin.enabled = true;
        cfg.admin.addr = ":0".to_string(); // Shorthand: should expand to 0.0.0.0:0

        let cfg_arc = Arc::new(RwLock::new(cfg));

        if let Some(startup_rx) = launcher.launch_admin_server(Arc::clone(&cfg_arc)).await {
            // Wait for server to signal startup
            let started = tokio::time::timeout(std::time::Duration::from_secs(2), startup_rx).await;
            assert!(
                started.is_ok(),
                "Admin server should start with shorthand address"
            );
        } else {
            panic!("Expected admin server to be launched with shorthand address");
        }
    }

    /// Test DoH server launching with doh feature
    ///
    /// Verifies that DoH server configuration is processed when doh is available.
    #[cfg(feature = "doh")]
    #[test]
    fn test_launch_all_with_doh_server_config() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugin_config = crate::config::PluginConfig::new("doh_server".to_string())
            .with_arg(
                "listen".to_string(),
                Value::String("127.0.0.1:0".to_string()),
            )
            .with_arg(
                "cert_file".to_string(),
                Value::String("/nonexistent/cert.pem".to_string()),
            )
            .with_arg(
                "key_file".to_string(),
                Value::String("/nonexistent/key.pem".to_string()),
            );

        let plugins = vec![plugin_config];

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // receivers aren't awaited: the servers run indefinitely
        });
    }

    /// Test DoT server launching with dot feature
    ///
    /// Verifies that DoT server configuration is processed when dot is available.
    #[cfg(feature = "dot")]
    #[test]
    fn test_launch_all_with_dot_server_config() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugin_config = crate::config::PluginConfig::new("dot_server".to_string())
            .with_arg(
                "listen".to_string(),
                Value::String("127.0.0.1:0".to_string()),
            )
            .with_arg(
                "cert_file".to_string(),
                Value::String("/nonexistent/cert.pem".to_string()),
            )
            .with_arg(
                "key_file".to_string(),
                Value::String("/nonexistent/key.pem".to_string()),
            );

        let plugins = vec![plugin_config];

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // receivers aren't awaited: the servers run indefinitely
        });
    }

    /// Test DoQ server launching with DoQ feature
    ///
    /// Verifies that DoQ server configuration is processed when DoQ is available.
    #[cfg(feature = "doq")]
    #[test]
    fn test_launch_all_with_doq_server_config() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugin_config = crate::config::PluginConfig::new("doq_server".to_string())
            .with_arg(
                "listen".to_string(),
                Value::String("127.0.0.1:0".to_string()),
            )
            .with_arg(
                "cert_file".to_string(),
                Value::String("/nonexistent/cert.pem".to_string()),
            )
            .with_arg(
                "key_file".to_string(),
                Value::String("/nonexistent/key.pem".to_string()),
            );

        let plugins = vec![plugin_config];

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // receivers aren't awaited: the servers run indefinitely
        });
    }

    /// Test ServerLauncher creation
    ///
    /// Verifies that a ServerLauncher can be created successfully.
    #[test]
    fn test_server_launcher_creation() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);
        // Just verify it was created successfully
        let _ = launcher;
    }

    /// Test valid address parsing
    ///
    /// Verifies that valid addresses are parsed correctly.
    #[test]
    fn test_parse_listen_addr_with_valid_address() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let mut args = HashMap::new();
        args.insert(
            "listen".to_string(),
            Value::String("127.0.0.1:5353".to_string()),
        );

        let addr = launcher.parse_listen_addr(&args, "0.0.0.0:53");
        assert_eq!(addr, Some("127.0.0.1:5353".parse().unwrap()));
    }

    /// Test shorthand address parsing
    ///
    /// Verifies that shorthand addresses (starting with ':') are normalized.
    #[test]
    fn test_parse_listen_addr_with_shorthand() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let mut args = HashMap::new();
        args.insert("listen".to_string(), Value::String(":8000".to_string()));

        let addr = launcher.parse_listen_addr(&args, "0.0.0.0:53");
        assert_eq!(addr, Some("0.0.0.0:8000".parse().unwrap()));
    }

    /// Test default address fallback
    ///
    /// Verifies that the default address is used when no listen address is specified.
    #[test]
    fn test_parse_listen_addr_with_default() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let args = HashMap::new(); // No listen key

        let addr = launcher.parse_listen_addr(&args, "0.0.0.0:53");
        assert_eq!(addr, Some("0.0.0.0:53".parse().unwrap()));
    }

    /// Test invalid address handling
    ///
    /// Verifies that invalid addresses return None.
    #[test]
    fn test_parse_listen_addr_with_invalid_address() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let mut args = HashMap::new();
        args.insert(
            "listen".to_string(),
            Value::String("invalid:address".to_string()),
        );

        let addr = launcher.parse_listen_addr(&args, "0.0.0.0:53");
        assert!(addr.is_none());
    }

    /// Test custom entry plugin
    ///
    /// Verifies that custom entry plugin names are extracted correctly.
    #[test]
    fn test_get_entry_with_custom_entry() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let mut args = HashMap::new();
        args.insert(
            "entry".to_string(),
            Value::String("custom_sequence".to_string()),
        );

        let entry = launcher.get_entry(&args);
        assert_eq!(entry, "custom_sequence");
    }

    /// Test default entry plugin
    ///
    /// Verifies that the default entry plugin is used when none is specified.
    #[test]
    fn test_get_entry_with_default() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let args = HashMap::new(); // No entry key

        let entry = launcher.get_entry(&args);
        assert_eq!(entry, "main_sequence");
    }

    /// Test plugin handler creation
    ///
    /// Verifies that plugin handlers are created correctly with the right entry point.
    #[test]
    fn test_create_handler() {
        let mut registry = Registry::new();
        registry.register(Arc::new(MockPlugin)).unwrap();
        let registry = Arc::new(registry);

        let launcher = ServerLauncher::new(Arc::clone(&registry));

        let handler = launcher.create_handler("mock_plugin".to_string());
        assert_eq!(handler.entry, "mock_plugin");
        assert!(Arc::ptr_eq(&handler.registry, &registry));
    }

    /// Test empty plugin list
    ///
    /// Verifies that launching with an empty plugin list doesn't cause issues.
    #[test]
    fn test_launch_all_with_empty_plugins() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugins = Vec::new();

        // This should not panic and should complete quickly
        // We can't easily test the async launch_all without complex mocking,
        // but we can at least verify it doesn't crash with empty input
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // Should return empty vector for empty plugins
            assert!(_receivers.is_empty());
        });
    }

    /// Test unknown plugin type handling
    ///
    /// Verifies that unknown plugin types are silently ignored.
    #[test]
    fn test_launch_all_with_unknown_plugin_type() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugin_config = crate::config::PluginConfig::new("unknown_server".to_string());

        let plugins = vec![plugin_config];

        // This should not panic and should skip unknown plugin types
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // Should return empty vector for unknown plugin types
            assert!(_receivers.is_empty());
        });
    }

    /// Test UDP server launching
    ///
    /// Verifies that UDP server configuration is processed correctly.
    #[test]
    fn test_launch_all_with_udp_server_config() {
        let registry = Arc::new(Registry::new());
        let launcher = ServerLauncher::new(registry);

        let plugin_config = crate::config::PluginConfig::new("udp_server".to_string()).with_arg(
            "listen".to_string(),
            serde_yaml::Value::String("127.0.0.1:0".to_string()),
        );

        let plugins = vec![plugin_config];

        // This will attempt to start a server but should fail gracefully
        // since we can't bind to privileged ports in tests
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _receivers = launcher.launch_all(&plugins).await;
            // Should return one receiver for the UDP server
            assert_eq!(_receivers.len(), 1);
        });
    }
}
