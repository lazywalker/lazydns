//! Server configuration
//!
//! Types and helpers for configuring the DNS servers used by lazydns.
//!
//! The `ServerConfig` encapsulates listen addresses, timeouts, limits,
//! and other runtime parameters. It provides a builder-style API for
//! convenient construction and modification.
//!
//! # Examples
//!
//! Construct a default configuration and override the UDP address:
//!
//! ```rust
//! use std::str::FromStr;
//! use lazydns::server::config::ServerConfig;
//! let cfg = ServerConfig::default().with_udp_addr(FromStr::from_str("192.0.2.1:53").unwrap());
//! ```

use crate::server::RequestHandler;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

#[cfg(any(feature = "doh", feature = "dot"))]
use crate::server::TlsConfig;

/// DNS server configuration
///
/// Holds settings that control server behavior. The struct is `Clone` so it
/// can be shared safely across server components. Typical fields include
/// listen addresses for UDP/TCP, request timeouts, and protocol-specific
/// size limits.
///
/// Use the builder-style methods (such as `with_udp_addr`) to customize a
/// configuration built from `ServerConfig::default()`.
#[derive(Clone)]
pub struct ServerConfig {
    /// UDP listen address
    pub udp_addr: Option<SocketAddr>,

    /// TCP listen address
    pub tcp_addr: Option<SocketAddr>,

    /// Maximum number of concurrent connections
    pub max_connections: usize,

    /// Query timeout duration
    pub timeout: Duration,

    /// Maximum UDP packet size
    pub max_udp_size: usize,

    /// Maximum TCP message size
    pub max_tcp_size: usize,

    /// Request handler for processing DNS queries
    pub handler: Option<Arc<dyn RequestHandler>>,

    /// TLS configuration for DoH/DoT servers
    #[cfg(any(feature = "doh", feature = "dot"))]
    pub tls_config: Option<TlsConfig>,

    /// Certificate path for DoQ server
    pub cert_path: Option<String>,

    /// Key path for DoQ server
    pub key_path: Option<String>,

    /// DoH query path (default: /dns-query)
    pub doh_path: Option<String>,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("udp_addr", &self.udp_addr)
            .field("tcp_addr", &self.tcp_addr)
            .field("max_connections", &self.max_connections)
            .field("timeout", &self.timeout)
            .field("max_udp_size", &self.max_udp_size)
            .field("max_tcp_size", &self.max_tcp_size)
            .field("handler", &self.handler.as_ref().map(|_| "<handler>"))
            .field("cert_path", &self.cert_path)
            .field("key_path", &self.key_path)
            .field("doh_path", &self.doh_path)
            .finish()
    }
}

