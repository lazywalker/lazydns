//! Plugin factory registration system
//!
//! Manages plugin type registration and factory lookup.
//! This module provides the infrastructure for registering plugin factories
//! and creating plugin instances from configuration.

use crate::config::types::PluginConfig;
use crate::plugin::Plugin;
use std::sync::Arc;
use tracing::debug;

/// Plugin factory trait for self-registering plugins
///
/// Implement this trait directly on your plugin type to enable automatic
/// registration without needing a separate factory struct.
///
/// # Example
///
/// ```ignore
/// use lazydns::plugin::factory::PluginFactory;
/// use lazydns::config::types::PluginConfig;
/// use std::sync::Arc;
/// use async_trait::async_trait;
///
/// #[derive(Debug)]
/// struct MyPlugin { config: String }
///
/// impl PluginFactory for MyPlugin {
///     fn create(&self, config: &PluginConfig) -> crate::Result<Arc<dyn lazydns::plugin::Plugin>> {
///         // Create plugin from config
///         Ok(Arc::new(Self { config: "default".to_string() }))
///     }
///
///     fn plugin_type(&self) -> &'static str {
///         "my_plugin"
///     }
///
///     fn aliases(&self) -> &'static [&'static str] {
///         &[]
///     }
/// }
/// ```
pub trait PluginFactory: Send + Sync {
    fn create(&self, config: &PluginConfig) -> crate::Result<Arc<dyn Plugin>>;
    fn plugin_type(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str];
}

/// Exec plugin factory trait for exec plugins
///
/// Similar to PluginFactory but specifically for exec plugins that implement ExecPlugin.
pub trait ExecPluginFactory: Send + Sync {
    fn create(&self, prefix: &str, exec_str: &str) -> crate::Result<Arc<dyn Plugin>>;
    fn plugin_type(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str];
}

/// Distributed slice for collecting plugin factories
#[linkme::distributed_slice]
pub static PLUGIN_FACTORIES_SLICE: [fn() -> Arc<dyn PluginFactory>];

/// Distributed slice for collecting exec plugin factories
#[linkme::distributed_slice]
pub static EXEC_PLUGIN_FACTORIES_SLICE: [fn() -> Arc<dyn ExecPluginFactory>];

/// Get a plugin factory by type name
pub fn get_plugin_factory(plugin_type: &str) -> Option<Arc<dyn PluginFactory>> {
    // Search through the distributed slice for a factory with matching type
    for factory_constructor in PLUGIN_FACTORIES_SLICE {
        let factory = factory_constructor();
        if factory.plugin_type() == plugin_type {
            return Some(factory);
        }
        // Also check aliases
        for alias in factory.aliases() {
            if *alias == plugin_type {
                return Some(factory);
            }
        }
    }
    None
}

/// Get an exec plugin factory by type name
pub fn get_exec_plugin_factory(plugin_type: &str) -> Option<Arc<dyn ExecPluginFactory>> {
    // Search through the distributed slice for a factory with matching type
    for factory_constructor in EXEC_PLUGIN_FACTORIES_SLICE {
        let factory = factory_constructor();
        if factory.plugin_type() == plugin_type {
            return Some(factory);
        }
        // Also check aliases
        for alias in factory.aliases() {
            if *alias == plugin_type {
                return Some(factory);
            }
        }
    }
    None
}

