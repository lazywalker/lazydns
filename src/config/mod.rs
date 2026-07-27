//! Configuration module
//!
//! This module provides configuration loading and validation for lazydns.
//! It supports YAML configuration files with comprehensive validation.
//!
//! # Example
//!
//! ```rust,no_run
//! use lazydns::config::Config;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Config::from_file("config.yaml")?;
//! println!("Configured plugins: {:?}", config.plugins);
//! # Ok(())
//! # }
//! ```

pub mod loader;
pub mod reload;
pub mod types;
pub mod validation;

use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

// Re-export commonly used types
#[cfg(feature = "web")]
pub use crate::web::config::WebConfig;
pub use reload::ConfigReloader;
pub use types::PluginConfig;

// Re-export lazylog types for configuration
#[cfg(feature = "log")]
pub use lazylog::{
    FileLogConfig as LazylogFileConfig, LogConfig as LazylogLogConfig, RotationPeriod,
    RotationTrigger,
};

/// File logging configuration with rotation support (lazydns adapter).
///
/// This struct controls file-based logging in lazydns. It wraps the
/// [lazylog::FileLogConfig] and adds an `enabled` field for convenience.
///
/// # Examples
///
/// Rotate by size:
///
/// ```yaml
/// log:
///   file:
///     enabled: true
///     path: ./logs/lazydns.log
///     rotation:
///       type: size
///       max_size: "10M"
///       max_files: 5
/// ```
///
/// To rotate files daily:
///
/// ```yaml
/// log:
///   file:
///     enabled: true
///     path: ./logs/lazydns.log
///     rotation:
///       type: time
///       period: daily
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileLogConfig {
    /// Whether file logging is enabled (default: false).
    ///
    /// Even if other file configuration is present, logging is only
    /// written to files if this is true.
    #[serde(default)]
    pub enabled: bool,

    /// Path to the log file.
    ///
    /// Can be absolute or relative. Parent directories are created
    /// automatically if needed.
    #[serde(default = "default_file_path")]
    pub path: String,

    /// Rotation configuration.
    ///
    /// Determines when and how log files are rotated.
    #[serde(default)]
    #[cfg(feature = "log")]
    pub rotation: RotationTrigger,

    /// Whether to compress rotated files (reserved for future use).
    #[serde(default)]
    pub compress: bool,
}

fn default_file_path() -> String {
    "lazydns.log".to_string()
}

impl Default for FileLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_file_path(),
            #[cfg(feature = "log")]
            rotation: RotationTrigger::default(),
            compress: false,
        }
    }
}

impl FileLogConfig {
    /// Create a new FileLogConfig with the given path
    pub fn new<S: Into<String>>(path: S) -> Self {
        Self {
            path: path.into(),
            enabled: false,
            #[cfg(feature = "log")]
            rotation: RotationTrigger::default(),
            compress: false,
        }
    }

    /// Enable file logging for this configuration
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

#[cfg(feature = "log")]
impl FileLogConfig {
    /// Set rotation trigger
    pub fn with_rotation(mut self, rotation: RotationTrigger) -> Self {
        self.rotation = rotation;
        self
    }

