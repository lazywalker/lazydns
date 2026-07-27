//! Configuration loader
//!
//! Loads and saves configuration from/to files and strings.

use crate::config::Config;
use crate::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

// Compile-once Regex instances to avoid repeated compilation at runtime/tests
static RE_ENV: Lazy<Regex> = Lazy::new(|| Regex::new(r"\$\{([A-Z0-9_]+)(?::-([^}]+))?\}").unwrap());
static RE_INCLUDE: Lazy<Regex> = Lazy::new(|| Regex::new(r"!include\s+([^\s\n]+)").unwrap());
static RE_PLUGIN_ARGS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^PLUGINS_([A-Z0-9_]+)_ARGS_(.+)$").unwrap());

/// Load configuration from a YAML file
///
/// Supports:
/// - Environment variable substitution: ${VAR_NAME} or ${VAR_NAME:-default}
/// - File includes: !include path/to/file.yaml
///
/// # Arguments
///
/// * `path` - Path to the YAML configuration file
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Config> {
    load_from_file_internal(path.as_ref(), &mut HashSet::new())
}

/// Internal implementation with circular include detection
fn load_from_file_internal(path: &Path, visited: &mut HashSet<PathBuf>) -> Result<Config> {
    let canonical_path = path
        .canonicalize()
        .map_err(|e| Error::Config(format!("Failed to resolve path: {}", e)))?;

    // Check for circular includes
    if visited.contains(&canonical_path) {
        return Err(Error::Config(format!(
            "Circular include detected: {}",
            path.display()
        )));
    }
    visited.insert(canonical_path.clone());

    let contents = fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;

    // Substitute environment variables
    let contents = substitute_env_vars(&contents)?;

    // Process includes
    let contents = process_includes(&contents, path.parent(), visited)?;

    load_from_yaml(&contents)
}

/// Substitute environment variables in configuration
///
/// Supports ${VAR_NAME} and ${VAR_NAME:-default_value}
fn substitute_env_vars(content: &str) -> Result<String> {
    let mut result = content.to_string();

    for cap in RE_ENV.captures_iter(content) {
        let full_match = cap.get(0).unwrap().as_str();
        let var_name = cap.get(1).unwrap().as_str();
        let default_value = cap.get(2).map(|m| m.as_str());

        let value = match env::var(var_name) {
            Ok(v) => v,
            Err(_) => {
                if let Some(default) = default_value {
                    default.to_string()
                } else {
                    return Err(Error::Config(format!(
                        "Environment variable {} not found and no default provided",
                        var_name
                    )));
                }
            }
        };

        result = result.replace(full_match, &value);
    }

    Ok(result)
}

