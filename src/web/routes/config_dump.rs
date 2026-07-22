//! Read-only configuration dump route for WebUI.
//!
//! Provides `GET /api/config/dump` which returns the currently loaded
//! configuration in a frontend-friendly JSON shape. The data is read from the
//! global [`Config`] held by [`WebState`] (only populated when the server is
//! built with admin capabilities). The handler is read-only: it never mutates
//! configuration.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_yaml::Value;
use std::sync::Arc;
use tracing::warn;

use crate::config::Config;
use crate::config::PluginConfig;
use crate::web::state::WebState;

/// Generic error response (mirrors `routes::admin::ErrorResponse` to stay
/// self-contained without a cross-module dependency for a single shape).
#[derive(Debug, Serialize)]
struct ErrorResponse {
    success: bool,
    error: String,
}

/// Top-level config dump returned by `GET /api/config/dump`.
#[derive(Debug, Serialize)]
pub struct ConfigDumpResponse {
    /// lazydns version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// Logging configuration summary.
    pub log: LogSummary,
    /// Admin API configuration (only present when admin feature is enabled).
    pub admin: Option<AddrSummary>,
    /// Monitoring server configuration.
    pub monitoring: Option<AddrSummary>,
    /// WebUI server configuration (only present when `web` feature is enabled).
    pub web: Option<WebSummary>,
    /// Number of plugins in the configuration.
    pub plugin_count: usize,
    /// Per-plugin summaries.
    pub plugins: Vec<PluginSummary>,
}

/// Logging configuration summary.
#[derive(Debug, Serialize)]
pub struct LogSummary {
    pub level: String,
    pub console: bool,
    pub format: String,
    pub file_enabled: bool,
}

/// A simple enabled + listen-address summary (admin/monitoring).
#[derive(Debug, Serialize)]
pub struct AddrSummary {
    pub enabled: bool,
    pub addr: String,
}

/// WebUI server summary.
#[derive(Debug, Serialize)]
pub struct WebSummary {
    pub enabled: bool,
    pub listen: String,
}

/// Summary of a single plugin instance.
#[derive(Debug, Serialize)]
pub struct PluginSummary {
    /// Effective name (tag if set, otherwise plugin type).
    pub tag: String,
    /// Plugin type identifier.
    pub plugin_type: String,
    /// Raw plugin args converted to JSON; the frontend interprets per type.
    pub args_summary: serde_json::Value,
    /// Whether this plugin is a `sequence` (and thus has `sequence_steps`).
    pub is_sequence: bool,
    /// Parsed sequence steps; `None` for non-sequence plugins.
    pub sequence_steps: Option<Vec<SequenceStepSummary>>,
}

/// One step within a sequence plugin.
#[derive(Debug, Serialize)]
pub struct SequenceStepSummary {
    /// Optional condition expression (e.g. `has_resp`, `qname $list`).
    pub matches: Option<String>,
    /// Executed action (e.g. `$forward`, `accept`, `black_hole 127.0.0.1`).
    pub exec: Option<String>,
}

/// GET /api/config/dump
///
/// Returns the currently loaded configuration as a read-only JSON summary.
/// Responds `503` when the global config is not available (non-admin build
/// or server started without admin capabilities).
pub async fn dump(State(state): State<Arc<WebState>>) -> Response {
    let Some(config_arc) = state.config_arc() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                success: false,
                error: "Configuration not available (requires admin build)".to_string(),
            }),
        )
            .into_response();
    };

    let config = config_arc.read().await;
    let response = build_response(&config);
    Json(response).into_response()
}

/// Build the JSON response from a loaded [`Config`].
///
/// Separated from the handler so it can be unit-tested without constructing a
/// full [`WebState`] (which requires a tokio runtime and event bus).
fn build_response(config: &Config) -> ConfigDumpResponse {
    let plugins: Vec<PluginSummary> = config.plugins.iter().map(summarize_plugin).collect();

    ConfigDumpResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        log: LogSummary {
            level: config.log.level.clone(),
            console: config.log.console,
            format: config.log.format.clone(),
            file_enabled: config.log.is_file_logging_enabled(),
        },
        admin: Some(AddrSummary {
            enabled: config.admin.enabled,
            addr: config.admin.addr.clone(),
        }),
        monitoring: Some(AddrSummary {
            enabled: config.monitoring.enabled,
            addr: config.monitoring.addr.clone(),
        }),
        #[cfg(feature = "web")]
        web: Some(WebSummary {
            enabled: config.web.enabled,
            listen: config.web.listen.clone(),
        }),
        #[cfg(not(feature = "web"))]
        web: None,
        plugin_count: plugins.len(),
        plugins,
    }
}

/// Summarize a single plugin config into a frontend-friendly shape.
fn summarize_plugin(plugin: &PluginConfig) -> PluginSummary {
    let plugin_type = plugin.plugin_type.clone();
    let is_sequence = plugin_type == "sequence";

    let (args_summary, sequence_steps) = if is_sequence {
        (
            serde_json::Value::Null,
            Some(parse_sequence_steps(&plugin.args)),
        )
    } else {
        (yaml_value_to_json(&plugin.args), None)
    };

    PluginSummary {
        tag: plugin.effective_name().to_string(),
        plugin_type,
        args_summary,
        is_sequence,
        sequence_steps,
    }
}

