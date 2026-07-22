//! Fallback plugin
//!
//! Provides fallback mechanism for query processing

use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, RegisterPlugin, Result};
use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Plugin that provides fallback to alternative plugins if primary fails
///
/// # Example
///
/// ```rust,no_run
/// use lazydns::plugins::executable::FallbackPlugin;
/// use lazydns::plugin::Plugin;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let primary: Arc<dyn Plugin> = todo!();
/// # let fallback: Arc<dyn Plugin> = todo!();
/// // Try primary, fallback to alternative if it fails or returns no response
/// let plugin = FallbackPlugin::new(vec![primary, fallback]);
/// # Ok(())
/// # }
/// ```
use std::sync::RwLock;

#[derive(RegisterPlugin, RegisterExecPlugin)]
pub struct FallbackPlugin {
    /// List of resolved plugins to try in order
    plugins: RwLock<Vec<Arc<dyn Plugin>>>,
    /// Pending child plugin names to resolve
    pending: RwLock<Vec<String>>,
    /// Whether to fallback on errors only or also on empty responses
    error_only: bool,
    /// Plugin tag from YAML configuration
    tag: Option<String>,
}

impl FallbackPlugin {
    /// Create a new fallback plugin with already-resolved child plugins
    ///
    /// # Arguments
    ///
    /// * `plugins` - List of plugins to try in order
    pub fn new(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        Self {
            plugins: RwLock::new(plugins),
            pending: RwLock::new(Vec::new()),
            error_only: false,
            tag: None,
        }
    }

    /// Create a fallback plugin that references children by name (to be resolved later)
    pub fn with_names(names: Vec<String>) -> Self {
        Self {
            plugins: RwLock::new(Vec::new()),
            pending: RwLock::new(names),
            error_only: false,
            tag: None,
        }
    }

    /// Set whether to fallback only on errors (not on empty responses)
    pub fn error_only(mut self, error_only: bool) -> Self {
        self.error_only = error_only;
        self
    }

    /// Resolve pending child names using provided plugin registry map
    pub fn resolve_children(&self, registry: &std::collections::HashMap<String, Arc<dyn Plugin>>) {
        let mut pending = self
            .pending
            .write()
            .expect("FallbackPlugin pending RwLock poisoned");
        if pending.is_empty() {
            return;
        }

        let mut resolved = self
            .plugins
            .write()
            .expect("FallbackPlugin plugins RwLock poisoned");

        for name in pending.drain(..) {
            if let Some(p) = registry.get(&name).cloned() {
                debug!(plugin = %name, child = %p.display_name(), "Resolved fallback child");
                resolved.push(p);
            } else {
                warn!(plugin = %name, "Fallback child plugin not found");
            }
        }
    }

    /// Return how many resolved child plugins there are (public helper)
    pub fn resolved_child_count(&self) -> usize {
        self.plugins
            .read()
            .expect("FallbackPlugin plugins RwLock poisoned")
            .len()
    }

    /// Return how many pending child names remain (public helper)
    pub fn pending_child_count(&self) -> usize {
        self.pending
            .read()
            .expect("FallbackPlugin pending RwLock poisoned")
            .len()
    }

    /// Check if we should try the next plugin
    fn should_fallback(&self, ctx: &Context, had_error: bool) -> bool {
        if had_error {
            return true;
        }

        if self.error_only {
            return false;
        }

        // Check if response is empty or missing
        if let Some(response) = ctx.response() {
            response.answers().is_empty()
        } else {
            true
        }
    }
}

impl fmt::Debug for FallbackPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let resolved_count = self.plugins.read().map(|g| g.len()).unwrap_or(0);
        let pending_count = self.pending.read().map(|g| g.len()).unwrap_or(0);
        f.debug_struct("FallbackPlugin")
            .field("resolved_children", &resolved_count)
            .field("pending_children", &pending_count)
            .field("error_only", &self.error_only)
            .finish()
    }
}

#[async_trait]
impl Plugin for FallbackPlugin {
    fn name(&self) -> &str {
        "fallback"
    }

    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        let plugins = {
            self.plugins
                .read()
                .expect("FallbackPlugin plugins RwLock poisoned")
                .clone()
        };

        debug!("Fallback: plugin count = {}", plugins.len());
        debug!(
            "Fallback children: {:?}",
            plugins.iter().map(|p| p.display_name()).collect::<Vec<_>>()
        );
        for (i, plugin) in plugins.iter().enumerate() {
            debug!(
                "Fallback: trying plugin {} (index {})",
                plugin.display_name(),
                i
            );

            let had_error = match plugin.execute(ctx).await {
                Ok(_) => false,
                Err(e) => {
                    warn!(
                        plugin_index = i,
                        plugin_name = plugin.display_name(),
                        error = %e,
                        "Fallback: plugin failed"
                    );
                    true
                }
            };

            // Check if we should try next plugin
            if !self.should_fallback(ctx, had_error) {
                debug!(
                    plugin_index = i,
                    plugin_name = plugin.display_name(),
                    "Fallback: plugin succeeded, stopping"
                );
                return Ok(());
            }

            if i < plugins.len() - 1 {
                debug!(
                    plugin_index = i,
                    plugin_name = plugin.display_name(),
                    "Fallback: trying next plugin"
                );
            }
        }

