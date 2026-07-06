//! Audit configuration types

use serde::de::{self, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Audit logging configuration
///
/// Controls DNS query logging and security event tracking.
/// Global buffer/rotation settings apply to both query_log and security_events
/// unless overridden at the individual log level.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AuditConfig {
    /// Enable audit logging (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Enable writing logs to filesystem (default: true)
    /// When false, only event bus publishing is active for real-time WebUI streaming
    #[serde(default = "default_log_to_file")]
    pub log_to_file: bool,

    /// Global log buffer size before flush (applies to both logs, default: 100)
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    /// Global maximum file size before rotation (applies to both logs, default: 100MB)
    #[serde(
        default = "default_max_file_size",
        deserialize_with = "deserialize_max_file_size"
    )]
    pub max_file_size: u64,

    /// Global number of rotated files to keep (applies to both logs, default: 10)
    #[serde(default = "default_max_files")]
    pub max_files: u32,

    /// Query logging configuration
    #[serde(default)]
    pub query_log: Option<QueryLogConfig>,

    /// Security event logging configuration
    #[serde(default)]
    pub security_events: Option<SecurityEventConfig>,
}

/// Query log configuration
///
/// Controls what DNS queries are logged and where.
/// Buffer and rotation settings inherit from AuditConfig if not specified.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryLogConfig {
    /// Enable writing query logs to filesystem (overrides global setting if specified)
    #[serde(default)]
    pub log_to_file: Option<bool>,

    /// Path to query log file
    #[serde(default = "default_query_log_path")]
    pub path: String,

    /// Output format: "json" or "text" (default: json)
    #[serde(default = "default_format")]
    pub format: String,

    /// Sampling rate (0.0 to 1.0): fraction of queries to log (default: 1.0 = all)
    /// Use 0.1 to log 10% of queries, reducing I/O overhead
    #[serde(default = "default_sampling_rate")]
    pub sampling_rate: f64,

    /// Include response details in log entries (default: true)
    #[serde(default = "default_include_response")]
    pub include_response: bool,

    /// Include client IP in log entries (default: true)
    #[serde(default = "default_include_client_ip")]
    pub include_client_ip: bool,

    /// Exclude DNS-SD discovery queries (default: true)
    /// Filters out queries matching _dns-sd._udp patterns to reduce noise
    #[serde(default = "default_exclude_dns_sd")]
    pub exclude_dns_sd: bool,

    /// Log buffer size before flush (overrides global audit.buffer_size if set)
    #[serde(default)]
    pub buffer_size: Option<usize>,

    /// Maximum file size in bytes before rotation (overrides global audit.max_file_size if set)
    #[serde(default, deserialize_with = "deserialize_optional_max_file_size")]
    pub max_file_size: Option<u64>,

    /// Number of rotated files to keep (overrides global audit.max_files if set)
    #[serde(default)]
    pub max_files: Option<u32>,
}

fn default_query_log_path() -> String {
    "queries.log".to_string()
}

fn default_format() -> String {
    "json".to_string()
}

fn default_sampling_rate() -> f64 {
    1.0
}

fn default_include_response() -> bool {
    true
}

fn default_include_client_ip() -> bool {
    true
}

fn default_exclude_dns_sd() -> bool {
    true
}

fn default_log_to_file() -> bool {
    true
}

fn default_buffer_size() -> usize {
    100
}

fn default_max_file_size() -> u64 {
    100 * 1024 * 1024 // 100MB
}

fn default_max_files() -> u32 {
    10
}

impl Default for QueryLogConfig {
    fn default() -> Self {
        Self {
            log_to_file: None,
            path: default_query_log_path(),
            format: default_format(),
            sampling_rate: default_sampling_rate(),
            include_response: default_include_response(),
            include_client_ip: default_include_client_ip(),
            exclude_dns_sd: default_exclude_dns_sd(),
            buffer_size: None,
            max_file_size: None,
            max_files: None,
        }
    }
}

