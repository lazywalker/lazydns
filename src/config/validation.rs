//! Configuration validation
//!
//! Validates configuration values for correctness and consistency.

use crate::config::Config;
use crate::{Error, Result};
use serde_yaml::Value;
use std::net::SocketAddr;

/// Valid port range (1-65535, 0 is reserved)
const MIN_PORT: u16 = 1;

/// Valid TTL range (0-2147483647, max i32)
const MAX_TTL: u32 = i32::MAX as u32;

/// Reasonable timeout range (1-3600 seconds)
const MIN_TIMEOUT_SECS: u64 = 1;
const MAX_TIMEOUT_SECS: u64 = 3600;

/// Reasonable rate limit range
const MIN_RATE_LIMIT: u32 = 1;
const MAX_RATE_LIMIT: u32 = 1_000_000;

/// Reasonable window range (1-86400 seconds = 1 day)
const MIN_WINDOW_SECS: u64 = 1;
const MAX_WINDOW_SECS: u64 = 86400;

/// Validate a configuration
///
/// Checks that all configuration values are valid and consistent.
///
/// # Arguments
///
/// * `config` - The configuration to validate
///
/// # Errors
///
/// Returns an error if validation fails.
pub fn validate_config(config: &Config) -> Result<()> {
    // Validate logging configuration
    validate_log_level(&config.log.level)?;
    validate_log_format(&config.log.format)?;
    // Rotation is now validated via serde/RotationTrigger enum

    // Validate admin and monitoring addresses
    validate_socket_addr(&config.admin.addr, "admin")?;
    validate_socket_addr(&config.monitoring.addr, "monitoring")?;

    // Validate plugins
    validate_plugins(&config.plugins)?;

    Ok(())
}

/// Validate log level
fn validate_log_level(level: &str) -> Result<()> {
    let valid_levels = ["trace", "debug", "info", "warn", "error"];

    if !valid_levels.contains(&level) {
        return Err(Error::Config(format!(
            "Invalid log level '{}'. Must be one of: {}",
            level,
            valid_levels.join(", ")
        )));
    }

    Ok(())
}

fn validate_log_format(format: &str) -> Result<()> {
    let valid = ["text", "json"];
    if !valid.contains(&format) {
        return Err(Error::Config(format!(
            "Invalid log format '{}'. Must be one of: {}",
            format,
            valid.join(", ")
        )));
    }
    Ok(())
}

/// Validate plugins
fn validate_plugins(plugins: &[crate::config::PluginConfig]) -> Result<()> {
    // Check for duplicate plugin names
    let mut names = std::collections::HashSet::new();

    for plugin in plugins {
        let name = plugin.effective_name();

        if !names.insert(name) {
            return Err(Error::Config(format!("Duplicate plugin name: '{}'", name)));
        }
    }

    // Validate plugin types are not empty
    for plugin in plugins {
        if plugin.plugin_type.is_empty() {
            return Err(Error::Config("Plugin type cannot be empty".to_string()));
        }

        // Validate plugin-specific numeric parameters
        validate_plugin_args(&plugin.plugin_type, &plugin.effective_args())?;
    }

    Ok(())
}

/// Validate socket address format
fn validate_socket_addr(addr: &str, context: &str) -> Result<()> {
    // Normalize address format:
    // ":port" -> "0.0.0.0:port" (IPv4 wildcard)
    // "::port" -> "[::]:port" (IPv6 wildcard)
    // "::1:port" -> "[::1]:port" (IPv6 loopback with port)
    let normalized_addr = if let Some(port) = addr.strip_prefix(":::") {
        // ":::port" -> "[::]:port" (IPv6 wildcard)
        format!("[::]:{}", port)
    } else if let Some(rest) = addr.strip_prefix("::") {
        // Check if it's "::" followed by IPv6 address and port (such as "::1:8080")
        // or just "::port" (such as "::8080")
        if let Some(last_colon) = rest.rfind(':') {
            if last_colon > 0 {
                // Has more than just "" after "::" before the last colon
                // Check if everything after the last colon is a valid port number
                if rest[last_colon + 1..].parse::<u16>().is_ok() {
                    // It's an IPv6 address with port (such as "::1:8080")
                    let ip_part = &rest[..last_colon];
                    let port_part = &rest[last_colon + 1..];
                    format!("[::{}]:{}", ip_part, port_part)
                } else {
                    // Not a port number, treat as plain IPv6 address
                    addr.to_string()
                }
            } else {
                // "::port" format -> "[::]:port"
                format!("[::]:{}", rest)
            }
        } else if rest.parse::<u16>().is_ok() {
            // No colon in rest, but rest is a valid port number (such as "::8080")
            // This is IPv6 wildcard shorthand: "::port" -> "[::]:port"
            format!("[::]:{}", rest)
        } else {
            // Just "::" without port, or something else
            addr.to_string()
        }
    } else if addr.starts_with(':') && !addr.starts_with("::[") {
        // ":port" -> "0.0.0.0:port" (IPv4 wildcard)
        format!("0.0.0.0{}", addr)
    } else {
        addr.to_string()
    };

    let socket_addr = normalized_addr
        .parse::<SocketAddr>()
        .map_err(|e| Error::Config(format!("Invalid {} address '{}': {}", context, addr, e)))?;

    // Validate port range (port 0 is reserved)
    let port = socket_addr.port();
    if port < MIN_PORT {
        return Err(Error::Config(format!(
            "Invalid {} port {}: must be at least {}",
            context, port, MIN_PORT
        )));
    }

    Ok(())
}