/// Process include directives in configuration
///
/// Replaces !include path/to/file.yaml with the file contents
fn process_includes(
    content: &str,
    base_dir: Option<&Path>,
    _visited: &mut HashSet<PathBuf>,
) -> Result<String> {
    let mut result = content.to_string();

    for cap in RE_INCLUDE.captures_iter(content) {
        let full_match = cap.get(0).unwrap().as_str();
        let include_path = cap.get(1).unwrap().as_str();

        // Resolve relative to base directory
        let resolved_path = if let Some(base) = base_dir {
            base.join(include_path)
        } else {
            PathBuf::from(include_path)
        };

        // Read included file
        let included_contents = fs::read_to_string(&resolved_path).map_err(|e| {
            Error::Config(format!(
                "Failed to read included file {}: {}",
                resolved_path.display(),
                e
            ))
        })?;

        // Recursively process the included content
        let included_contents = substitute_env_vars(&included_contents)?;
        let included_contents =
            process_includes(&included_contents, resolved_path.parent(), _visited)?;

        result = result.replace(full_match, &included_contents);
    }

    Ok(result)
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
pub fn load_from_yaml(yaml: &str) -> Result<Config> {
    let mut config: Config = serde_yaml::from_str(yaml)
        .map_err(|e| Error::Config(format!("Failed to parse YAML: {}", e)))?;

    // Apply environment variable overrides (before validation)
    apply_env_overrides(&mut config)?;

    // Validate the loaded configuration
    config.validate()?;

    Ok(config)
}

/// Apply environment variable overrides to the configuration
///
/// Supports two patterns:
/// 1. Top-level fields: `LOG_FORMAT=json` → sets `config.log.format`
/// 2. Plugin args: `PLUGINS_<TAG>_ARGS_<KEY>=value` → sets `plugin.args.<key>`
///
/// Values are parsed as YAML (numbers, booleans, arrays) with string fallback.
fn apply_env_overrides(config: &mut Config) -> Result<()> {
    // Collect all env vars into a HashMap to ensure we see all updates
    // We'll consider both a fresh single-var read and the snapshot to increase the
    // chance of seeing recent transient updates from other tests.
    let env_vars: std::collections::HashMap<String, String> = env::vars().collect();

    // Helper to pick the first non-empty value between a direct env::var read and
    // the snapshot (env_vars). This reduces flakiness when other tests change
    // environment variables concurrently during test runs.
    fn first_non_empty(
        key: &str,
        env_snapshot: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        let direct = std::env::var(key).ok().filter(|s| !s.is_empty());
        let snap = env_snapshot.get(key).cloned().filter(|s| !s.is_empty());
        direct.or(snap)
    }

    // Apply top-level overrides (pick the first non-empty observed value)
    if let Some(val) = first_non_empty("LOG_LEVEL", &env_vars) {
        info!("Applied env override: LOG_LEVEL = {}", val);
        config.log.level = val;
    }
    if let Some(val) = first_non_empty("LOG_FORMAT", &env_vars) {
        info!("Applied env override: LOG_FORMAT = {}", val);
        config.log.format = val;
    }
    if let Some(val) = first_non_empty("LOG_FILE", &env_vars) {
        info!("Applied env override: LOG_FILE = {}", val);
        // Create or update file config with the path
        if let Some(ref mut file_cfg) = config.log.file {
            file_cfg.path = val;
            file_cfg.enabled = true;
        } else {
            config.log.file = Some(crate::config::FileLogConfig {
                enabled: true,
                path: val,
                ..Default::default()
            });
        }
    }
    if let Some(val) = first_non_empty("LOG_CONSOLE", &env_vars) {
        info!("Applied env override: LOG_CONSOLE = {}", val);
        let normalized = val.to_lowercase();
        config.log.console = normalized == "true" || normalized == "1" || normalized == "yes";
    }

    apply_env_overrides_from_snapshot(config, &env_vars)
}

/// Apply environment overrides using a supplied environment snapshot. This is
/// useful for deterministic tests that want to avoid races with other tests
/// changing process globals.
pub(crate) fn apply_env_overrides_from_snapshot(
    config: &mut Config,
    env_snapshot: &std::collections::HashMap<String, String>,
) -> Result<()> {
    use serde_yaml::Value;

    // Support plugin args env overrides with nested paths and indices.
    // Examples supported:
    //   PLUGINS_MYTAG_ARGS_SIZE=2048
    //   PLUGINS_MYTAG_ARGS_JOBS_0_CRON="0 */6 * * *"

    for (key, value_str) in env_snapshot {
        // Handle top-level environment variables
        match key.as_str() {
            "LOG_LEVEL" => {
                if !value_str.is_empty() {
                    info!("Applied env override: LOG_LEVEL = {}", value_str);
                    config.log.level = value_str.clone();
                }
                continue;
            }
            "LOG_FORMAT" => {
                if !value_str.is_empty() {
                    info!("Applied env override: LOG_FORMAT = {}", value_str);
                    config.log.format = value_str.clone();
                }
                continue;
            }
            "LOG_FILE" => {
                if !value_str.is_empty() {
                    info!("Applied env override: LOG_FILE = {}", value_str);
                    // Create or update file config with the path
                    if let Some(ref mut file_cfg) = config.log.file {
                        file_cfg.path = value_str.clone();
                        file_cfg.enabled = true;
                    } else {
                        config.log.file = Some(crate::config::FileLogConfig {
                            enabled: true,
                            path: value_str.clone(),
                            ..Default::default()
                        });
                    }
                }
                continue;
            }
            "LOG_CONSOLE" => {
                if !value_str.is_empty() {
                    info!("Applied env override: LOG_CONSOLE = {}", value_str);
                    let normalized = value_str.to_lowercase();
                    config.log.console =
                        normalized == "true" || normalized == "1" || normalized == "yes";
                }
                continue;
            }

            // Admin server overrides
            "ADMIN_ENABLED" => {
                info!("Applied env override: ADMIN_ENABLED = {}", value_str);
                // parse boolean-like values: true/false, 1/0
                let normalized = value_str.to_lowercase();
                config.admin.enabled =
                    normalized == "true" || normalized == "1" || normalized == "yes";
                continue;
            }

            "ADMIN_ADDR" => {
                info!("Applied env override: ADMIN_ADDR = {}", value_str);
                config.admin.addr = value_str.clone();
                continue;
            }

            // Monitoring server overrides
            "METRICS_ENABLED" => {
                info!("Applied env override: METRICS_ENABLED = {}", value_str);
                // parse boolean-like values: true/false, 1/0
                let normalized = value_str.to_lowercase();
                config.monitoring.enabled =
                    normalized == "true" || normalized == "1" || normalized == "yes";
                continue;
            }

            "METRICS_ADDR" => {
                info!("Applied env override: METRICS_ADDR = {}", value_str);
                config.monitoring.addr = value_str.clone();
                continue;
            }

            _ => {}
        }

        // Handle plugin args: PLUGINS_<TAG>_ARGS_<KEYPATH>=value
        if let Some(caps) = RE_PLUGIN_ARGS.captures(key) {
            let tag_raw = &caps[1];
            let key_path_raw = &caps[2];

            // Normalize tag (lowercase, _ -> -)
            let tag = normalize_identifier(tag_raw);

            // Parse key path into segments (keys or indices)
            #[derive(Debug)]
            enum Segment {
                Key(String),
                Index(usize),
            }

            let mut path: Vec<Segment> = Vec::new();
            for part in key_path_raw.split('_') {
                if let Ok(idx) = part.parse::<usize>() {
                    path.push(Segment::Index(idx));
                } else {
                    path.push(Segment::Key(normalize_identifier(part)));
                }
            }

            // Find matching plugin by effective_name (normalized)
            if let Some(plugin) = config
                .plugins
                .iter_mut()
                .find(|p| normalize_identifier(p.effective_name()) == tag)
            {
                let value = parse_yaml_value(value_str);

                // Ensure args is a mapping
                if !matches!(plugin.args, Value::Mapping(_)) {
                    plugin.args = Value::Mapping(serde_yaml::Mapping::new());
                }

                // Set value into plugin.args following the path
                fn set_path(target: &mut Value, path: &[Segment], value: Value) {
                    use Segment::*;
                    if path.is_empty() {
                        *target = value;
                        return;
                    }

                    let mut cur: &mut Value = target;
                    for i in 0..path.len() {
                        let is_last = i == path.len() - 1;
                        match &path[i] {
                            Key(k) => match cur {
                                Value::Mapping(map) => {
                                    if is_last {
                                        map.insert(Value::String(k.clone()), value);
                                        return;
                                    }
                                    // descend or create
                                    if !map.contains_key(Value::String(k.clone())) {
                                        let next = match &path[i + 1] {
                                            Index(_) => Value::Sequence(vec![]),
                                            _ => Value::Mapping(serde_yaml::Mapping::new()),
                                        };
                                        map.insert(Value::String(k.clone()), next);
                                    }
                                    cur = map.get_mut(Value::String(k.clone())).unwrap();
                                }
                                _ => {
                                    // replace with mapping
                                    *cur = Value::Mapping(serde_yaml::Mapping::new());
                                    if let Value::Mapping(map) = cur {
                                        if is_last {
                                            map.insert(Value::String(k.clone()), value);
                                            return;
                                        }
                                        let next = match &path[i + 1] {
                                            Index(_) => Value::Sequence(vec![]),
                                            _ => Value::Mapping(serde_yaml::Mapping::new()),
                                        };
                                        map.insert(Value::String(k.clone()), next);
                                        cur = map.get_mut(Value::String(k.clone())).unwrap();
                                    }
                                }
                            },
                            Index(idx) => match cur {
                                Value::Sequence(seq) => {
                                    if *idx >= seq.len() {
                                        seq.resize(*idx + 1, Value::Null);
                                    }
                                    if is_last {
                                        seq[*idx] = value;
                                        return;
                                    }
                                    if seq[*idx].is_null() {
                                        seq[*idx] = match &path[i + 1] {
                                            Index(_) => Value::Sequence(vec![]),
                                            _ => Value::Mapping(serde_yaml::Mapping::new()),
                                        };
                                    }
                                    cur = &mut seq[*idx];
                                }
                                _ => {
                                    // replace with sequence
                                    *cur = Value::Sequence(vec![]);
                                    if let Value::Sequence(seq) = cur {
                                        seq.resize(*idx + 1, Value::Null);
                                        if is_last {
                                            seq[*idx] = value;
                                            return;
                                        }
                                        seq[*idx] = match &path[i + 1] {
                                            Index(_) => Value::Sequence(vec![]),
                                            _ => Value::Mapping(serde_yaml::Mapping::new()),
                                        };
                                        cur = &mut seq[*idx];
                                    }
                                }
                            },
                        }
                    }
                }

                set_path(&mut plugin.args, &path, value);

                info!(
                    "Applied plugin env override: {} -> plugin[{}].args path={:?}",
                    key, tag, path
                );
            } else {
                tracing::warn!("Plugin '{}' not found for env override: {}", tag, key);
            }
        }
    }

    Ok(())
}

/// Parse a string value as YAML (numbers, booleans, arrays) with string fallback
fn parse_yaml_value(value_str: &str) -> serde_yaml::Value {
    // Try to parse as YAML
    match serde_yaml::from_str::<serde_yaml::Value>(value_str) {
        Ok(v) => {
            // If it's just a plain string wrapped by YAML, unwrap it
            match v {
                serde_yaml::Value::String(s) => serde_yaml::Value::String(s),
                _ => v,
            }
        }
        Err(_) => {
            // Fall back to treating it as a string
            serde_yaml::Value::String(value_str.to_string())
        }
    }
}

/// Normalize identifiers: lowercase and convert _ to -
fn normalize_identifier(s: &str) -> String {
    s.to_lowercase().replace('_', "-")
}

/// Save configuration to a YAML file
///
/// # Arguments
///
/// * `config` - The configuration to save
/// * `path` - Path to save the YAML configuration file
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn save_to_file<P: AsRef<Path>>(config: &Config, path: P) -> Result<()> {
    let yaml = to_yaml(config)?;

    fs::write(path.as_ref(), yaml)
        .map_err(|e| Error::Config(format!("Failed to write config file: {}", e)))?;

    Ok(())
}

/// Convert configuration to YAML string
///
/// # Arguments
///
/// * `config` - The configuration to convert
///
/// # Errors
///
/// Returns an error if serialization fails.
pub fn to_yaml(config: &Config) -> Result<String> {
    serde_yaml::to_string(config)
        .map_err(|e| Error::Config(format!("Failed to serialize YAML: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_from_yaml_minimal() {
        let yaml = r#"
log:
  level: debug
"#;
        let config = load_from_yaml(yaml).unwrap();
        assert_eq!(config.log.level, "debug");
    }

    #[test]
    fn test_load_from_yaml_full() {
        // Ensure environment overrides do not interfere with this test by
        // setting LOG_LEVEL to an empty value (treated as no override by the
        // loader logic). This reduces flakiness from concurrent tests.
        unsafe {
            std::env::set_var("LOG_LEVEL", "");
        }

        let yaml = r#"
log:
  level: info
  console: true
  file:
    enabled: true
    path: /var/log/app.log
    rotation:
      type: time
      period: daily
plugins:
  - plugin_type: forward
"#;
        let config = load_from_yaml(yaml).unwrap();
        assert_eq!(config.log.level, "info");
        assert!(config.log.console);
        assert!(config.log.file.is_some());
        let file_cfg = config.log.file.as_ref().unwrap();
        assert!(file_cfg.enabled);
        assert_eq!(file_cfg.path, "/var/log/app.log");
        assert_eq!(config.plugins.len(), 1);

        unsafe {
            std::env::remove_var("LOG_LEVEL");
        }
    }

    #[test]
    fn test_load_from_yaml_invalid() {
        let yaml = "invalid: yaml: content: [";
        let result = load_from_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_yaml() {
        let config = Config::new();
        let yaml = to_yaml(&config).unwrap();

        assert!(yaml.contains("log:"));
    }

    #[test]
    fn test_save_and_load_file() {
        let config = Config::new();

        // Create a temporary file
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        // Save config
        save_to_file(&config, &path).unwrap();

        // Load it back
        let loaded = load_from_file(&path).unwrap();

        assert_eq!(config.log, loaded.log);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = load_from_file("/nonexistent/path/config.yaml");
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip() {
        // Clear environment overrides that may interfere with roundtrip test.
        // This reduces flakiness from concurrent tests that set LOG_FORMAT, LOG_LEVEL, and others.
        unsafe {
            env::set_var("LOG_LEVEL", "");
            env::set_var("LOG_FORMAT", "");
            env::set_var("LOG_FILE", "");
            env::set_var("LOG_CONSOLE", "");
        }

        let original = Config::new();
        let yaml = to_yaml(&original).unwrap();
        let loaded = load_from_yaml(&yaml).unwrap();

        assert_eq!(original.log, loaded.log);
    }

    #[test]
    fn test_substitute_env_vars() {
        // Set a test environment variable
        unsafe {
            env::set_var("TEST_VAR", "test_value");
            env::set_var("DNS_PORT", "5353");
        }

        let content = "server: ${TEST_VAR}\nport: ${DNS_PORT}";
        let result = substitute_env_vars(content).unwrap();

        assert_eq!(result, "server: test_value\nport: 5353");

        unsafe {
            env::remove_var("TEST_VAR");
            env::remove_var("DNS_PORT");
        }
    }

    #[test]
    fn test_substitute_env_vars_with_default() {
        // Don't set the variable
        unsafe {
            env::remove_var("MISSING_VAR");
        }

        let content = "value: ${MISSING_VAR:-default_value}";
        let result = substitute_env_vars(content).unwrap();

        assert_eq!(result, "value: default_value");
    }

    #[test]
    fn test_substitute_env_vars_missing_no_default() {
        unsafe {
            env::remove_var("MISSING_VAR");
        }

        let content = "value: ${MISSING_VAR}";
        let result = substitute_env_vars(content);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("MISSING_VAR not found")
        );
    }

    // NOTE: These env override tests must run single-threaded due to environment variable interference
    // Run with: cargo test -- --test-threads=1

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_top_level_log_level() {
        // Use a deterministic snapshot-based approach to avoid races with other
        // tests that may modify process environment variables concurrently.
        let yaml = r#"
log:
  level: info
  format: text
plugins: []
"#;

        let mut config: Config = serde_yaml::from_str(yaml).unwrap();
        let mut snapshot = std::collections::HashMap::new();
        snapshot.insert("LOG_LEVEL".to_string(), "debug".to_string());

        apply_env_overrides_from_snapshot(&mut config, &snapshot).unwrap();
        assert_eq!(
            config.log.level, "debug",
            "LOG_LEVEL should override config"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_top_level_log_format() {
        // Use a deterministic snapshot to avoid races with other tests
        use std::collections::HashMap;

        let yaml = r#"
log:
  level: info
  format: text
plugins: []
"#;

        let mut config: Config = serde_yaml::from_str(yaml).unwrap();
        let mut snapshot = HashMap::new();
        snapshot.insert("LOG_FORMAT".to_string(), "json".to_string());

        apply_env_overrides_from_snapshot(&mut config, &snapshot).unwrap();
        assert_eq!(
            config.log.format, "json",
            "LOG_FORMAT should override config"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_admin_config() {
        unsafe {
            env::set_var("ADMIN_ENABLED", "false");
            env::set_var("ADMIN_ADDR", "127.0.0.1:9999");
        }

        // minimal config with no admin section: env should override defaults
        let yaml = r#"
log:
  level: info
plugins: []
"#;
        let config = load_from_yaml(yaml).unwrap();
        assert!(!config.admin.enabled);
        assert_eq!(config.admin.addr, "127.0.0.1:9999");

        unsafe {
            env::remove_var("ADMIN_ENABLED");
            env::remove_var("ADMIN_ADDR");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_monitoring_config() {
        unsafe {
            env::set_var("METRICS_ENABLED", "true");
            env::set_var("METRICS_ADDR", "127.0.0.1:9999");
        }

        // minimal config with no metrics section: env should override defaults
        let yaml = r#"
log:
  level: info
plugins: []
"#;
        let config = load_from_yaml(yaml).unwrap();
        assert!(config.monitoring.enabled);
        assert_eq!(config.monitoring.addr, "127.0.0.1:9999");

        unsafe {
            env::remove_var("METRICS_ENABLED");
            env::remove_var("METRICS_ADDR");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_plugin_args() {
        unsafe {
            env::set_var("PLUGINS_CACHE_ARGS_SIZE", "2048");
        }

        let yaml = r#"
log:
  level: info
  format: text
plugins:
  - plugin_type: cache
    args:
      size: 1024
"#;
        let config = load_from_yaml(yaml).unwrap();

        // Find the cache plugin and verify args
        let cache_plugin = config
            .plugins
            .iter()
            .find(|p| p.plugin_type == "cache")
            .unwrap();
        if let serde_yaml::Value::Mapping(args) = &cache_plugin.args {
            let size_value = args.get(serde_yaml::Value::String("size".to_string()));
            assert!(size_value.is_some());
            // The value should be 2048 (from env override)
            if let Some(serde_yaml::Value::Number(n)) = size_value {
                assert_eq!(
                    n.as_u64(),
                    Some(2048),
                    "PLUGINS_CACHE_ARGS_SIZE should override config"
                );
            }
        }

        unsafe {
            env::remove_var("PLUGINS_CACHE_ARGS_SIZE");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_plugin_args_string_value() {
        unsafe {
            env::set_var("PLUGINS_ADD_GFWLIST_ARGS_SERVER", "http://10.100.100.1");
        }

        let yaml = r#"
log:
  level: info
  format: text
plugins:
  - plugin_type: add-gfwlist
    args:
      server: http://default.com
"#;
        let config = load_from_yaml(yaml).unwrap();

        let plugin = config
            .plugins
            .iter()
            .find(|p| p.plugin_type == "add-gfwlist")
            .unwrap();
        if let serde_yaml::Value::Mapping(args) = &plugin.args {
            let server_value = args.get(serde_yaml::Value::String("server".to_string()));
            if let Some(serde_yaml::Value::String(s)) = server_value {
                assert_eq!(
                    s, "http://10.100.100.1",
                    "PLUGINS_ADD_GFWLIST_ARGS_SERVER should override config"
                );
            }
        }

        unsafe {
            env::remove_var("PLUGINS_ADD_GFWLIST_ARGS_SERVER");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_jobs_index_cron() {
        // Override jobs[0].cron via env var
        unsafe {
            env::set_var(
                "PLUGINS_AUTO_UPDATE_SCHEDULER_ARGS_JOBS_0_CRON",
                "0 */6 * * *",
            );
        }

        let yaml = r#"
plugins:
  - tag: auto_update_scheduler
    plugin_type: cron
    args:
      jobs: []
"#;
        let config = load_from_yaml(yaml).unwrap();

        let plugin = config
            .plugins
            .iter()
            .find(|p| p.effective_name() == "auto_update_scheduler")
            .unwrap();

        if let serde_yaml::Value::Mapping(args_map) = &plugin.args {
            let jobs_val = args_map.get(serde_yaml::Value::String("jobs".to_string()));
            assert!(jobs_val.is_some());
            if let serde_yaml::Value::Sequence(seq) = jobs_val.unwrap() {
                assert!(!seq.is_empty());
                if let serde_yaml::Value::Mapping(job0) = &seq[0] {
                    let cron_val = job0.get(serde_yaml::Value::String("cron".to_string()));
                    assert!(cron_val.is_some());
                    assert_eq!(cron_val.unwrap().as_str().unwrap(), "0 */6 * * *");
                } else {
                    panic!("jobs[0] is not a mapping");
                }
            } else {
                panic!("jobs is not a sequence");
            }
        } else {
            panic!("plugin.args is not a mapping");
        }

        unsafe {
            env::remove_var("PLUGINS_AUTO_UPDATE_SCHEDULER_ARGS_JOBS_0_CRON");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_numeric_string_parsing() {
        let value_str = "2048";
        let result = parse_yaml_value(value_str);

        // Should parse as a number, not a string
        match result {
            serde_yaml::Value::Number(n) => {
                assert_eq!(n.as_u64(), Some(2048));
            }
            _ => panic!("Expected number, got {:?}", result),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_apply_env_overrides_boolean_parsing() {
        let value_true = parse_yaml_value("true");
        let value_false = parse_yaml_value("false");

        assert_eq!(value_true, serde_yaml::Value::Bool(true));
        assert_eq!(value_false, serde_yaml::Value::Bool(false));
    }

    #[test]
    fn test_apply_env_overrides_array_parsing() {
        let value_array = parse_yaml_value("[8.8.8.8, 1.1.1.1]");

        match value_array {
            serde_yaml::Value::Sequence(seq) => {
                assert_eq!(seq.len(), 2);
            }
            _ => panic!("Expected sequence, got {:?}", value_array),
        }
    }

    #[test]
    fn test_normalize_identifier() {
        assert_eq!(normalize_identifier("ADD_GFWLIST"), "add-gfwlist");
        assert_eq!(normalize_identifier("CACHE_SIZE"), "cache-size");
        assert_eq!(
            normalize_identifier("ENABLE_LAZY_CACHE"),
            "enable-lazy-cache"
        );
    }

    // Cleanup test that runs last and clears all env overrides
    #[test]
    fn test_zzz_cleanup_env_overrides() {
        unsafe {
            env::remove_var("LOG_LEVEL");
            env::remove_var("LOG_FORMAT");
            env::remove_var("LOG_FILE");
            env::remove_var("LOG_ROTATE");
            env::remove_var("PLUGINS_CACHE_ARGS_SIZE");
            env::remove_var("PLUGINS_ADD_GFWLIST_ARGS_SERVER");
        }
    }
}
