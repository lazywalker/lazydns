use crate::{
    RegisterExecPlugin, Result,
    plugin::{Context, ExecPlugin, Plugin, RETURN_FLAG},
};
use async_trait::async_trait;

#[derive(Debug, Default, Clone, Copy, RegisterExecPlugin)]
pub struct ReturnPlugin;

impl ReturnPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for ReturnPlugin {
    fn name(&self) -> &str {
        "return"
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        ctx.set_metadata(RETURN_FLAG, true);
        Ok(())
    }
}

#[async_trait]
impl ExecPlugin for ReturnPlugin {
    /// Parse exec string for return plugin: "return"
    ///
    /// Examples:
    /// - "return" - stops execution of the current sequence
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<std::sync::Arc<dyn Plugin>> {
        if prefix != "return" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'return'",
                prefix
            )));
        }

        // Return plugin doesn't take any arguments, just "return"
        if !exec_str.trim().is_empty() {
            return Err(crate::Error::Config(
                "return exec does not take any arguments".to_string(),
            ));
        }

        Ok(std::sync::Arc::new(ReturnPlugin::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_return_plugin_stops_execution() {
        #[derive(Debug)]
        struct Counter {
            counter: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl Plugin for Counter {
            async fn execute(&self, _ctx: &mut Context) -> crate::Result<()> {
                self.counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            fn name(&self) -> &str {
                "counter"
            }
        }

        // Mimic the sequence executor: run plugins in order, stop early when
        // RETURN_FLAG is set (what SequencePlugin does in production).
        let return_plugin = ReturnPlugin::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_plugin = Counter {
            counter: counter.clone(),
        };
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(return_plugin), Arc::new(counter_plugin)];

        let mut ctx = Context::new(Message::new());
        for plugin in &plugins {
            plugin.execute(&mut ctx).await.unwrap();
            if ctx.get_metadata::<bool>(RETURN_FLAG) == Some(&true) {
                break;
            }
        }

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert_eq!(ctx.get_metadata::<bool>(RETURN_FLAG), Some(&true));
    }

    #[tokio::test]
    async fn test_exec_plugin_return() {
        let plugin = ReturnPlugin::quick_setup("return", "").unwrap();
        let mut ctx = Context::new(Message::new());

        plugin.execute(&mut ctx).await.unwrap();

        let return_flag = ctx.get_metadata::<bool>(RETURN_FLAG).unwrap();
        assert!(*return_flag);
    }

    #[tokio::test]
    async fn test_exec_plugin_invalid_prefix() {
        let result = ReturnPlugin::quick_setup("invalid", "");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_exec_plugin_with_args() {
        let result = ReturnPlugin::quick_setup("return", "some_arg");
        assert!(result.is_err());
    }
}