    /// Convert lazydns FileLogConfig to lazylog FileLogConfig
    pub fn to_lazylog(&self) -> LazylogFileConfig {
        LazylogFileConfig::new(self.path.clone()).with_rotation_trigger(self.rotation.clone())
    }
}

/// Logging configuration (lazydns adapter).
///
/// lazydns provides a thin wrapper around [lazylog::LogConfig] to:
/// 1. Maintain a simple, DNS-focused configuration interface
/// 2. Support domain-specific defaults (such as file.enabled)
/// 3. Enable potential future logging backend swapping
///
/// # Conversion to lazylog
///
/// Use [LogConfig::to_lazylog] to convert to the underlying lazylog configuration
/// for actual logging initialization.
///
/// # Examples
///
/// ```yaml
/// log:
///   level: info
///   console: true
///   format: text
///   file:
///     enabled: false
///     path: ./lazydns.log
/// ```
///
/// ```rust,no_run
/// # use lazydns::config::LogConfig;
/// let config = LogConfig::default();
/// let lazy_config = config.to_lazylog("info".to_string());
/// // Use lazy_config with lazylog::init_logging()
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogConfig {
    /// Log level: trace|debug|info|warn|error (default: info).
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Whether to output logs to console/stdout (default: false).
    #[serde(default = "default_console")]
    pub console: bool,

    /// Log output format: text|json (default: text).
    #[serde(default = "default_log_format")]
    pub format: String,

    /// File logging configuration.
    #[serde(default)]
    pub file: Option<FileLogConfig>,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_console() -> bool {
    false
}

fn default_log_format() -> String {
    "text".to_string()
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            console: default_console(),
            format: default_log_format(),
            file: None,
        }
    }
}

impl LogConfig {
    /// Create a new LogConfig with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if file logging is enabled
    pub fn is_file_logging_enabled(&self) -> bool {
        self.file.as_ref().is_some_and(|f| f.enabled)
    }

    /// Check if console logging is enabled
    pub fn is_console_logging_enabled(&self) -> bool {
        self.console
    }

    /// Get effective logging configuration (for debugging)
    pub fn summary(&self) -> String {
        format!(
            "LogConfig {{ level: {}, console: {}, format: {}, file_logging: {} }}",
            self.level,
            self.console,
            self.format,
            self.is_file_logging_enabled()
        )
    }
}

#[cfg(feature = "log")]
impl LogConfig {
    /// Convert to lazylog configuration with specified log level
    ///
    /// The `log_spec` parameter allows overriding the configured level,
    /// typically used for CLI verbosity options.
    ///
    /// File logging is only enabled if both:
    /// - `self.file` is Some
    /// - `file.enabled` is true
    ///
    /// # Arguments
    ///
    /// * `log_spec` - Log level specification (such as "info", "debug")
    ///
    /// # Returns
    ///
    /// A [LazylogLogConfig] ready to pass to [lazylog::init_logging]
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use lazydns::config::LogConfig;
    /// let config = LogConfig::default();
    /// let lazy = config.to_lazylog("debug".to_string());
    /// // Pass to lazylog::init_logging(&lazy)
    /// ```
    pub fn to_lazylog(&self, log_spec: String) -> LazylogLogConfig {
        let mut config = LazylogLogConfig::new()
            .with_console(self.console)
            .with_level(log_spec)
            .with_format(self.format.clone());

        if let Some(ref file) = self.file
            && file.enabled
        {
            config = config.with_file(file.to_lazylog());
        }

        config
    }

    /// Convert to lazylog configuration using the configured level
    ///
    /// This is equivalent to calling [LogConfig::to_lazylog] with
    /// the configured level value.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use lazydns::config::LogConfig;
    /// let config = LogConfig::default();
    /// let lazy = config.to_lazylog_with_default();
    /// ```
    pub fn to_lazylog_with_default(&self) -> LazylogLogConfig {
        self.to_lazylog(self.level.clone())
    }
}

/// Admin API configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    /// Enable admin API
    #[serde(default = "default_admin_enabled")]
    pub enabled: bool,

    /// Listen address for admin API (such as "127.0.0.1:8000")
    #[serde(default = "default_admin_addr")]
    pub addr: String,
}

fn default_admin_enabled() -> bool {
    false
}

fn default_admin_addr() -> String {
    "127.0.0.1:8000".to_string()
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            enabled: default_admin_enabled(),
            addr: default_admin_addr(),
        }
    }
}

/// Monitoring server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Enable monitoring server
    #[serde(default = "default_monitoring_enabled")]
    pub enabled: bool,

    /// Listen address for monitoring server (such as "127.0.0.1:8001")
    #[serde(default = "default_monitoring_addr")]
    pub addr: String,

    /// Memory metrics collection configuration
    #[serde(default)]
    pub memory_metrics: MemoryMetricsConfig,
}

