//! Configuration type definitions
//!
//! Common types used across configuration modules.

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;

/// Plugin configuration
///
/// Defines a plugin instance with its settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Plugin tag/name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,

    /// Plugin type
    #[serde(rename = "type", alias = "plugin_type")]
    pub plugin_type: String,

    /// Plugin-specific arguments/configuration
    #[serde(default)]
    pub args: serde_yaml::Value,
}

impl PluginConfig {
    /// Create a new plugin configuration
    ///
    /// # Example
    ///
    /// ```
    /// use lazydns::config::types::PluginConfig;
    ///
    /// let config = PluginConfig::new("forward".to_string());
    /// assert_eq!(config.plugin_type, "forward");
    /// ```
    pub fn new(plugin_type: String) -> Self {
        Self {
            tag: None,
            plugin_type,
            args: Value::Mapping(Mapping::new()),
        }
    }

    /// Set the plugin tag
    ///
    /// # Example
    ///
    /// ```
    /// use lazydns::config::types::PluginConfig;
    ///
    /// let config = PluginConfig::new("forward".to_string())
    ///     .with_tag("my_forward".to_string());
    /// assert_eq!(config.effective_name(), "my_forward");
    /// ```
    pub fn with_tag(mut self, tag: String) -> Self {
        self.tag = Some(tag);
        self
    }

    /// Add an argument value
    ///
    /// # Example
    ///
    /// ```
    /// use lazydns::config::types::PluginConfig;
    ///
    /// let config = PluginConfig::new("forward".to_string())
    ///     .with_arg("key".to_string(), serde_yaml::Value::String("value".to_string()));
    /// assert!(config.effective_args().contains_key("key"));
    /// ```
    pub fn with_arg(mut self, key: String, value: Value) -> Self {
        if let Value::Mapping(ref mut map) = self.args {
            map.insert(Value::String(key), value);
        } else {
            // If args is not a mapping, replace it with a mapping
            let mut map = Mapping::new();
            map.insert(Value::String(key), value);
            self.args = Value::Mapping(map);
        }
        self
    }

    /// Get the effective name (tag, name, or plugin_type in that order)
    ///
    /// # Example
    ///
    /// ```
    /// use lazydns::config::types::PluginConfig;
    ///
    /// let config1 = PluginConfig::new("forward".to_string());
    /// assert_eq!(config1.effective_name(), "forward");
    ///
    /// let config2 = PluginConfig::new("forward".to_string())
    ///     .with_tag("my_forward".to_string());
    /// assert_eq!(config2.effective_name(), "my_forward");
    /// ```
    pub fn effective_name(&self) -> &str {
        self.tag.as_deref().unwrap_or(&self.plugin_type)
    }

    /// Get the effective args as a HashMap.
    ///
    /// Converts the `args` YAML mapping into a flat key-value HashMap. Non-mapping
    /// args (such as for sequence plugins) yield an empty map.
    pub fn effective_args(&self) -> HashMap<String, Value> {
        let mut result = HashMap::new();

        if let Value::Mapping(map) = &self.args {
            for (k, v) in map {
                if let Value::String(key) = k {
                    result.insert(key.clone(), v.clone());
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_config_creation() {
        let config = PluginConfig::new("forward".to_string());

        assert_eq!(config.plugin_type, "forward");
    }

    #[test]
    fn test_plugin_config_builder() {
        let config = PluginConfig::new("forward".to_string()).with_tag("my_forward".to_string());

        assert_eq!(config.effective_name(), "my_forward");
    }

    #[test]
    fn test_plugin_effective_name() {
        let config1 = PluginConfig::new("forward".to_string()).with_tag("forward".to_string());
        assert_eq!(config1.effective_name(), "forward");

        let config2 = PluginConfig::new("forward".to_string()).with_tag("my_forward".to_string());
        assert_eq!(config2.effective_name(), "my_forward");
    }
}