/// Validate plugin-specific arguments
fn validate_plugin_args(
    plugin_type: &str,
    args: &std::collections::HashMap<String, Value>,
) -> Result<()> {
    match plugin_type {
        "rate_limit" => {
            // both optional: the plugin defaults to 100 queries / 60s
            validate_int_range::<u32>(args, "max_queries", MIN_RATE_LIMIT, MAX_RATE_LIMIT, true)?;
            validate_int_range::<u64>(args, "window_secs", MIN_WINDOW_SECS, MAX_WINDOW_SECS, true)?;
        }
        "ttl" => {
            validate_int_range::<u32>(args, "ttl", 0, MAX_TTL, true)?;
            validate_int_range::<u32>(args, "min", 0, MAX_TTL, true)?;
            validate_int_range::<u32>(args, "max", 0, MAX_TTL, true)?;
        }
        "downloader" => {
            validate_int_range::<u64>(
                args,
                "timeout_secs",
                MIN_TIMEOUT_SECS,
                MAX_TIMEOUT_SECS,
                true,
            )?;
            validate_int_range::<u64>(args, "retry_delay_secs", 0, MAX_TIMEOUT_SECS, true)?;
            validate_int_range::<usize>(args, "max_retries", 0, 100, true)?;
        }
        "cache" => {
            validate_int_range::<usize>(args, "size", 1, usize::MAX, true)?;
            validate_int_range::<u32>(args, "negative_ttl", 0, MAX_TTL, true)?;
        }
        "forward" => {
            validate_int_range::<u64>(args, "timeout", MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS, true)?;
            validate_int_range::<usize>(args, "concurrent", 1, 1000, true)?;
        }
        _ => {}
    }

    Ok(())
}

trait IntValue: Copy + PartialOrd + std::fmt::Display + std::str::FromStr + TryFrom<u64> {
    fn from_yaml_number(n: &serde_yaml::Number) -> Option<Self>;
    fn type_name() -> &'static str;
}

impl IntValue for u32 {
    fn from_yaml_number(n: &serde_yaml::Number) -> Option<Self> {
        n.as_u64().and_then(|v| u32::try_from(v).ok())
    }
    fn type_name() -> &'static str {
        "u32"
    }
}

impl IntValue for u64 {
    fn from_yaml_number(n: &serde_yaml::Number) -> Option<Self> {
        n.as_u64()
    }
    fn type_name() -> &'static str {
        "u64"
    }
}

impl IntValue for usize {
    fn from_yaml_number(n: &serde_yaml::Number) -> Option<Self> {
        n.as_u64().and_then(|v| usize::try_from(v).ok())
    }
    fn type_name() -> &'static str {
        "usize"
    }
}

fn extract_int<T: IntValue>(value: &Value, key: &str) -> Result<T> {
    match value {
        Value::Number(n) => T::from_yaml_number(n).ok_or_else(|| {
            Error::Config(format!(
                "Parameter '{}' must be a valid {} number",
                key,
                T::type_name()
            ))
        }),
        Value::String(s) => s.parse::<T>().map_err(|_| {
            Error::Config(format!(
                "Parameter '{}' must be a valid {} number",
                key,
                T::type_name()
            ))
        }),
        _ => Err(Error::Config(format!(
            "Parameter '{}' must be a number",
            key
        ))),
    }
}