/// Memory metrics collection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetricsConfig {
    /// Enable memory metrics collection
    #[serde(default = "default_memory_metrics_enabled")]
    pub enabled: bool,

    /// Sampling interval in milliseconds
    #[serde(default = "default_memory_metrics_interval_ms")]
    pub interval_ms: u64,
}

fn default_memory_metrics_enabled() -> bool {
    true
}

fn default_memory_metrics_interval_ms() -> u64 {
    5000 // 5 seconds
}

impl Default for MemoryMetricsConfig {
    fn default() -> Self {
        Self {
            enabled: default_memory_metrics_enabled(),
            interval_ms: default_memory_metrics_interval_ms(),
        }
    }
}

fn default_monitoring_enabled() -> bool {
    false
}

fn default_monitoring_addr() -> String {
    "127.0.0.1:8001".to_string()
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enabled: default_monitoring_enabled(),
            addr: default_monitoring_addr(),
            memory_metrics: MemoryMetricsConfig::default(),
        }
    }
}

/// Main configuration structure
///
/// This is the root configuration object that contains settings for
/// plugins and logging. The legacy `server` configuration section has
/// been removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Plugin configurations
    #[serde(default)]
    pub plugins: Vec<PluginConfig>,

    /// Logging configuration
    #[serde(default = "default_log_config")]
    pub log: LogConfig,

    /// Admin API configuration
    #[serde(default = "default_admin_config")]
    pub admin: AdminConfig,

    /// Monitoring server configuration
    #[serde(default = "default_monitoring_config", alias = "metrics")]
    pub monitoring: MonitoringConfig,

    /// WebUI server configuration
    #[cfg(feature = "web")]
    #[serde(default = "default_web_config")]
    pub web: crate::web::config::WebConfig,
}

fn default_log_config() -> LogConfig {
    LogConfig::default()
}

fn default_admin_config() -> AdminConfig {
    AdminConfig::default()
}

fn default_monitoring_config() -> MonitoringConfig {
    MonitoringConfig::default()
}

#[cfg(feature = "web")]
fn default_web_config() -> crate::web::config::WebConfig {
    crate::web::config::WebConfig::default()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            log: default_log_config(),
            admin: default_admin_config(),
            monitoring: default_monitoring_config(),
            #[cfg(feature = "web")]
            web: default_web_config(),
            // rotation disabled by default
            // `rotate_dir` defaults to None which means use parent dir of `file` if provided
        }
    }
}