/// Security event logging configuration
///
/// Controls security event logging. Buffer and rotation settings inherit
/// from AuditConfig if not specified.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityEventConfig {
    /// Enable security event logging (default: true when present)
    #[serde(default = "default_security_enabled")]
    pub enabled: bool,

    /// Enable writing security events to filesystem (overrides global setting if specified)
    #[serde(default)]
    pub log_to_file: Option<bool>,

    /// Path to security event log file
    #[serde(default = "default_security_log_path")]
    pub path: String,

    /// Events to track (empty = all events)
    #[serde(default)]
    pub events: Vec<String>,

    /// Include full query details (default: true)
    #[serde(default = "default_include_query_details")]
    pub include_query_details: bool,

    /// Log buffer size before flush (overrides global audit.buffer_size if set)
    #[serde(default)]
    pub buffer_size: Option<usize>,

    /// Maximum file size in bytes before rotation (overrides global audit.max_file_size if set)
    #[serde(default, deserialize_with = "deserialize_optional_max_file_size")]
    pub max_file_size: Option<u64>,

    /// Number of rotated files to keep (overrides global audit.max_files if set)
    #[serde(default)]
    pub max_files: Option<u32>,
}

fn default_security_enabled() -> bool {
    true
}

fn default_security_log_path() -> String {
    "security.log".to_string()
}

fn default_include_query_details() -> bool {
    true
}

impl Default for SecurityEventConfig {
    fn default() -> Self {
        Self {
            enabled: default_security_enabled(),
            log_to_file: None,
            path: default_security_log_path(),
            events: Vec::new(), // empty = all events
            include_query_details: default_include_query_details(),
            buffer_size: None,
            max_file_size: None,
            max_files: None,
        }
    }
}

/// Parse a size string with optional units (K, M, G, case-insensitive)
fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty size string".to_string());
    }

    // Collect char indices so that all slicing is done on char boundaries.
    // Mixing byte slicing (`s[len-2..]`) with char iteration would panic on a
    // trailing multi-byte UTF-8 character (e.g. a non-ASCII unit suffix).
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let last = chars.last().unwrap().1;

    let (num_str, unit) = if last.is_ascii_alphabetic() {
        // Two-character unit (KB/MB/GB) vs single-char unit (K/M/G).
        if chars.len() >= 2 {
            let two_char_unit: String = chars[chars.len() - 2..]
                .iter()
                .map(|(_, c)| c.to_ascii_uppercase())
                .collect();
            if matches!(two_char_unit.as_str(), "KB" | "MB" | "GB") {
                let unit_end = chars[chars.len() - 2].0;
                (
                    &s[..unit_end],
                    chars[chars.len() - 2].1.to_ascii_uppercase(),
                )
            } else {
                let unit_start = chars[chars.len() - 1].0;
                (&s[..unit_start], last.to_ascii_uppercase())
            }
        } else {
            // Single character total: the whole string is the unit char, no number.
            return Err(format!("invalid size string: {}", s));
        }
    } else {
        (s, 'B')
    };

    let num: u64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("invalid number: {}", num_str))?;

    let multiplier = match unit {
        'B' => 1,
        'K' => 1024,
        'M' => 1024 * 1024,
        'G' => 1024 * 1024 * 1024,
        _ => return Err(format!("invalid unit: {}, supported: K, M, G", unit)),
    };

    num.checked_mul(multiplier)
        .ok_or_else(|| "size too large".to_string())
}

/// Custom deserializer for max_file_size to support human-readable strings
fn deserialize_max_file_size<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: de::Deserializer<'de>,
{
    struct MaxFileSizeVisitor;

    impl<'de> Visitor<'de> for MaxFileSizeVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a number or a string with units (e.g., 100K, 10M, 1G)")
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v)
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v < 0 {
                return Err(de::Error::custom("negative file size"));
            }
            Ok(v as u64)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_size(v).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(MaxFileSizeVisitor)
}