fn validate_int_range<T: IntValue>(
    args: &std::collections::HashMap<String, Value>,
    key: &str,
    min: T,
    max: T,
    optional: bool,
) -> Result<()> {
    if let Some(value) = args.get(key) {
        let val = extract_int::<T>(value, key)?;
        if val < min || val > max {
            return Err(Error::Config(format!(
                "Parameter '{}' must be between {} and {}, got {}",
                key, min, max, val
            )));
        }
    } else if !optional {
        return Err(Error::Config(format!(
            "Required parameter '{}' is missing",
            key
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PluginConfig, loader};

    #[test]
    fn test_validate_valid_config() {
        let config = Config::new();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_log_level_valid() {
        assert!(validate_log_level("info").is_ok());
        assert!(validate_log_level("debug").is_ok());
        assert!(validate_log_level("trace").is_ok());
        assert!(validate_log_level("warn").is_ok());
        assert!(validate_log_level("error").is_ok());
    }

    #[test]
    fn test_validate_log_level_invalid() {
        let result = validate_log_level("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_plugins_duplicate_names() {
        // Duplicate names should fail: two forward plugins is a duplicate
        let plugins = vec![
            PluginConfig::new("forward".to_string()),
            PluginConfig::new("forward".to_string()),
        ];

        let result = validate_plugins(&plugins);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_plugins_empty_type() {
        let plugins = vec![PluginConfig::new("".to_string())];

        let result = validate_plugins(&plugins);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_plugins_valid() {
        let plugins = vec![
            PluginConfig::new("forward".to_string()),
            PluginConfig::new("cache".to_string()),
        ];

        let result = validate_plugins(&plugins);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_socket_addr_valid() {
        // IPv4 formats
        assert!(validate_socket_addr("127.0.0.1:8080", "test").is_ok());
        assert!(validate_socket_addr("0.0.0.0:53", "test").is_ok());
        assert!(validate_socket_addr(":8000", "test").is_ok()); // IPv4 wildcard shorthand
        assert!(validate_socket_addr(":53", "test").is_ok());

        // IPv6 formats
        assert!(validate_socket_addr("[::1]:8080", "test").is_ok()); // Standard IPv6
        assert!(validate_socket_addr("[::]:8080", "test").is_ok()); // IPv6 wildcard
        assert!(validate_socket_addr("::1:8080", "test").is_ok()); // IPv6 without brackets
        assert!(validate_socket_addr("::8080", "test").is_ok()); // IPv6 wildcard shorthand
        assert!(validate_socket_addr(":::8080", "test").is_ok()); // Alternative IPv6 wildcard shorthand
    }

    #[test]
    fn test_validate_socket_addr_invalid() {
        assert!(validate_socket_addr("invalid", "test").is_err());
        assert!(validate_socket_addr("127.0.0.1:99999", "test").is_err());
        assert!(validate_socket_addr("127.0.0.1", "test").is_err());
        // Port 0 is reserved
        assert!(validate_socket_addr("127.0.0.1:0", "test").is_err());
    }

    #[test]
    fn test_validate_rate_limit_params() {
        use std::collections::HashMap;

        let mut args = HashMap::new();
        args.insert("max_queries".to_string(), Value::Number(100.into()));
        args.insert("window_secs".to_string(), Value::Number(60.into()));

        assert!(validate_plugin_args("rate_limit", &args).is_ok());
    }

    #[test]
    fn test_validate_rate_limit_defaults() {
        // plugin defaults to 100/60, so an empty args map must validate
        use std::collections::HashMap;

        let args: HashMap<String, Value> = HashMap::new();
        assert!(validate_plugin_args("rate_limit", &args).is_ok());
    }

    #[test]
    fn test_validate_config_with_shorthand_admin_addr() {
        let yaml = r#"
admin:
  enabled: true
  addr: ":8000"
plugins: []
"#;
        let config = loader::load_from_yaml(yaml).unwrap();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_rate_limit_out_of_range() {
        use std::collections::HashMap;

        let mut args = HashMap::new();
        args.insert("max_queries".to_string(), Value::Number(0.into())); // Too small
        args.insert("window_secs".to_string(), Value::Number(60.into()));

        assert!(validate_plugin_args("rate_limit", &args).is_err());
    }

    #[test]
    fn test_validate_ttl_params() {
        use std::collections::HashMap;

        let mut args = HashMap::new();
        args.insert("ttl".to_string(), Value::Number(300.into()));
        args.insert("min".to_string(), Value::Number(30.into()));
        args.insert("max".to_string(), Value::Number(3600.into()));

        assert!(validate_plugin_args("ttl", &args).is_ok());
    }

    #[test]
    fn test_validate_downloader_timeout() {
        use std::collections::HashMap;

        let mut args = HashMap::new();
        args.insert("timeout_secs".to_string(), Value::Number(30.into()));

        assert!(validate_plugin_args("downloader", &args).is_ok());

        // Test out of range
        args.insert("timeout_secs".to_string(), Value::Number(5000.into())); // Too large
        assert!(validate_plugin_args("downloader", &args).is_err());
    }

    #[test]
    fn test_validate_cache_params() {
        use std::collections::HashMap;

        let mut args = HashMap::new();
        args.insert("size".to_string(), Value::Number(1024.into()));
        args.insert("negative_ttl".to_string(), Value::Number(300.into()));

        assert!(validate_plugin_args("cache", &args).is_ok());

        // Test invalid size (0)
        args.insert("size".to_string(), Value::Number(0.into()));
        assert!(validate_plugin_args("cache", &args).is_err());
    }
}