impl Config {
    /// Create a new default configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Load configuration from a YAML file
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the YAML configuration file
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        loader::load_from_file(path)
    }

    /// Load configuration from a YAML string
    ///
    /// # Arguments
    ///
    /// * `yaml` - YAML configuration string
    ///
    /// # Errors
    ///
    /// Returns an error if the YAML cannot be parsed.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        loader::load_from_yaml(yaml)
    }

    /// Validate the configuration
    ///
    /// Checks that all configuration values are valid and consistent.
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails.
    pub fn validate(&self) -> Result<()> {
        validation::validate_config(self)
    }

    /// Save configuration to a YAML file
    ///
    /// # Arguments
    ///
    /// * `path` - Path to save the YAML configuration file
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        loader::save_to_file(self, path)
    }

    /// Convert configuration to YAML string
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_yaml(&self) -> Result<String> {
        loader::to_yaml(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.log.level, "info");
        assert!(config.plugins.is_empty());
    }

    #[test]
    fn test_new_config() {
        let config = Config::new();
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn test_from_yaml_minimal() {
        let yaml = r#"
log:
  level: debug
"#;
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.log.level, "debug");
    }

    #[test]
    fn test_to_yaml() {
        let config = Config::new();
        let yaml = config.to_yaml().unwrap();
        assert!(yaml.contains("log:"));
    }

    // LogConfig tests
    #[test]
    fn test_log_config_new() {
        let config = LogConfig::new();
        assert_eq!(config.level, "info");
        assert!(!config.console);
        assert_eq!(config.format, "text");
        assert!(config.file.is_none());
    }

    #[test]
    fn test_log_config_is_console_logging_enabled() {
        let mut config = LogConfig::default();
        assert!(!config.is_console_logging_enabled());
        config.console = true;
        assert!(config.is_console_logging_enabled());
    }

    #[test]
    fn test_log_config_is_file_logging_enabled() {
        let mut config = LogConfig::default();
        assert!(!config.is_file_logging_enabled());

        config.file = Some(FileLogConfig::new("test.log"));
        assert!(!config.is_file_logging_enabled()); // Not enabled even with file config

        config.file = Some(FileLogConfig::new("test.log").with_enabled(true));
        assert!(config.is_file_logging_enabled());
    }

    #[test]
    fn test_log_config_summary() {
        let config = LogConfig::default();
        let summary = config.summary();
        assert!(summary.contains("level: info"));
        assert!(summary.contains("console: false"));
        assert!(summary.contains("file_logging: false"));
    }

    #[test]
    #[cfg(feature = "log")]
    fn test_log_config_to_lazylog_with_level_override() {
        let config = LogConfig {
            level: "debug".to_string(),
            console: true,
            format: "json".to_string(),
            file: None,
        };

        let lazy = config.to_lazylog("info".to_string());
        assert_eq!(lazy.level, "info"); // Level is overridden
        assert!(lazy.console);
        assert_eq!(lazy.format, "json");
        assert!(lazy.file.is_none());
    }

    #[test]
    #[cfg(feature = "log")]
    fn test_log_config_to_lazylog_with_default() {
        let config = LogConfig {
            level: "debug".to_string(),
            console: true,
            format: "text".to_string(),
            file: None,
        };

        let lazy = config.to_lazylog_with_default();
        assert_eq!(lazy.level, "debug"); // Uses configured level
        assert!(lazy.console);
        assert_eq!(lazy.format, "text");
        assert!(lazy.file.is_none());
    }

    #[test]
    #[cfg(feature = "log")]
    fn test_log_config_to_lazylog_file_enabled() {
        let config = LogConfig {
            console: true,
            file: Some(FileLogConfig::new("test.log").with_enabled(true)),
            ..Default::default()
        };

        let lazy = config.to_lazylog("info".to_string());
        assert!(lazy.console);
        assert!(lazy.file.is_some());
        assert_eq!(lazy.file.unwrap().path.to_string_lossy(), "test.log");
    }

    #[test]
    #[cfg(feature = "log")]
    fn test_log_config_to_lazylog_file_disabled() {
        let config = LogConfig {
            console: true,
            file: Some(FileLogConfig::new("test.log").with_enabled(false)),
            ..Default::default()
        };

        let lazy = config.to_lazylog("info".to_string());
        assert!(lazy.console);
        assert!(lazy.file.is_none()); // File not included when disabled
    }

    // FileLogConfig tests
    #[test]
    fn test_file_log_config_new() {
        let config = FileLogConfig::new("app.log");
        assert_eq!(config.path, "app.log");
        assert!(!config.enabled);
        assert!(!config.compress);
    }

    #[test]
    fn test_file_log_config_with_enabled() {
        let config = FileLogConfig::new("app.log").with_enabled(true);
        assert!(config.enabled);
    }

    #[test]
    #[cfg(feature = "log")]
    fn test_file_log_config_with_rotation() {
        let config = FileLogConfig::new("app.log")
            .with_enabled(true)
            .with_rotation(RotationTrigger::Never);
        assert!(config.enabled);
        assert_eq!(config.rotation, RotationTrigger::Never);
    }

    #[test]
    #[cfg(feature = "log")]
    fn test_file_log_config_to_lazylog() {
        let config = FileLogConfig::new("test.log")
            .with_enabled(true)
            .with_rotation(RotationTrigger::Never);
        let lazy = config.to_lazylog();
        assert_eq!(lazy.path.to_string_lossy(), "test.log");
    }
}