impl Default for ServerConfig {
    /// Return a sensible default configuration intended for local testing
    /// and development.
    ///
    /// Defaults:
    /// - `udp_addr` / `tcp_addr`: `127.0.0.1:5353`
    /// - `max_connections`: 1000
    /// - `timeout`: 5 seconds
    /// - `max_udp_size`: 512
    /// - `max_tcp_size`: 65535
    fn default() -> Self {
        Self {
            udp_addr: Some("127.0.0.1:5353".parse().unwrap()),
            tcp_addr: Some("127.0.0.1:5353".parse().unwrap()),
            max_connections: 1000,
            timeout: Duration::from_secs(5),
            max_udp_size: 512,
            max_tcp_size: 65535,
            handler: None,
            #[cfg(any(feature = "doh", feature = "dot"))]
            tls_config: None,
            cert_path: None,
            key_path: None,
            doh_path: None,
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration with the given UDP and TCP addresses.
    ///
    /// This helper sets the supplied addresses and inherits remaining defaults
    /// from `ServerConfig::default()`.
    ///
    /// # Arguments
    ///
    /// * `udp_addr` - Optional UDP listen address
    /// * `tcp_addr` - Optional TCP listen address
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use lazydns::server::config::ServerConfig;
    /// let cfg = ServerConfig::new(Some(FromStr::from_str("192.0.2.1:53").unwrap()), None);
    /// ```
    pub fn new(udp_addr: Option<SocketAddr>, tcp_addr: Option<SocketAddr>) -> Self {
        Self {
            udp_addr,
            tcp_addr,
            ..Default::default()
        }
    }

    /// Set the UDP listen address.
    ///
    /// This follows a builder pattern and returns `self` for chaining.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use lazydns::server::config::ServerConfig;
    /// let cfg = ServerConfig::default().with_udp_addr(FromStr::from_str("0.0.0.0:53").unwrap());
    /// ```
    pub fn with_udp_addr(mut self, addr: SocketAddr) -> Self {
        self.udp_addr = Some(addr);
        self
    }

    /// Set the TCP listen address.
    ///
    /// Returns `self` to allow chaining with other builder methods.
    pub fn with_tcp_addr(mut self, addr: SocketAddr) -> Self {
        self.tcp_addr = Some(addr);
        self
    }

    /// Set the maximum number of concurrent connections.
    ///
    /// This limit is applied per-server instance and helps control resource
    /// usage under load.
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the query timeout duration.
    ///
    /// This timeout applies to upstream queries and socket operations that
    /// respect the configured deadline.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the maximum UDP packet size (in bytes).
    ///
    /// This value controls buffer allocation for UDP reads and may affect
    /// truncation behavior when responses exceed the buffer size.
    pub fn with_max_udp_size(mut self, size: usize) -> Self {
        self.max_udp_size = size;
        self
    }

    /// Set the maximum TCP message size (in bytes).
    ///
    /// Controls how large a single TCP DNS message can be; larger messages
    /// may be rejected or truncated based on this setting.
    pub fn with_max_tcp_size(mut self, size: usize) -> Self {
        self.max_tcp_size = size;
        self
    }

    /// Set the request handler.
    pub fn with_handler(mut self, handler: Arc<dyn RequestHandler>) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Set the TLS configuration for DoH/DoT.
    #[cfg(any(feature = "doh", feature = "dot"))]
    pub fn with_tls_config(mut self, tls_config: TlsConfig) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Set certificate and key paths for DoQ.
    pub fn with_cert_paths(mut self, cert_path: String, key_path: String) -> Self {
        self.cert_path = Some(cert_path);
        self.key_path = Some(key_path);
        self
    }

    /// Set the DoH query path.
    pub fn with_doh_path(mut self, path: String) -> Self {
        self.doh_path = Some(path);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_default_config() {
        let config = ServerConfig::default();
        assert!(config.udp_addr.is_some());
        assert!(config.tcp_addr.is_some());
        assert_eq!(config.max_connections, 1000);
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.max_udp_size, 512);
        assert_eq!(config.max_tcp_size, 65535);
    }

    #[test]
    fn test_builder_pattern() {
        let addr = SocketAddr::from_str("192.0.2.1:53").unwrap();
        let config = ServerConfig::default()
            .with_udp_addr(addr)
            .with_max_connections(500)
            .with_timeout(Duration::from_secs(10));

        assert_eq!(config.udp_addr, Some(addr));
        assert_eq!(config.max_connections, 500);
        assert_eq!(config.timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_new_config() {
        let udp = SocketAddr::from_str("192.0.2.1:53").unwrap();
        let tcp = SocketAddr::from_str("192.0.2.2:53").unwrap();
        let config = ServerConfig::new(Some(udp), Some(tcp));

        assert_eq!(config.udp_addr, Some(udp));
        assert_eq!(config.tcp_addr, Some(tcp));
    }

    #[test]
    fn test_with_tcp_addr() {
        let addr = SocketAddr::from_str("192.0.2.1:53").unwrap();
        let config = ServerConfig::default().with_tcp_addr(addr);
        assert_eq!(config.tcp_addr, Some(addr));
    }

    #[test]
    fn test_with_max_udp_size() {
        let config = ServerConfig::default().with_max_udp_size(4096);
        assert_eq!(config.max_udp_size, 4096);
    }

    #[test]
    fn test_with_max_tcp_size() {
        let config = ServerConfig::default().with_max_tcp_size(32768);
        assert_eq!(config.max_tcp_size, 32768);
    }

    #[test]
    fn test_with_cert_paths() {
        let config = ServerConfig::default().with_cert_paths(
            "/etc/ssl/cert.pem".to_string(),
            "/etc/ssl/key.pem".to_string(),
        );
        assert_eq!(config.cert_path, Some("/etc/ssl/cert.pem".to_string()));
        assert_eq!(config.key_path, Some("/etc/ssl/key.pem".to_string()));
    }

    #[test]
    fn test_with_doh_path() {
        let config = ServerConfig::default().with_doh_path("/dns".to_string());
        assert_eq!(config.doh_path, Some("/dns".to_string()));
    }

    #[test]
    fn test_config_debug() {
        let config = ServerConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("ServerConfig"));
        assert!(debug_str.contains("max_connections"));
        assert!(debug_str.contains("timeout"));
    }

    #[test]
    fn test_config_clone() {
        let config = ServerConfig::default().with_max_connections(500);
        let cloned = config.clone();
        assert_eq!(cloned.max_connections, 500);
    }
}