/// Parse a sequence plugin's `args` (a YAML sequence of step mappings) into
/// structured step summaries. Each step may contain `matches` and/or `exec`.
/// Returns an empty vec on any structural mismatch (never panics).
fn parse_sequence_steps(args: &Value) -> Vec<SequenceStepSummary> {
    let Value::Sequence(steps) = args else {
        if !matches!(args, Value::Null | Value::Sequence(_)) {
            warn!(
                args_kind = ?std::mem::discriminant(args),
                "sequence plugin args is not a sequence; returning empty steps"
            );
        }
        return Vec::new();
    };

    steps
        .iter()
        .filter_map(|step| {
            let Value::Mapping(map) = step else {
                return None;
            };
            let matches = map
                .get(Value::String("matches".to_string()))
                .and_then(value_to_string);
            let exec = map
                .get(Value::String("exec".to_string()))
                .and_then(value_to_string);
            // A step must have at least one of matches/exec to be meaningful.
            if matches.is_none() && exec.is_none() {
                return None;
            }
            Some(SequenceStepSummary { matches, exec })
        })
        .collect()
}

/// Convert a [`serde_yaml::Value`] into a [`serde_json::Value`] for the
/// frontend. `serde_yaml::Value` serializes losslessly to JSON. A YAML null
/// (which is also the default when a plugin has no `args:`) is normalized to
/// an empty JSON object so the frontend always sees a record it can index.
fn yaml_value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::json!({}),
        other => serde_json::to_value(other).unwrap_or(serde_json::json!({})),
    }
}

/// Best-effort string extraction from a YAML scalar. Non-scalars return `None`.
fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(type_: &str, args_yaml: &str) -> PluginConfig {
        PluginConfig::new(type_.to_string())
            .with_tag(format!("{}_tag", type_))
            .with_arg(
                "key".to_string(),
                serde_yaml::from_str(args_yaml).unwrap_or(Value::Null),
            )
    }

    #[test]
    fn test_parse_sequence_steps_extracts_matches_and_exec() {
        let yaml = r#"
- exec: $forward
- matches: has_resp
  exec: accept
- matches: qname $reject_list
  exec: black_hole 127.0.0.1
"#;
        let args: Value = serde_yaml::from_str(yaml).unwrap();
        let steps = parse_sequence_steps(&args);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].exec.as_deref(), Some("$forward"));
        assert!(steps[0].matches.is_none());
        assert_eq!(steps[1].matches.as_deref(), Some("has_resp"));
        assert_eq!(steps[1].exec.as_deref(), Some("accept"));
        assert_eq!(steps[2].matches.as_deref(), Some("qname $reject_list"));
        assert_eq!(steps[2].exec.as_deref(), Some("black_hole 127.0.0.1"));
    }

    #[test]
    fn test_parse_sequence_steps_empty_for_non_sequence_args() {
        // Mapping args (not a sequence) → empty steps, no panic.
        let args: Value = serde_yaml::from_str("size: 1024").unwrap();
        let steps = parse_sequence_steps(&args);
        assert!(steps.is_empty());

        // Null args → empty steps.
        assert!(parse_sequence_steps(&Value::Null).is_empty());
    }

    #[test]
    fn test_parse_sequence_steps_skips_empty_steps() {
        let yaml = r#"
- {}
- exec: accept
"#;
        let args: Value = serde_yaml::from_str(yaml).unwrap();
        let steps = parse_sequence_steps(&args);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].exec.as_deref(), Some("accept"));
    }

    #[test]
    fn test_summarize_non_sequence_plugin_returns_args_json() {
        let cfg = plugin("cache", "1024");
        let summary = summarize_plugin(&cfg);
        assert!(!summary.is_sequence);
        assert!(summary.sequence_steps.is_none());
        // The args (containing key: 1024) round-trip to a JSON object.
        assert!(summary.args_summary.is_object());
    }

    #[test]
    fn test_summarize_sequence_plugin_marks_is_sequence() {
        let mut cfg = PluginConfig::new("sequence".to_string()).with_tag("main".to_string());
        cfg = cfg.with_arg(
            "steps".to_string(),
            serde_yaml::from_str("- exec: accept").unwrap(),
        );
        // Override args directly to a sequence (matches real config layout).
        cfg.args = serde_yaml::from_str(
            r#"
- exec: $forward
- exec: accept
"#,
        )
        .unwrap();
        let summary = summarize_plugin(&cfg);
        assert!(summary.is_sequence);
        let steps = summary.sequence_steps.expect("sequence steps");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].exec.as_deref(), Some("$forward"));
    }

    #[test]
    fn test_build_response_includes_log_and_plugins() {
        let config = Config::default();
        let resp = build_response(&config);
        assert_eq!(resp.log.level, "info");
        assert!(!resp.log.console);
        assert_eq!(resp.plugin_count, 0);
        assert!(resp.plugins.is_empty());
        // admin/monitoring are always Some (they have defaults).
        assert!(resp.admin.is_some());
        assert!(resp.monitoring.is_some());
    }
}