/// Custom deserializer for optional max_file_size to support human-readable strings
fn deserialize_optional_max_file_size<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: de::Deserializer<'de>,
{
    struct OptionalMaxFileSizeVisitor;

    impl<'de> Visitor<'de> for OptionalMaxFileSizeVisitor {
        type Value = Option<u64>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a number or a string with units (e.g., 100K, 10M, 1G)")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: de::Deserializer<'de>,
        {
            deserialize_max_file_size(deserializer).map(Some)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(v))
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if v < 0 {
                return Err(de::Error::custom("negative file size"));
            }
            Ok(Some(v as u64))
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            parse_size(v).map(Some).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_option(OptionalMaxFileSizeVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_config_default() {
        let config = AuditConfig::default();
        assert!(!config.enabled);
        assert!(config.query_log.is_none());
        assert!(config.security_events.is_none());
    }

    #[test]
    fn test_query_log_config_default() {
        let config = QueryLogConfig::default();
        assert_eq!(config.path, "queries.log");
        assert_eq!(config.format, "json");
        assert!((config.sampling_rate - 1.0).abs() < f64::EPSILON);
        assert!(config.include_response);
        assert!(config.include_client_ip);
    }

    #[test]
    fn test_security_event_config_default() {
        let config = SecurityEventConfig::default();
        assert!(config.enabled);
        assert_eq!(config.path, "security.log");
        assert!(config.events.is_empty());
    }

    #[test]
    fn test_audit_config_deserialize() {
        let yaml = r#"
enabled: true
query_log:
  path: /var/log/queries.log
  sampling_rate: 0.1
security_events:
  enabled: true
  events:
    - rate_limit_exceeded
    - blocked_domain_query
"#;
        let config: AuditConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.enabled);

        let query_log = config.query_log.unwrap();
        assert_eq!(query_log.path, "/var/log/queries.log");
        assert!((query_log.sampling_rate - 0.1).abs() < f64::EPSILON);

        let security = config.security_events.unwrap();
        assert!(security.enabled);
        assert_eq!(security.events.len(), 2);
    }

    #[test]
    fn test_max_file_size_parsing() {
        let yaml = r#"
query_log:
  max_file_size: 10M
"#;
        let config: AuditConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.query_log.unwrap().max_file_size,
            Some(10 * 1024 * 1024)
        );

        let yaml = r#"
query_log:
  max_file_size: 100K
"#;
        let config: AuditConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.query_log.unwrap().max_file_size, Some(100 * 1024));

        let yaml = r#"
query_log:
  max_file_size: 1G
"#;
        let config: AuditConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.query_log.unwrap().max_file_size,
            Some(1024 * 1024 * 1024)
        );
    }

    /// Regression test for a char-boundary panic in `parse_size`.
    ///
    /// The old implementation mixed byte slicing (`s[len-2..]`) with `chars()`
    /// iteration, so a trailing multi-byte UTF-8 character (e.g. a CJK suffix)
    /// sliced into the middle of a character and panicked with
    /// "byte index ... is not a char boundary". The rewritten parser operates
    /// purely on char indices and must reject such input gracefully.
    #[test]
    fn test_parse_size_rejects_multibyte_suffix_without_panic() {
        // Trailing CJK character: must NOT panic, must be an error.
        let result = parse_size("100M中文");
        assert!(result.is_err(), "multibyte suffix should be rejected");

        // Sanity: valid inputs still parse correctly.
        assert_eq!(parse_size("10M").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("100K").unwrap(), 100 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("5MB").unwrap(), 5 * 1024 * 1024);
    }

    /// A size string consisting of a single alphabetic character is not a
    /// valid size (no numeric part) and must be rejected, not panicked on.
    #[test]
    fn test_parse_size_single_char_is_error() {
        assert!(parse_size("M").is_err());
        assert!(parse_size("").is_err());
    }
}