        debug!("Fallback: all plugins attempted");
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn init(config: &crate::config::types::PluginConfig) -> Result<std::sync::Arc<dyn Plugin>> {
        // Read primary/secondary names from args and create plugin with pending names
        let args = config.effective_args();
        let primary = args
            .get("primary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let secondary = args
            .get("secondary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        info!(
            "Creating fallback plugin (pending references): primary={}, secondary={}",
            primary, secondary
        );

        let mut names = Vec::new();
        if !primary.is_empty() {
            names.push(primary);
        }
        if !secondary.is_empty() {
            names.push(secondary);
        }

        // Honor the documented `error_only` option (default: false). When true,
        // fallback only triggers on plugin errors, not on empty responses.
        let error_only = args
            .get("error_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok(Arc::new(FallbackPlugin {
            plugins: RwLock::new(Vec::new()),
            pending: RwLock::new(names),
            error_only,
            tag: config.tag.clone(),
        }))
    }
}

impl ExecPlugin for FallbackPlugin {
    /// Parse a quick configuration string for fallback plugin.
    ///
    /// Accepts comma-separated list of plugin names to try in order.
    /// Examples: "primary,secondary", "upstream1,upstream2,upstream3"
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != "fallback" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'fallback'",
                prefix
            )));
        }

        // Parse comma-separated plugin names
        let names: Vec<String> = exec_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if names.is_empty() {
            return Err(crate::Error::Config(
                "fallback plugin requires at least one plugin name".to_string(),
            ));
        }

        let plugin = FallbackPlugin::with_names(names);
        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;

    #[derive(Debug)]
    struct TestPlugin {
        name: String,
        should_fail: bool,
        should_empty: bool,
    }

    #[async_trait]
    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        async fn execute(&self, ctx: &mut Context) -> Result<()> {
            if self.should_fail {
                return Err(crate::Error::Plugin("test error".to_string()));
            }

            if !self.should_empty {
                let mut response = Message::new();
                use crate::dns::types::{RecordClass, RecordType};
                use crate::dns::{RData, ResourceRecord};

                response.add_answer(ResourceRecord::new(
                    "example.com",
                    RecordType::A,
                    RecordClass::IN,
                    300,
                    RData::A("192.0.2.1".parse().unwrap()),
                ));
                ctx.set_response(Some(response));
            }

            Ok(())
        }
    }

    #[tokio::test]
    async fn test_fallback_first_succeeds() {
        let primary = Arc::new(TestPlugin {
            name: "primary".to_string(),
            should_fail: false,
            should_empty: false,
        });
        let fallback_plugin = FallbackPlugin::new(vec![primary]);

        let mut ctx = Context::new(Message::new());
        fallback_plugin.execute(&mut ctx).await.unwrap();

        // Should have response from primary
        assert!(ctx.response().is_some());
        assert!(!ctx.response().unwrap().answers().is_empty());
    }

    #[tokio::test]
    async fn test_fallback_on_error() {
        let primary = Arc::new(TestPlugin {
            name: "primary".to_string(),
            should_fail: true,
            should_empty: false,
        });
        let secondary = Arc::new(TestPlugin {
            name: "secondary".to_string(),
            should_fail: false,
            should_empty: false,
        });

        let fallback_plugin = FallbackPlugin::new(vec![primary, secondary]);

        let mut ctx = Context::new(Message::new());
        fallback_plugin.execute(&mut ctx).await.unwrap();

        // Should have response from secondary
        assert!(ctx.response().is_some());
        assert!(!ctx.response().unwrap().answers().is_empty());
    }

    #[tokio::test]
    async fn test_fallback_on_empty_response() {
        let primary = Arc::new(TestPlugin {
            name: "primary".to_string(),
            should_fail: false,
            should_empty: true, // Returns empty response
        });
        let secondary = Arc::new(TestPlugin {
            name: "secondary".to_string(),
            should_fail: false,
            should_empty: false,
        });

        let fallback_plugin = FallbackPlugin::new(vec![primary, secondary]);

        let mut ctx = Context::new(Message::new());
        fallback_plugin.execute(&mut ctx).await.unwrap();

        // Should have response from secondary
        assert!(ctx.response().is_some());
        assert!(!ctx.response().unwrap().answers().is_empty());
    }

    #[tokio::test]
    async fn test_fallback_error_only_mode() {
        let primary = Arc::new(TestPlugin {
            name: "primary".to_string(),
            should_fail: false,
            should_empty: true, // Returns empty response
        });
        let secondary = Arc::new(TestPlugin {
            name: "secondary".to_string(),
            should_fail: false,
            should_empty: false,
        });

        let fallback_plugin = FallbackPlugin::new(vec![primary, secondary]).error_only(true); // Only fallback on errors, not empty responses

        let mut ctx = Context::new(Message::new());
        fallback_plugin.execute(&mut ctx).await.unwrap();

        // Should NOT fallback to secondary (empty response is OK in error_only mode)
        // The response might be empty or have the empty response from primary
        // In error_only mode, we don't fallback on empty responses
    }

    #[test]
    fn test_exec_plugin_quick_setup() {
        // Test that ExecPlugin::quick_setup works correctly
        let plugin =
            <FallbackPlugin as ExecPlugin>::quick_setup("fallback", "primary,secondary").unwrap();
        assert_eq!(plugin.name(), "fallback");

        // Test single plugin
        let plugin = <FallbackPlugin as ExecPlugin>::quick_setup("fallback", "upstream").unwrap();
        assert_eq!(plugin.name(), "fallback");

        // Test invalid prefix
        let result = <FallbackPlugin as ExecPlugin>::quick_setup("invalid", "primary");
        assert!(result.is_err());

        // Test empty exec_str
        let result = <FallbackPlugin as ExecPlugin>::quick_setup("fallback", "");
        assert!(result.is_err());

        // Test with spaces
        let plugin =
            <FallbackPlugin as ExecPlugin>::quick_setup("fallback", " primary , secondary ")
                .unwrap();
        assert_eq!(plugin.name(), "fallback");
    }
}
