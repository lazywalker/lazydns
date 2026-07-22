//! Plugin trait definitions
//!
//! Defines the core Plugin trait that all plugins must implement.

use crate::Result;
use crate::config::PluginConfig;
use crate::plugin::Context;
use async_trait::async_trait;
use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

/// Core plugin trait
///
/// All DNS query processing plugins must implement this trait.
/// Plugins receive a mutable context containing the DNS query and can
/// modify it or add a response.
///
/// # Example
///
/// ```rust
/// use lazydns::plugin::{Plugin, Context};
/// use lazydns::Result;
/// use async_trait::async_trait;
///
/// #[derive(Debug)]
/// struct LogPlugin;
///
/// #[async_trait]
/// impl Plugin for LogPlugin {
///     async fn execute(&self, ctx: &mut Context) -> Result<()> {
///         println!("Processing query: {:?}", ctx.request().questions());
///         Ok(())
///     }
///
///     fn name(&self) -> &str {
///         "log"
///     }
/// }
/// ```
#[async_trait]
pub trait Plugin: Send + Sync + Debug + Any + 'static {
    /// Execute the plugin logic
    ///
    /// This method is called to process a DNS query. The plugin can:
    /// - Read the query from the context
    /// - Modify the query
    /// - Set a response
    /// - Add metadata for other plugins
    ///
    /// # Arguments
    ///
    /// * `ctx` - The execution context containing the DNS query and response
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success, or an error if plugin execution fails.
    async fn execute(&self, _ctx: &mut Context) -> Result<()> {
        Ok(())
    }

    /// Get the plugin name
    ///
    /// Returns a unique identifier for this plugin.
    fn name(&self) -> &str;

    /// Get the plugin tag (from YAML configuration)
    ///
    /// Returns the tag value from the plugin's YAML configuration if set,
    /// otherwise None. This allows distinguishing between multiple instances
    /// of the same plugin type.
    fn tag(&self) -> Option<&str> {
        None
    }

    /// Get the effective display name for logging
    ///
    /// Returns the tag if set, otherwise falls back to the plugin name.
    /// This is useful for logging to distinguish between multiple instances
    /// of the same plugin type.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use lazydns::plugin::Plugin;
    /// # fn example(plugin: &dyn Plugin) {
    /// // Instead of: plugin.tag().unwrap_or(plugin.name())
    /// // Use: plugin.display_name()
    /// println!("Executing plugin: {}", plugin.display_name());
    /// # }
    /// ```
    fn display_name(&self) -> &str {
        self.tag().unwrap_or(self.name())
    }

    /// Check if this plugin should execute
    ///
    /// Plugins can override this to provide conditional execution logic.
    /// By default, plugins always execute.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The execution context
    ///
    /// # Returns
    ///
    /// Returns `true` if the plugin should execute, `false` otherwise.
    fn should_execute(&self, _ctx: &Context) -> bool {
        true
    }

    /// Get the plugin as Any for downcasting
    fn as_any(&self) -> &dyn Any {
        // This is a default implementation that won't work for downcasting
        // Concrete implementations should override this
        &()
    }

    /// factory method to init a plugin instance from configuration.
    ///
    /// Default implementation returns an error indicating no builder is
    /// provided for this plugin type. Implementations that support
    /// configuration-based construction should override this method.
    fn init(_config: &PluginConfig) -> Result<Arc<dyn Plugin>>
    where
        Self: Sized,
    {
        Err(crate::Error::Config(format!(
            "no builder for plugin {}",
            std::any::type_name::<Self>()
        )))
    }

    /// Optional aliases for plugin type names.
    ///
    /// Returns a `'static` slice of alias names (for example `&["sinkhole", "black_hole"]`).
    ///
    /// These aliases are used by the plugin factory and exec-plugin registration
    /// system: each alias is registered as an alternative canonical name that
    /// can be used in configuration or quick-setup strings. For exec plugins,
    /// the `quick_setup(prefix, exec_str)` implementation should accept either
    /// the canonical name (`plugin_type()`) or any of the aliases returned by
    /// this method.
    ///
    /// The default implementation returns an empty slice `&[]`.
    fn aliases() -> &'static [&'static str]
    where
        Self: Sized,
    {
        &[]
    }

    /// Bridge to `Shutdown` trait implementations.
    ///
    /// Plugins that implement `Shutdown` should override this to return
    /// `Some(self)` so that the builder or shutdown coordinator can invoke
    /// the concrete `Shutdown::shutdown` implementation. The default
    /// implementation returns `None` indicating no shutdown work is required.
    fn as_shutdown(&self) -> Option<&dyn Shutdown> {
        None
    }

    /// Spawn background task for this plugin if it requires one.
    ///
    /// Plugins that need periodic background work (cleanup, refresh, etc.)
    /// should override this method to spawn their background task.
    /// The default implementation returns `None` indicating no background task.
    ///
    /// # Returns
    ///
    /// `Some(JoinHandle)` if a background task was spawned, `None` otherwise.
    fn spawn_background_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        None
    }
}

