//! Plugin system module
//!
//! This module provides the plugin system architecture for lazydns.
//! Plugins are the core building blocks that process DNS queries in a
//! flexible and composable way.
//!
//! # Architecture
//!
//! The plugin system consists of:
//! - **Plugin trait**: The core interface all plugins must implement
//! - **Context**: Execution context passed between plugins
//! - **Registry**: Manages plugin registration and lookup
//!
//! # Example
//!
//! ```rust
//! use lazydns::plugin::{Plugin, Context};
//! use lazydns::Result;
//! use async_trait::async_trait;
//!
//! #[derive(Debug)]
//! struct MyPlugin;
//!
//! #[async_trait]
//! impl Plugin for MyPlugin {
//!     async fn execute(&self, ctx: &mut Context) -> Result<()> {
//!         // Process the DNS query in context
//!         Ok(())
//!     }
//!
//!     fn name(&self) -> &str {
//!         "my_plugin"
//!     }
//! }
//! ```

pub mod builder;
pub mod condition;
pub mod context;
pub mod factory;
pub mod registry;
pub mod traits;

/// Metadata key used by control-flow plugins to stop execution.
pub const RETURN_FLAG: &str = "__return_flag";

// Re-export commonly used types
pub use builder::PluginBuilder;
pub use context::Context;
pub use registry::Registry;
pub use traits::{BackgroundTask, ExecPlugin, Plugin};

use crate::Result;
use crate::dns::Message;
use crate::server::{RequestContext, RequestHandler};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, warn};

/// Request handler that uses the plugin registry
pub struct PluginHandler {
    pub registry: Arc<Registry>,
    pub entry: String,
}

#[async_trait::async_trait]
impl RequestHandler for PluginHandler {
    async fn handle(&self, req_ctx: RequestContext) -> Result<Message> {
        use crate::plugin::Context;

        let mut ctx = Context::new(req_ctx.message.clone());

        // Record request start time for latency tracking
        ctx.set_metadata("request_start_time", Instant::now());

        // Set client IP metadata if available from request context
        if let Some(client_info) = req_ctx.client_info {
            ctx.set_metadata("client_ip", client_info.ip);
            ctx.set_metadata("client_addr", client_info.addr);
            ctx.set_metadata("client_port", client_info.port);
        }

        // Set protocol metadata
        ctx.set_metadata("protocol", req_ctx.protocol.to_string());

        // Check if this is a background lazy refresh (marked by special ID)
        if req_ctx.message.id() == 0xFFFF {
            ctx.set_metadata("background_lazy_refresh", true);
        }

        // Inject lazy refresh handler for plugins that need background processing
        // This allows plugins like CachePlugin to perform lazy refresh by creating
        // new PluginHandler instances for background queries
        ctx.set_metadata(
            "lazy_refresh_handler",
            Arc::new(PluginHandler {
                registry: Arc::clone(&self.registry),
                entry: self.entry.clone(),
            }),
        );
        ctx.set_metadata("lazy_refresh_entry", self.entry.clone());

        // Inject plugin registry into context for jump_target handling by SequencePlugin
        ctx.set_metadata("__plugin_registry", Arc::clone(&self.registry));

        if let Some(plugin) = self.registry.get(&self.entry) {
            plugin.execute(&mut ctx).await?;
        }

        // Handle goto labels set by plugins (goto replaces the current sequence)
        // Execute goto targets in a loop in case a goto target sets another goto.
        while ctx.has_metadata("goto_label") {
            if let Some(target) = ctx.get_metadata::<String>("goto_label").cloned() {
                // Remove goto label metadata and return flag before executing target
                ctx.remove_metadata("goto_label");
                ctx.remove_metadata(RETURN_FLAG);

                debug!(goto_target = %target, "Handling goto target: replacing current sequence");

                if let Some(plugin) = self.registry.get(&target) {
                    plugin.execute(&mut ctx).await?;
                } else {
                    warn!(goto_target = %target, "Goto target plugin not found");
                    break;
                }
            } else {
                break;
            }
        }

        // After executing sequence and jump targets, perform post-processing hooks:

        // 1. Store responses in cache if cache plugin is registered
        if ctx.has_response() {
            for name in self.registry.plugin_names() {
                if let Some(p) = self.registry.get(&name)
                    && p.name() == "cache"
                {
                    // Call execute again on cache plugin to trigger write phase
                    // (CachePlugin checks if response exists and stores it)
                    if let Err(e) = p.execute(&mut ctx).await {
                        warn!("Error during cache store post-processing: {}", e);
                    }
                }
            }
        }

        // 2. Allow reverse-lookup plugins to observe the populated response
        // and save IP->name mappings.
        if ctx.has_response()
            && let Some(resp) = ctx.response()
        {
            for name in self.registry.plugin_names() {
                if let Some(p) = self.registry.get(&name)
                    && p.name() == "reverse_lookup"
                    && let Some(rl) =
                        p.as_ref()
                            .as_any()
                            .downcast_ref::<crate::plugins::executable::ReverseLookupPlugin>()
                {
                    rl.save_ips_after(ctx.request(), resp);
                }
            }
        }

        // 3. Automatically publish query log to the event bus (WebUI / alert
        // engine consume from there). Compiled with the `web` feature; no user
        // configuration required. Runs after the sequence so all response
        // details are available.
        #[cfg(feature = "web")]
        crate::plugins::audit::log_query_for_context(&ctx);

        // Ensure any response uses the original request ID
        let req_id = ctx.request().id();
        let mut response = ctx.take_response().unwrap_or_else(|| {
            let mut r = Message::new();
            r.set_response_code(crate::dns::ResponseCode::ServFail);
            r
        });
        response.set_id(req_id);

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;
    use async_trait::async_trait;

    #[derive(Debug)]
    struct TestPlugin;

    #[async_trait]
    impl Plugin for TestPlugin {
        async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
            Ok(())
        }

        fn name(&self) -> &str {
            "test"
        }
    }

