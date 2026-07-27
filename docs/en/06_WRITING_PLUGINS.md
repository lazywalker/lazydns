# Writing Plugins (Developer)

This guide gives a minimal, practical example of implementing a plugin, wiring it into the builder system, and testing it.

## Minimal plugin example

Below is a small `echo` plugin that stores a metadata key on the request `Context`. It shows the essential pieces: struct, `Plugin` impl, `init`, and registration.

```rust
use std::sync::Arc;
use async_trait::async_trait;
use crate::plugin::{Context, Plugin};
use crate::config::PluginConfig;

#[derive(Debug, Clone)]
pub struct EchoPlugin {
	key: String,
	value: String,
}

impl EchoPlugin {
	pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
		Self { key: key.into(), value: value.into() }
	}
}

#[async_trait]
impl Plugin for EchoPlugin {
	fn name(&self) -> &str { "echo" }

	async fn execute(&self, ctx: &mut Context) -> crate::Result<()> {
		ctx.set_metadata(self.key.clone(), self.value.clone());
		Ok(())
	}

	fn init(config: &PluginConfig) -> crate::Result<Arc<dyn Plugin>> {
		let args = config.effective_args();
		let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("echo").to_string();
		let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("ok").to_string();
		Ok(Arc::new(EchoPlugin::new(key, value)))
	}
}

// Preferred: derive registration so the factory is registered automatically.
// The derive macro generates a factory wrapper, derives a canonical plugin name
// from the type (PascalCase -> snake_case, strip trailing "Plugin" suffix),
// and registers the factory via `linkme` so it is discoverable at runtime.
#[derive(RegisterPlugin)]
pub struct EchoPlugin;
```

## Exec-style quick setup

If your plugin should support compact `exec:` configuration, implement `ExecPlugin::quick_setup`:

```rust
use crate::plugin::ExecPlugin;
use std::sync::Arc;

#[derive(RegisterExecPlugin)]
pub struct EchoPlugin;

#[async_trait]
impl ExecPlugin for EchoPlugin {
	fn quick_setup(prefix: &str, exec_str: &str) -> crate::Result<Arc<dyn Plugin>> {
		if prefix != "echo" {
			return Err(crate::Error::Config("unsupported prefix".to_string()));
		}
		// exec_str could be "key=value" or any concise format you choose
		let parts: Vec<&str> = exec_str.splitn(2, '=').collect();
		let key = parts.get(0).cloned().unwrap_or("echo").trim().to_string();
		let value = parts.get(1).cloned().unwrap_or("ok").trim().to_string();
		Ok(Arc::new(EchoPlugin::new(key, value)))
	}
}
```

## Test template

Use the existing `Context` and `Message` helpers in tests. Prefer small, fast unit tests that exercise `execute` and `init`.

```rust
#[cfg(test)]
mod tests {
	use super::*;
	use crate::dns::Message;
	use crate::plugin::Context;

	#[tokio::test]
	async fn test_echo_execute_sets_metadata() {
		let plugin = EchoPlugin::new("my_key", "my_value");
		let mut req = Message::new();
		let mut ctx = Context::new(req);

		plugin.execute(&mut ctx).await.unwrap();

		let v = ctx.get_metadata::<String>("my_key").expect("metadata");
		assert_eq!(v, "my_value");
	}

	#[test]
	fn test_init_from_config() {
		use crate::config::PluginConfig;
		let mut cfg = PluginConfig::new("echo".to_string());
		cfg.args = serde_yaml::from_str(r#"{ key: test, value: hello }"#).unwrap();
		let plugin = EchoPlugin::init(&cfg).unwrap();
		assert_eq!(plugin.name(), "echo");
	}
}
```

## Plugin API
- `Plugin` trait
- `ExecPlugin`, `Matcher`, and matchers
- `PluginConfig` and builder pattern

## Builder & Factory
- Automatic registration via `#[derive(RegisterPlugin)]` (preferred)
- `PluginFactory` lifecycle

