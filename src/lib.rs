//! lazydns - A DNS server implementation in Rust
//!
//! This crate provides a complete DNS server implementation
//!
//! # Architecture
//!
//! The crate is organized into several main modules:
//!
//! - `dns`: DNS protocol implementation (parsing, serialization, message handling)
//! - `server`: DNS server implementations (UDP, TCP, DoH, DoT, DoQ)
//! - `plugin`: Plugin system architecture and core plugin trait
//! - `plugins`: Collection of DNS plugins (forward, cache, hosts, and others)
//! - `config`: Configuration loading and validation
//! - `error`: Error types and handling
//!

#![warn(clippy::all)]

// Re-export proc_macro derives for plugin registration
pub use lazydns_macros::{RegisterExecPlugin, RegisterPlugin, ShutdownPlugin};

/// DNS protocol implementation
///
/// This module provides DNS message parsing, serialization, and core DNS types.
pub mod dns;

/// DNS server implementations
///
/// Includes UDP, TCP, DoH (DNS over HTTPS), DoT (DNS over TLS), and DoQ (DNS over QUIC) servers.
pub mod server;

/// Plugin system architecture
///
/// Defines the plugin trait and execution pipeline.
pub mod plugin;

/// Collection of DNS plugins
///
/// Includes forward, cache, hosts, domain matching, and other plugins.
pub mod plugins;

/// Utility helpers shared across the crate
pub mod utils;

/// Configuration loading and validation
///
/// Supports YAML configuration files with validation.
pub mod config;

/// Re-export logging types from lazylog
#[cfg(feature = "log")]
pub use lazylog::{FileLogConfig, LogConfig, RotationPeriod, RotationTrigger, init_logging};

/// Metrics collection and Prometheus exporter
///
/// Provides monitoring metrics for DNS server operations.
#[cfg(feature = "metrics")]
pub mod metrics;

/// WebUI backend server
///
/// Provides REST API and WebSocket server for real-time monitoring and analytics.
#[cfg(feature = "web")]
pub mod web;

/// Error types and handling
///
/// Provides unified error types for the entire crate.
pub mod error {

    use thiserror::Error;

    /// Main error type for lazydns
    #[derive(Error, Debug)]
    pub enum Error {
        // DNS Protocol Errors
        /// DNS protocol error
        #[error("DNS protocol error: {0}")]
        DnsProtocol(String),

        // Upstream Errors
        /// Upstream server timeout
        #[error("Upstream timeout: {upstream} ({timeout_ms}ms)")]
        UpstreamTimeout {
            /// The upstream server address
            upstream: String,
            /// Timeout duration in milliseconds
            timeout_ms: u64,
        },

        // Configuration Errors
        /// Configuration error (legacy, prefer structured variants)
        #[error("Configuration error: {0}")]
        Config(String),

        /// Missing required configuration field
        #[error("Missing required config field: {field} in {context}")]
        MissingConfigField {
            /// The missing field name
            field: String,
            /// Configuration context (such as plugin name)
            context: String,
        },

        /// Invalid configuration value
        #[error("Invalid config value for {field}: {value} - {reason}")]
        InvalidConfigValue {
            /// The field name
            field: String,
            /// The invalid value
            value: String,
            /// Reason why it's invalid
            reason: String,
        },

        // Plugin Errors
        /// Plugin error (legacy, prefer structured variants)
        #[error("Plugin error: {0}")]
        Plugin(String),

        // Network Errors
        /// Network connection error
        #[error("Connection error to {address}: {reason}")]
        Connection {
            /// Target address
            address: String,
            /// Failure reason
            reason: String,
        },

        /// Address parsing error
        #[error("Invalid address: {input}")]
        InvalidAddress {
            /// The invalid address input
            input: String,
        },

        // File/IO Errors
        /// IO error
        #[error("IO error: {0}")]
        Io(#[from] std::io::Error),

        /// File parse error
        #[error("Failed to parse file '{path}': {reason}")]
        FileParse {
            /// Path to the file
            path: String,
            /// Parse error description
            reason: String,
        },

        // Generic Errors
        /// Other error (legacy, try to use specific variants)
        #[error("Error: {0}")]
        Other(String),
    }

    impl Error {
        /// Create an UpstreamTimeout error
        pub fn upstream_timeout(upstream: impl Into<String>, timeout_ms: u64) -> Self {
            Self::UpstreamTimeout {
                upstream: upstream.into(),
                timeout_ms,
            }
        }

        /// Create a Connection error
        pub fn connection(address: impl Into<String>, reason: impl Into<String>) -> Self {
            Self::Connection {
                address: address.into(),
                reason: reason.into(),
            }
        }

        /// Create an InvalidAddress error
        pub fn invalid_address(input: impl Into<String>) -> Self {
            Self::InvalidAddress {
                input: input.into(),
            }
        }

        /// Create a MissingConfigField error
        pub fn missing_config_field(field: impl Into<String>, context: impl Into<String>) -> Self {
            Self::MissingConfigField {
                field: field.into(),
                context: context.into(),
            }
        }

        /// Create an InvalidConfigValue error
        pub fn invalid_config_value(
            field: impl Into<String>,
            value: impl Into<String>,
            reason: impl Into<String>,
        ) -> Self {
            Self::InvalidConfigValue {
                field: field.into(),
                value: value.into(),
                reason: reason.into(),
            }
        }

        /// Create a FileParse error
        pub fn file_parse(path: impl Into<String>, reason: impl Into<String>) -> Self {
            Self::FileParse {
                path: path.into(),
                reason: reason.into(),
            }
        }
    }

    /// Result type for lazydns operations
    pub type Result<T> = std::result::Result<T, Error>;
}

// Re-export commonly used types
pub use error::{Error, Result};
#[cfg(feature = "web")]
pub use web::WebServer;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_types() {
        // Test legacy error type creation
        let _dns_err = Error::DnsProtocol("test error".to_string());
        let _config_err = Error::Config("test error".to_string());
        let _plugin_err = Error::Plugin("test error".to_string());
    }

    #[test]
    fn test_structured_error_types() {
        // Test UpstreamTimeout
        let err = Error::upstream_timeout("8.8.8.8:53", 5000);
        assert!(matches!(err, Error::UpstreamTimeout { .. }));
        assert!(err.to_string().contains("8.8.8.8:53"));
        assert!(err.to_string().contains("5000ms"));

        // Test Connection
        let err = Error::connection("1.1.1.1:53", "connection refused");
        assert!(matches!(err, Error::Connection { .. }));
        assert!(err.to_string().contains("1.1.1.1:53"));

        // Test InvalidAddress
        let err = Error::invalid_address("not-an-ip");
        assert!(matches!(err, Error::InvalidAddress { .. }));
        assert!(err.to_string().contains("not-an-ip"));

        // Test InvalidConfigValue
        let err = Error::invalid_config_value("port", "abc", "must be a number");
        assert!(matches!(err, Error::InvalidConfigValue { .. }));
        assert!(err.to_string().contains("port"));
        assert!(err.to_string().contains("abc"));

        // Test MissingConfigField
        let err = Error::missing_config_field("upstream", "forward");
        assert!(matches!(err, Error::MissingConfigField { .. }));
        assert!(err.to_string().contains("upstream"));

        // Test FileParse
        let err = Error::file_parse("/etc/hosts", "invalid line");
        assert!(matches!(err, Error::FileParse { .. }));
        assert!(err.to_string().contains("/etc/hosts"));
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
    }
}