    #[test]
    fn test_plugin_system_reexports() {
        // Verify all re-exported types are accessible
        let request = Message::new();
        let _ctx = Context::new(request);
        let _registry = Registry::new();
    }

    #[test]
    fn test_registry_plugin_registration() {
        use std::sync::Arc;

        let mut registry = Registry::new();
        let plugin: Arc<dyn Plugin> = Arc::new(TestPlugin);

        registry.register(plugin.clone()).ok();
        let retrieved = registry.get("test");
        assert!(retrieved.is_some());
    }

    #[test]
    fn test_return_flag_constant() {
        // Verify the RETURN_FLAG constant is accessible
        assert_eq!(RETURN_FLAG, "__return_flag");
    }

    #[tokio::test]
    async fn test_context_metadata() {
        let request = Message::new();
        let mut ctx = Context::new(request);

        // Test metadata operations using set_metadata and get_metadata
        ctx.set_metadata("key", "value".to_string());
        let value = ctx.get_metadata::<String>("key");
        assert_eq!(value, Some(&"value".to_string()));
    }

    #[tokio::test]
    async fn test_goto_replaces_sequence_and_executes_target() {
        use crate::server::Protocol;
        use std::sync::Arc;

        // Build a registry with an entry that sets goto_label and a target that sets a response
        let mut registry = Registry::new();

        #[derive(Debug)]
        struct Entry;
        #[async_trait::async_trait]
        impl crate::plugin::Plugin for Entry {
            async fn execute(&self, ctx: &mut Context) -> crate::Result<()> {
                ctx.set_metadata("goto_label", "target".to_string());
                ctx.set_metadata(RETURN_FLAG, true);
                Ok(())
            }

            fn name(&self) -> &str {
                "entry"
            }
        }

        #[derive(Debug)]
        struct Target;
        #[async_trait::async_trait]
        impl crate::plugin::Plugin for Target {
            async fn execute(&self, ctx: &mut Context) -> crate::Result<()> {
                let mut r = Message::new();
                r.set_response(true);
                ctx.set_response(Some(r));
                Ok(())
            }

            fn name(&self) -> &str {
                "target"
            }
        }

        registry.register(Arc::new(Entry)).unwrap();
        registry.register(Arc::new(Target)).unwrap();

        let handler = PluginHandler {
            registry: Arc::new(registry),
            entry: "entry".to_string(),
        };

        let req = RequestContext::new(Message::new(), Protocol::Udp);
        let resp = handler.handle(req).await.unwrap();

        // The response should have been set by the target plugin executed via goto
        assert!(resp.is_response());
    }

