//! Sequence executor plugin (executable wrapper)
//!
//! This module provides the `SequencePlugin` used by the runtime to execute
//! a sequential list of plugins. It is extracted here from the larger
//! `plugins::advanced` module so it can be reused by executable/plugin
//! composition code and documented independently.

use crate::Result;
use crate::plugin::{Context, Plugin, RETURN_FLAG};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::trace;

/// A sequential execution step for `SequencePlugin`.
pub enum SequenceStep {
    /// Execute a plugin unconditionally
    Exec(Arc<dyn Plugin>),
    /// Execute a plugin conditionally
    If {
        /// Condition invoked with current `Context`
        condition: Arc<dyn Fn(&Context) -> bool + Send + Sync>,
        /// Plugin to execute when condition is true
        action: Arc<dyn Plugin>,
        /// Human-readable condition description (for tracing)
        desc: String,
    },
}

impl std::fmt::Debug for SequenceStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SequenceStep::Exec(plugin) => f.debug_tuple("Exec").field(plugin).finish(),
            SequenceStep::If { action, desc, .. } => f
                .debug_struct("If")
                .field("action", action)
                .field("cond", desc)
                .finish(),
        }
    }
}

/// Executes a sequence of plugins in order.
#[derive(Debug)]
pub struct SequencePlugin {
    steps: Vec<SequenceStep>,
    tag: Option<String>,
}

impl SequencePlugin {
    /// Create a new sequence plugin from a simple list of plugins.
    pub fn new(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        let steps = plugins.into_iter().map(SequenceStep::Exec).collect();
        Self { steps, tag: None }
    }

    /// Create a sequence plugin with explicit steps (including conditional steps).
    pub fn with_steps(steps: Vec<SequenceStep>) -> Self {
        Self { steps, tag: None }
    }

    /// Create a sequence plugin with explicit steps and an optional tag.
    /// This preserves the configured tag so `display_name()` can include it.
    pub fn with_steps_and_tag(steps: Vec<SequenceStep>, tag: Option<String>) -> Self {
        Self { steps, tag }
    }

    pub fn steps(&self) -> &[SequenceStep] {
        &self.steps
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Resolve pending `fallback` child references inside this sequence's
    /// steps. A `- exec: "fallback a,b"` step creates the fallback by name,
    /// which can only be wired up once the whole registry exists.
    pub fn resolve_fallback_children(
        &self,
        registry: &std::collections::HashMap<String, Arc<dyn Plugin>>,
    ) {
        use super::FallbackPlugin;

        for step in &self.steps {
            let plugin = match step {
                SequenceStep::Exec(plugin) => plugin,
                SequenceStep::If { action, .. } => action,
            };
            if let Some(fp) = plugin.as_ref().as_any().downcast_ref::<FallbackPlugin>() {
                fp.resolve_children(registry);
            }
        }
    }
}

#[async_trait]
impl Plugin for SequencePlugin {
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        for step in &self.steps {
            match step {
                SequenceStep::Exec(plugin) => {
                    trace!(
                        plugin = plugin.display_name(),
                        "Sequence: executing plugin (exec)"
                    );
                    match plugin.execute(ctx).await {
                        Ok(_) => trace!(plugin = plugin.display_name(), "Sequence: exec succeeded"),
                        Err(e) => {
                            trace!(plugin = plugin.display_name(), error = %e, "Sequence: exec failed");
                            return Err(e);
                        }
                    }
                }
                SequenceStep::If {
                    condition,
                    action,
                    desc,
                } => {
                    let cond = condition(ctx);
                    trace!(condition = %desc, result = cond, plugin = action.display_name(), "Sequence: conditional step evaluated");
                    if cond {
                        trace!(plugin = action.display_name(), condition = %desc, "Sequence: executing conditional action");
                        match action.execute(ctx).await {
                            Ok(_) => {
                                trace!(plugin = action.display_name(), condition = %desc, "Sequence: conditional action succeeded")
                            }
                            Err(e) => {
                                trace!(plugin = action.display_name(), condition = %desc, error = %e, "Sequence: conditional action failed");
                                return Err(e);
                            }
                        }
                    }
                }
            }

            // Handle jump_target (push/return semantics): execute target and continue with next step
            while ctx.has_metadata("jump_target") {
                if let Some(target) = ctx.get_metadata::<String>("jump_target").cloned() {
                    // Remove jump target and return flag before executing target
                    ctx.remove_metadata("jump_target");
                    ctx.remove_metadata(RETURN_FLAG);

                    trace!(jump_target = %target, "Sequence: handling jump target (push/return)");

                    // Get registry from context metadata
                    if let Some(registry) = ctx
                        .get_metadata::<std::sync::Arc<crate::plugin::Registry>>(
                            "__plugin_registry",
                        )
                    {
                        if let Some(target_plugin) = registry.get(&target) {
                            // Save the current RETURN_FLAG state before executing jump target
                            // This prevents jump targets from stopping the calling sequence
                            let saved_return_flag = ctx.get_metadata::<bool>(RETURN_FLAG).copied();

                            match target_plugin.execute(ctx).await {
                                Ok(_) => {
                                    trace!(jump_target = %target, "Sequence: jump target succeeded")
                                }
                                Err(e) => {
                                    trace!(jump_target = %target, error = %e, "Sequence: jump target failed");
                                    return Err(e);
                                }
                            }

                            // Restore the RETURN_FLAG state after jump target execution
                            // This ensures jump targets don't affect the calling sequence's flow
                            if let Some(flag) = saved_return_flag {
                                ctx.set_metadata(RETURN_FLAG, flag);
                            } else {
                                ctx.remove_metadata(RETURN_FLAG);
                            }
                        } else {
                            trace!(jump_target = %target, "Sequence: jump target plugin not found");
                        }
                    }
                } else {
                    break;
                }
            }

            // Handle goto_label (replace sequence semantics): stop and return to PluginHandler
            if ctx.has_metadata("goto_label") {
                // Set RETURN_FLAG to signal PluginHandler to handle the goto
                ctx.set_metadata(RETURN_FLAG, true);
                trace!("Sequence: goto_label detected, stopping sequence execution");
                break;
            }

            // If a plugin set the return flag (and it's not a goto), stop executing further steps.
            if matches!(ctx.get_metadata::<bool>(RETURN_FLAG), Some(true)) {
                break;
            }
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "sequence"
    }

