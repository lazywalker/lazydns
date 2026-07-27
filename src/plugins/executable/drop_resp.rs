use std::sync::Arc;

use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, Result};
use async_trait::async_trait;

/// Plugin that clears any existing response from the execution `Context`.
///
/// This is a lightweight control-flow helper used in sequences or other
/// composite plugins when you want to discard a previously-set response
/// and continue processing with subsequent plugins.
///
/// Example:
///
/// ```rust
/// use lazydns::plugins::executable::drop_resp::DropRespPlugin;
/// use lazydns::plugin::Context;
///
/// let plugin = DropRespPlugin::new();
/// // in executor: plugin.execute(&mut ctx).await? will clear ctx.response()
/// ```
#[derive(Debug, Default, RegisterExecPlugin)]
pub struct DropRespPlugin;

impl DropRespPlugin {
    /// Create a new `DropRespPlugin` instance.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for DropRespPlugin {
    fn name(&self) -> &str {
        "drop_resp"
    }

    /// Execute the plugin: clear any response currently stored in the
    /// `Context` so subsequent plugins see an empty response state.
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        ctx.set_response(None);
        Ok(())
    }

    /// Initialize plugin from configuration.
    ///
    /// This plugin takes no configuration parameters.
    fn init(_config: &crate::config::types::PluginConfig) -> Result<Arc<dyn Plugin>> {
        Ok(Arc::new(DropRespPlugin::new()))
    }
}

impl ExecPlugin for DropRespPlugin {
    /// Parse a quick configuration string for drop_resp plugin.
    ///
    /// This plugin takes no parameters, so the exec_str is ignored.
    /// Examples: "drop_resp", ""
    fn quick_setup(prefix: &str, _exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != "drop_resp" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'drop_resp'",
                prefix
            )));
        }

        Ok(Arc::new(DropRespPlugin::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;

    #[tokio::test]
    async fn test_drop_resp() {
        let plugin = DropRespPlugin;
        let req = Message::new();
        let mut ctx = Context::new(req);
        ctx.set_response(Some(Message::new()));
        plugin.execute(&mut ctx).await.unwrap();
        assert!(!ctx.has_response());
    }

    #[test]
    fn test_exec_plugin_quick_setup() {
        // Test that ExecPlugin::quick_setup works correctly
        let plugin = <DropRespPlugin as ExecPlugin>::quick_setup("drop_resp", "").unwrap();
        assert_eq!(plugin.name(), "drop_resp");

        // Test with some exec_str (should be ignored)
        let plugin = <DropRespPlugin as ExecPlugin>::quick_setup("drop_resp", "ignored").unwrap();
        assert_eq!(plugin.name(), "drop_resp");

        // Test invalid prefix
        let result = <DropRespPlugin as ExecPlugin>::quick_setup("invalid", "");
        assert!(result.is_err());
    }
}