### Automatic registration via derive macros 🔧
The project now prefers using derive macros to register plugin factories automatically.

- Use `#[derive(RegisterPlugin)]` on your plugin type to generate and register a factory at compile time.
- The derived canonical plugin type name is produced by stripping a trailing `Plugin` suffix (if present) and converting PascalCase to snake_case, for example `MyCachePlugin` -> `my_cache`.
- The macro submits the factory into the `linkme` distributed slice; the runtime can then discover it with `crate::plugin::factory::get_plugin_factory("type_name")`.
- For exec-style plugins, use `#[derive(RegisterExecPlugin)]` which behaves the same but registers into the exec-factory slice.

Note: The old `crate::register_plugin_builder!(Type)` macro is still available for backward compatibility but is deprecated; prefer `#[derive(RegisterPlugin)]` for new code.

## Plugin Lifecycle
- `init`, `exec`, `shutdown`
- Thread-safety and concurrency patterns

### Shutdown and graceful cleanup

Plugins that spawn background tasks, hold file-watcher handles, or manage other resources should implement graceful shutdown to avoid leaks and to enable the application to stop cleanly in tests and in production.

1. Implement the `Shutdown` trait for cleanup logic:

```rust
use async_trait::async_trait;
use crate::plugin::traits::Shutdown;

#[async_trait]
impl Shutdown for MyPlugin {
	async fn shutdown(&self) -> crate::Result<()> {
		// stop background tasks, close watchers, flush data, etc.
		if let Some(h) = self.watcher.lock().take() {
			h.stop().await;
		}
		Ok(())
	}
}
```

2. Use the `#[derive(ShutdownPlugin)]` macro to auto-generate the `as_shutdown` bridge method:

```rust
use crate::plugin::{Plugin, traits::Shutdown};
use crate::ShutdownPlugin;

#[derive(ShutdownPlugin)]
struct MyPlugin {
	// fields
}

#[async_trait]
impl Plugin for MyPlugin {
	fn name(&self) -> &str { "my_plugin" }

	// other methods...
	// as_shutdown() is auto-generated by the macro, no need to implement it
}

#[async_trait]
impl Shutdown for MyPlugin {
	async fn shutdown(&self) -> crate::Result<()> {
		// cleanup code
		Ok(())
	}
}
```

Alternatively, if not using the derive macro, manually override `as_shutdown` in your `Plugin` impl:

```rust
impl crate::plugin::Plugin for MyPlugin {
	fn name(&self) -> &str { "my_plugin" }

	// other methods...

	fn as_shutdown(&self) -> Option<&dyn Shutdown> {
		Some(self)
	}
}
```

3. Notes and best practices
- Prefer using `#[derive(ShutdownPlugin)]` for automatic bridge method generation.
- The shutdown coordinator will automatically discover and call `Shutdown::shutdown()` via the `as_shutdown` bridge.
- Do not hold non-`Send` locks (such as `std::sync::MutexGuard`) across `.await`. Take/clone the handles out of the lock before awaiting their JoinHandles.
- Keep shutdown fast and idempotent; it may be called during tests or on repeated reloads.

Following this pattern makes plugins safe to use in the runtime and makes tests deterministic by allowing explicit cleanup of background work.

## Testing plugins
- Unit tests, doc-tests, integration tests
- Example test harness snippets

## Best practices and notes

- Thread-safety: plugins are typically stored behind `Arc` and may be accessed from multiple threads; keep interior mutability behind locks if needed.
- Errors: return meaningful `crate::Error` variants; sequence runners will propagate errors and usually stop processing.
- Logging: use `tracing` macros (`info!`, `debug!`, `warn!`) rather than `println!`.
- Tests: keep unit tests deterministic and avoid external network I/O. For integration tests that require network, mark them separately or use mocks.
- Quick-setup: provide both `init(config)` and `ExecPlugin::quick_setup` when convenient so users can configure plugins in either full or compact forms.