    // without this the as_any default returns &() and every downcast fails
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    fn init(config: &crate::config::PluginConfig) -> Result<std::sync::Arc<dyn Plugin>> {
        // For now, implement a simple sequence that expects a "plugins" array
        // with plugin names. Full sequence parsing with conditions is complex
        // and should be handled by the builder system.
        let args = config.effective_args();

        if let Some(serde_yaml::Value::Sequence(_plugin_names)) = args.get("plugins") {
            // This is a simplified implementation - in practice, sequences with
            // plugin references need to be resolved later in the build process
            // For now, return an empty sequence that will be resolved later
            Ok(std::sync::Arc::new(Self {
                steps: vec![],
                tag: config.tag.clone(),
            }))
        } else {
            // Default to empty sequence
            Ok(std::sync::Arc::new(Self {
                steps: vec![],
                tag: config.tag.clone(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;
    use crate::plugin::Context;
    use std::sync::Arc;

    #[derive(Debug)]
    struct Recorder {
        order: Arc<std::sync::Mutex<Vec<&'static str>>>,
        label: &'static str,
    }

    #[async_trait]
    impl Plugin for Recorder {
        async fn execute(&self, ctx: &mut Context) -> Result<()> {
            ctx.set_metadata("seen", true);
            let mut order = self.order.lock().unwrap_or_else(|poisoned| {
                // In tests, if mutex is poisoned, still allow access
                poisoned.into_inner()
            });
            order.push(self.label);
            Ok(())
        }

        fn name(&self) -> &str {
            self.label
        }
    }

    #[test]
    fn test_sequence_new() {
        let seq = SequencePlugin::new(vec![]);
        assert_eq!(seq.name(), "sequence");
    }

    #[test]
    fn test_sequence_with_steps() {
        let seq = SequencePlugin::with_steps(vec![]);
        assert!(seq.tag.is_none());
    }

    #[test]
    fn test_sequence_with_steps_and_tag() {
        let seq = SequencePlugin::with_steps_and_tag(vec![], Some("my_tag".to_string()));
        assert_eq!(seq.tag, Some("my_tag".to_string()));
    }

    #[test]
    fn test_sequence_debug() {
        let seq = SequencePlugin::new(vec![]);
        let debug_str = format!("{:?}", seq);
        assert!(debug_str.contains("SequencePlugin"));
    }

    #[test]
    fn test_sequence_step_exec_debug() {
        #[derive(Debug)]
        struct DummyPlugin;

        #[async_trait]
        impl Plugin for DummyPlugin {
            async fn execute(&self, _ctx: &mut Context) -> Result<()> {
                Ok(())
            }
            fn name(&self) -> &str {
                "dummy"
            }
        }

        let step = SequenceStep::Exec(Arc::new(DummyPlugin));
        let debug_str = format!("{:?}", step);
        assert!(debug_str.contains("Exec"));
    }

    #[test]
    fn test_sequence_step_if_debug() {
        #[derive(Debug)]
        struct DummyPlugin;

        #[async_trait]
        impl Plugin for DummyPlugin {
            async fn execute(&self, _ctx: &mut Context) -> Result<()> {
                Ok(())
            }
            fn name(&self) -> &str {
                "dummy"
            }
        }

        let step = SequenceStep::If {
            condition: Arc::new(|_| true),
            action: Arc::new(DummyPlugin),
            desc: "always true".to_string(),
        };
        let debug_str = format!("{:?}", step);
        assert!(debug_str.contains("If"));
        assert!(debug_str.contains("always true"));
    }

    #[tokio::test]
    async fn sequence_executes_in_order() {
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seq = SequencePlugin::new(vec![
            Arc::new(Recorder {
                order: order.clone(),
                label: "one",
            }),
            Arc::new(Recorder {
                order: order.clone(),
                label: "two",
            }),
        ]);

        let mut ctx = Context::new(Message::new());
        seq.execute(&mut ctx).await.unwrap();

        let logged = order.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert_eq!(logged, vec!["one", "two"]);
    }

    #[tokio::test]
    async fn test_sequence_conditional_step_true() {
        #[derive(Debug)]
        struct SetFlag;

        #[async_trait]
        impl Plugin for SetFlag {
            async fn execute(&self, ctx: &mut Context) -> Result<()> {
                ctx.set_metadata("flag_set", true);
                Ok(())
            }
            fn name(&self) -> &str {
                "set_flag"
            }
        }

        let seq = SequencePlugin::with_steps(vec![SequenceStep::If {
            condition: Arc::new(|_| true),
            action: Arc::new(SetFlag),
            desc: "always true".to_string(),
        }]);

        let mut ctx = Context::new(Message::new());
        seq.execute(&mut ctx).await.unwrap();

        assert_eq!(ctx.get_metadata::<bool>("flag_set"), Some(&true));
    }

    #[tokio::test]
    async fn test_sequence_conditional_step_false() {
        #[derive(Debug)]
        struct SetFlag;

        #[async_trait]
        impl Plugin for SetFlag {
            async fn execute(&self, ctx: &mut Context) -> Result<()> {
                ctx.set_metadata("flag_set", true);
                Ok(())
            }
            fn name(&self) -> &str {
                "set_flag"
            }
        }

        let seq = SequencePlugin::with_steps(vec![SequenceStep::If {
            condition: Arc::new(|_| false),
            action: Arc::new(SetFlag),
            desc: "always false".to_string(),
        }]);

        let mut ctx = Context::new(Message::new());
        seq.execute(&mut ctx).await.unwrap();

        assert!(ctx.get_metadata::<bool>("flag_set").is_none());
    }

    #[tokio::test]
    async fn test_sequence_stops_on_return_flag() {
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        #[derive(Debug)]
        struct SetReturnFlag {
            order: Arc<std::sync::Mutex<Vec<&'static str>>>,
        }

        #[async_trait]
        impl Plugin for SetReturnFlag {
            async fn execute(&self, ctx: &mut Context) -> Result<()> {
                let mut order = self.order.lock().unwrap_or_else(|p| p.into_inner());
                order.push("first");
                ctx.set_metadata(RETURN_FLAG, true);
                Ok(())
            }
            fn name(&self) -> &str {
                "set_return"
            }
        }

        let seq = SequencePlugin::new(vec![
            Arc::new(SetReturnFlag {
                order: order.clone(),
            }),
            Arc::new(Recorder {
                order: order.clone(),
                label: "should_not_run",
            }),
        ]);

        let mut ctx = Context::new(Message::new());
        seq.execute(&mut ctx).await.unwrap();

        let logged = order.lock().unwrap_or_else(|p| p.into_inner()).clone();
        // Only "first" should be logged, "should_not_run" should be skipped
        assert_eq!(logged, vec!["first"]);
    }
}