/// Trait for executable plugins that support quick setup from strings
///
/// Exec plugins are a special category of plugins that can be quickly
/// instantiated from simple string configurations, typically used in
/// exec-style configurations similar to mosdns.
pub trait ExecPlugin: Plugin {
    /// Quick setup from prefix and exec string
    ///
    /// # Arguments
    /// * `prefix` - The plugin type prefix (e.g., "ttl")
    /// * `exec_str` - The execution configuration string (e.g., "60" or "30-300")
    ///
    /// # Returns
    /// A configured plugin instance, or an error if setup fails.
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>>
    where
        Self: Sized;
}

/// Shutdown trait for plugins that need cleanup
///
/// Plugins that require graceful shutdown (e.g., closing connections,
/// flushing caches, stopping background tasks) should implement this trait.
#[async_trait]
pub trait Shutdown: Send + Sync {
    /// Perform graceful shutdown
    ///
    /// This method is called when the application is shutting down.
    /// Plugins should clean up resources, flush data, and stop background tasks.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful shutdown, or an error if cleanup fails.
    async fn shutdown(&self) -> Result<()>;
}

/// Trait for plugins that spawn periodic background cleanup tasks
///
/// Plugins like CachePlugin, RateLimitPlugin, and ReverseLookupPlugin often need
/// to periodically clean up expired entries. This trait provides a unified interface
/// for spawning such background tasks.
///
/// # Example
///
/// ```rust,ignore
/// use lazydns::plugin::BackgroundTask;
/// use std::sync::Arc;
///
/// let plugin = Arc::new(MyPlugin::new());
/// let handle = plugin.spawn_background_task();
/// // Later: handle.abort() to stop the task
/// ```
pub trait BackgroundTask: Send + Sync {
    /// Get the interval between background task runs
    fn background_task_interval(&self) -> std::time::Duration;

    /// Perform a single iteration of the background task
    ///
    /// This method is called periodically by the spawned task.
    /// It should perform cleanup, maintenance, or other periodic work.
    fn run_background_task(&self);

    /// Get the name of this background task for logging
    fn background_task_name(&self) -> &str;

    /// Spawn the background cleanup task
    ///
    /// Returns a JoinHandle that can be used to abort the task.
    /// The default implementation spawns a tokio task that calls
    /// `run_background_task` at the interval specified by `background_task_interval`.
    fn spawn_background_task(self: Arc<Self>) -> tokio::task::JoinHandle<()>
    where
        Self: 'static,
    {
        let interval = self.background_task_interval();
        let name = self.background_task_name().to_string();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(interval);
            loop {
                timer.tick().await;
                tracing::trace!(task = %name, "Running background task");
                self.run_background_task();
            }
        })
    }
}

/// Matcher trait for plugins that can match against DNS data
#[async_trait]
pub trait Matcher: Plugin {
    /// Check if the context matches this matcher
    fn matches_context(&self, ctx: &Context) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;

    #[derive(Debug)]
    struct TestPlugin {
        name: String,
    }

    #[async_trait]
    impl Plugin for TestPlugin {
        async fn execute(&self, _ctx: &mut Context) -> Result<()> {
            Ok(())
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_plugin_trait() {
        let plugin = TestPlugin {
            name: "test".to_string(),
        };

        assert_eq!(plugin.name(), "test");

        let request = Message::new();
        let mut ctx = Context::new(request);
        assert!(plugin.should_execute(&ctx));
        assert!(plugin.execute(&mut ctx).await.is_ok());
    }
}