/// Get all registered plugin types
pub fn get_all_plugin_types() -> Vec<String> {
    let mut types: Vec<String> = PLUGIN_FACTORIES_SLICE
        .iter()
        .flat_map(|factory_constructor| {
            let factory = factory_constructor();
            let mut names = vec![factory.plugin_type().to_string()];
            names.extend(factory.aliases().iter().map(|s| s.to_string()));
            names
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    types.sort();
    types
}

/// Get all registered exec plugin types
pub fn get_all_exec_plugin_types() -> Vec<String> {
    let mut types: Vec<String> = EXEC_PLUGIN_FACTORIES_SLICE
        .iter()
        .flat_map(|factory_constructor| {
            let factory = factory_constructor();
            let mut names = vec![factory.plugin_type().to_string()];
            names.extend(factory.aliases().iter().map(|s| s.to_string()));
            names
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    types.sort();
    types
}

/// Initialize all plugin and exec plugin factories
/// This function should be called early in program initialization.
pub fn init() {
    initialize_all_plugin_factories();
    initialize_all_exec_plugin_factories();
}

/// Initialize all plugin factories
///
/// This function ensures that all plugin factory registrations are triggered.
/// It should be called early in program initialization, before any plugins
/// are created from configuration.
///
/// The function works by iterating over all factories in the distributed slice
/// and registering them automatically.
///
/// # Example
///
/// ```rust
/// use lazydns::plugin::factory::initialize_all_plugin_factories;
///
/// // Initialize plugin factories before loading config
/// initialize_all_plugin_factories();
/// ```
pub fn initialize_all_plugin_factories() {
    use once_cell::sync::OnceCell;

    static INIT: OnceCell<()> = OnceCell::new();

    INIT.get_or_init(|| {
        // Force initialization by accessing the distributed slice
        // This ensures all linkme-registered factories are available
        let _ = PLUGIN_FACTORIES_SLICE.len();

        let types = get_all_plugin_types();
        debug!("Initialized {} plugin factories: {:?}", types.len(), types);
    });
}

/// Initialize all exec plugin factories
///
/// Similar to initialize_all_plugin_factories but for exec plugins.
fn initialize_all_exec_plugin_factories() {
    use once_cell::sync::OnceCell;

    static EXEC_INIT: OnceCell<()> = OnceCell::new();

    EXEC_INIT.get_or_init(|| {
        // Force initialization by accessing the distributed slice
        // This ensures all linkme-registered factories are available
        let _ = EXEC_PLUGIN_FACTORIES_SLICE.len();

        let types = get_all_exec_plugin_types();
        debug!(
            "Initialized {} exec plugin factories: {:?}",
            types.len(),
            types
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::PluginConfig;
    use crate::plugin::Context;
    use crate::{RegisterExecPlugin, RegisterPlugin};
    use async_trait::async_trait;
    use std::sync::Arc;

    // Test that plugins deriving #[derive(RegisterPlugin)] are discoverable
    #[test]
    fn test_derive_register_plugin_discovery() {
        // Define a test plugin type and derive registration
        #[derive(Debug, RegisterPlugin)]
        struct MyMacroPlugin;

        #[async_trait]
        impl crate::plugin::traits::Plugin for MyMacroPlugin {
            async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
                Ok(())
            }

            fn name(&self) -> &str {
                "my_macro_plugin"
            }

            fn init(_config: &PluginConfig) -> crate::Result<Arc<dyn crate::plugin::Plugin>> {
                Ok(Arc::new(MyMacroPlugin))
            }
        }

        // Initialize factories and verify discovery
        initialize_all_plugin_factories();
        let types = get_all_plugin_types();
        // Derived name from `MyMacroPlugin` -> `my_macro`
        assert!(
            types.iter().any(|t| t == "my_macro"),
            "Expected derived factory name 'my_macro' present"
        );
        assert!(
            get_plugin_factory("my_macro").is_some(),
            "Factory for 'my_macro' should be found"
        );

        // Ensure factory can create an instance
        let factory = get_plugin_factory("my_macro").unwrap();
        let plugin = factory
            .create(&PluginConfig::new("my_macro".to_string()))
            .unwrap();
        assert_eq!(plugin.name(), "my_macro_plugin");
    }

    // Test that exec plugins deriving #[derive(RegisterExecPlugin)] are discoverable
    #[test]
    fn test_derive_register_exec_plugin_discovery() {
        #[derive(Debug, RegisterExecPlugin)]
        struct MyExecPlugin;

        impl crate::plugin::traits::ExecPlugin for MyExecPlugin {
            fn quick_setup(
                _prefix: &str,
                _exec_str: &str,
            ) -> crate::Result<Arc<dyn crate::plugin::Plugin>> {
                Ok(Arc::new(MyExecPlugin))
            }
        }

        #[async_trait]
        impl crate::plugin::traits::Plugin for MyExecPlugin {
            async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
                Ok(())
            }

            fn name(&self) -> &str {
                "my_exec_plugin"
            }

            fn init(_config: &PluginConfig) -> crate::Result<Arc<dyn crate::plugin::Plugin>> {
                // Not used for exec quick_setup
                Err(crate::Error::Config("not supported".to_string()))
            }

            fn aliases() -> &'static [&'static str] {
                &[]
            }
        }

        initialize_all_exec_plugin_factories();
        let exec_types = get_all_exec_plugin_types();
        // Derived name: MyExecPlugin -> strip suffix -> MyExec -> my_exec
        assert!(
            exec_types.iter().any(|t| t == "my_exec"),
            "Expected exec factory name 'my_exec' present"
        );
        assert!(
            get_exec_plugin_factory("my_exec").is_some(),
            "Exec factory for 'my_exec' should be found"
        );

        let exec_factory = get_exec_plugin_factory("my_exec").unwrap();
        let plugin = exec_factory.create("my_exec", "arg").unwrap();
        assert_eq!(plugin.name(), "my_exec_plugin");
    }

    #[test]
    fn test_alias_matching_plugin_factory() {
        #[derive(Debug, RegisterPlugin)]
        struct MyAliasPlugin;

        #[async_trait]
        impl crate::plugin::traits::Plugin for MyAliasPlugin {
            async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
                Ok(())
            }

            fn name(&self) -> &str {
                "my_alias_plugin"
            }

            fn init(_config: &PluginConfig) -> crate::Result<Arc<dyn crate::plugin::Plugin>> {
                Ok(Arc::new(MyAliasPlugin))
            }

            fn aliases() -> &'static [&'static str] {
                &["alias_one", "other"]
            }
        }

        initialize_all_plugin_factories();
        // Derived canonical name
        assert!(get_plugin_factory("my_alias").is_some());
        // Aliases should resolve to the same factory
        assert!(get_plugin_factory("alias_one").is_some());
        assert!(get_plugin_factory("other").is_some());

        let f1 = get_plugin_factory("alias_one").unwrap();
        let inst = f1
            .create(&PluginConfig::new("alias_one".to_string()))
            .unwrap();
        assert_eq!(inst.name(), "my_alias_plugin");
    }

    #[test]
    fn test_alias_matching_exec_plugin_factory() {
        #[derive(Debug, RegisterExecPlugin)]
        struct MyAliasExecPlugin;

        impl crate::plugin::traits::ExecPlugin for MyAliasExecPlugin {
            fn quick_setup(
                _prefix: &str,
                _exec_str: &str,
            ) -> crate::Result<Arc<dyn crate::plugin::Plugin>> {
                Ok(Arc::new(MyAliasExecPlugin))
            }
        }

        #[async_trait]
        impl crate::plugin::traits::Plugin for MyAliasExecPlugin {
            async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
                Ok(())
            }
            fn name(&self) -> &str {
                "my_alias_exec_plugin"
            }
            fn init(_config: &PluginConfig) -> crate::Result<Arc<dyn crate::plugin::Plugin>> {
                Err(crate::Error::Config("not supported".to_string()))
            }
            fn aliases() -> &'static [&'static str] {
                &["alias_exec"]
            }
        }

        initialize_all_exec_plugin_factories();
        assert!(get_exec_plugin_factory("my_alias_exec").is_some());
        assert!(get_exec_plugin_factory("alias_exec").is_some());

        let f = get_exec_plugin_factory("alias_exec").unwrap();
        let p = f.create("alias_exec", "x").unwrap();
        assert_eq!(p.name(), "my_alias_exec_plugin");
    }
}