    #[tokio::test]
    async fn test_jump_continues_after_target_execution() {
        use crate::plugins::SequencePlugin;

        // This test verifies jump's push/return semantics via SequencePlugin
        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));

        #[derive(Debug, Clone)]
        struct Recorder {
            label: String,
            order: Arc<std::sync::Mutex<Vec<String>>>,
        }

        #[async_trait::async_trait]
        impl crate::plugin::Plugin for Recorder {
            async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
                self.order.lock().unwrap().push(self.label.clone());
                Ok(())
            }

            fn name(&self) -> &str {
                &self.label
            }
        }

        // Build a registry with target plugin
        let mut registry = Registry::new();
        registry
            .register(Arc::new(Recorder {
                label: "target".to_string(),
                order: execution_order.clone(),
            }))
            .unwrap();

        // Create sequence: plugin_a -> jump plugin -> plugin_b
        // Expected: a -> (jump) -> target -> b (push/return semantics)
        let seq = SequencePlugin::new(vec![
            Arc::new(Recorder {
                label: "a".to_string(),
                order: execution_order.clone(),
            }),
            Arc::new({
                #[derive(Debug)]
                struct JumpPlugin {
                    order: Arc<std::sync::Mutex<Vec<String>>>,
                }

                #[async_trait::async_trait]
                impl crate::plugin::Plugin for JumpPlugin {
                    async fn execute(&self, ctx: &mut Context) -> crate::Result<()> {
                        self.order.lock().unwrap().push("jump".to_string());
                        ctx.set_metadata("jump_target", "target".to_string());
                        ctx.set_metadata(RETURN_FLAG, true);
                        Ok(())
                    }

                    fn name(&self) -> &str {
                        "jump_plugin"
                    }
                }

                JumpPlugin {
                    order: execution_order.clone(),
                }
            }),
            Arc::new(Recorder {
                label: "b".to_string(),
                order: execution_order.clone(),
            }),
        ]);

        let mut ctx = Context::new(Message::new());
        ctx.set_metadata("__plugin_registry", Arc::new(registry));
        seq.execute(&mut ctx).await.unwrap();

        let logged = execution_order.lock().unwrap().clone();
        // Verify that jump executed target but continued to b (push/return semantics)
        assert_eq!(logged, vec!["a", "jump", "target", "b"]);
    }

    #[tokio::test]
    async fn test_goto_stops_sequence_execution() {
        use crate::plugins::SequencePlugin;

        // Test that goto stops sequence execution without continuing to next step
        let execution_order = Arc::new(std::sync::Mutex::new(Vec::new()));

        #[derive(Debug, Clone)]
        struct Recorder {
            label: String,
            order: Arc<std::sync::Mutex<Vec<String>>>,
        }

        #[async_trait::async_trait]
        impl crate::plugin::Plugin for Recorder {
            async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
                self.order.lock().unwrap().push(self.label.clone());
                Ok(())
            }

            fn name(&self) -> &str {
                &self.label
            }
        }

        // Create sequence: plugin_a -> goto plugin -> plugin_b (should NOT execute)
        let seq = SequencePlugin::new(vec![
            Arc::new(Recorder {
                label: "a".to_string(),
                order: execution_order.clone(),
            }),
            Arc::new({
                #[derive(Debug)]
                struct GotoPlugin {
                    order: Arc<std::sync::Mutex<Vec<String>>>,
                }

                #[async_trait::async_trait]
                impl crate::plugin::Plugin for GotoPlugin {
                    async fn execute(&self, ctx: &mut Context) -> crate::Result<()> {
                        self.order.lock().unwrap().push("goto".to_string());
                        ctx.set_metadata("goto_label", "alternative".to_string());
                        ctx.set_metadata(RETURN_FLAG, true);
                        Ok(())
                    }

                    fn name(&self) -> &str {
                        "goto_plugin"
                    }
                }

                GotoPlugin {
                    order: execution_order.clone(),
                }
            }),
            Arc::new(Recorder {
                label: "b".to_string(),
                order: execution_order.clone(),
            }),
        ]);

        let mut ctx = Context::new(Message::new());
        seq.execute(&mut ctx).await.unwrap();

        let logged = execution_order.lock().unwrap().clone();
        // Verify that goto stopped execution (b should NOT be in the list)
        assert_eq!(logged, vec!["a", "goto"]);
        // Verify that goto_label is still in context for PluginHandler to process
        assert_eq!(
            ctx.get_metadata::<String>("goto_label"),
            Some(&"alternative".to_string())
        );
    }
}
