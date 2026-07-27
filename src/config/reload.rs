//! Configuration hot reload support
//!
//! Provides the ability to reload configuration without restarting the server.

use crate::Result;
use crate::config::Config;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

/// Configuration reload handler
///
/// Monitors configuration file for changes and triggers reload.
///
/// # Example
///
/// ```no_run
/// use lazydns::config::reload::ConfigReloader;
/// use std::sync::Arc;
/// use tokio::sync::RwLock;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Arc::new(RwLock::new(lazydns::config::Config::new()));
/// let reloader = ConfigReloader::new("config.yaml", Arc::clone(&config));
/// reloader.start_watching().await?;
/// # Ok(())
/// # }
/// ```
pub struct ConfigReloader {
    config_path: PathBuf,
    config: Arc<RwLock<Config>>,
}

impl ConfigReloader {
    /// Create a new configuration reloader
    ///
    /// # Arguments
    ///
    /// * `config_path` - Path to the configuration file
    /// * `config` - Shared configuration reference
    pub fn new(config_path: impl AsRef<Path>, config: Arc<RwLock<Config>>) -> Self {
        Self {
            config_path: config_path.as_ref().to_path_buf(),
            config,
        }
    }

    /// Start watching for configuration changes
    ///
    /// This function starts a background task that monitors the configuration
    /// file for changes and automatically reloads it.
    pub async fn start_watching(self) -> Result<()> {
        // Use the shared file watcher utility to avoid duplicating logic (debounce, re-watch, and others)
        let path = self.config_path.clone();
        let config = Arc::clone(&self.config);

        crate::utils::spawn_file_watcher(
            format!("config-reloader:{}", path.display()),
            vec![path.clone()],
            500, // debounce in ms
            move |p, _files| {
                let p = p.to_path_buf();
                let config = Arc::clone(&config);
                // Spawn an async task to perform reload
                tokio::spawn(async move {
                    info!("Config file changed ({:?}), reloading...", p);
                    match ConfigReloader::reload_config(&p, &config).await {
                        Ok(()) => info!("Configuration reloaded successfully"),
                        Err(e) => error!("Failed to reload configuration: {}", e),
                    }
                });
            },
        );

        Ok(())
    }

    /// Reload configuration from file
    async fn reload_config(path: &Path, config: &Arc<RwLock<Config>>) -> Result<()> {
        // Load new configuration
        let new_config = crate::config::loader::load_from_file(path)?;

        // Validate the new configuration
        new_config.validate()?;

        // Acquire write lock and update
        let mut config_guard = config.write().await;
        *config_guard = new_config;

        Ok(())
    }

    /// Manually trigger a configuration reload
    ///
    /// This can be called to reload configuration without waiting for file changes.
    pub async fn reload(&self) -> Result<()> {
        info!("Manual configuration reload triggered");
        Self::reload_config(&self.config_path, &self.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_config_reloader_creation() {
        let config = Arc::new(RwLock::new(Config::new()));
        let reloader = ConfigReloader::new("test.yaml", config);
        assert_eq!(reloader.config_path, PathBuf::from("test.yaml"));
    }

    #[tokio::test]
    async fn test_manual_reload() {
        let mut temp_file = NamedTempFile::new().unwrap();
        let config_content = r#"
log:
  level: info
server:
  timeout_secs: 5
"#;
        write!(temp_file, "{}", config_content).unwrap();
        temp_file.flush().unwrap();

        let config = Arc::new(RwLock::new(Config::new()));
        let reloader = ConfigReloader::new(temp_file.path(), Arc::clone(&config));

        // Manual reload should succeed
        let result = reloader.reload().await;
        assert!(result.is_ok());

        // Check config was updated
        let config_guard = config.read().await;
        assert_eq!(config_guard.log.level, "info");
    }

    #[tokio::test]
    async fn test_reload_invalid_config() {
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "invalid: yaml: {{").unwrap();
        temp_file.flush().unwrap();

        let config = Arc::new(RwLock::new(Config::new()));
        let reloader = ConfigReloader::new(temp_file.path(), config);

        // Reload should fail with invalid YAML
        let result = reloader.reload().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_start_watching_detects_change() {
        use tokio::time::Duration;

        // Create a temp file with an initial valid config
        let mut temp_file = NamedTempFile::new().unwrap();
        let initial = r#"
log:
  level: info
server:
  timeout_secs: 5
"#;
        write!(temp_file, "{}", initial).unwrap();
        temp_file.flush().unwrap();

        let config = Arc::new(RwLock::new(Config::new()));

        // Start the reloader which will spawn the watcher task
        let reloader = ConfigReloader::new(temp_file.path(), Arc::clone(&config));
        reloader.start_watching().await.unwrap();

        // Allow watcher task to start
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Prepare new content
        let new_content = r#"
log:
  level: debug
server:
  timeout_secs: 5
"#;

        // Try several different writes (modify + atomic replace) to handle platform/editor differences
        let dir = temp_file.path().parent().unwrap();
        let mut success = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            // 1) In-place write + sync
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(temp_file.path())
            {
                use std::io::Write as _;
                let _ = f.write_all(new_content.as_bytes());
                let _ = f.sync_all();
            }

            // 2) Atomic replace
            if let Ok(mut replace) = NamedTempFile::new_in(dir) {
                write!(replace, "{}", new_content).unwrap();
                replace.flush().unwrap();
                let _ = std::fs::rename(replace.path(), temp_file.path());
            }

            // Wait and check for reload for short windows
            for _ in 0..40 {
                {
                    let guard = config.read().await;
                    if guard.log.level == "debug" {
                        success = true;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            if success {
                break;
            }
        }

        assert!(success, "timeout waiting for config reload");

        // Verify the in-memory config was updated
        let guard = config.read().await;
        assert_eq!(guard.log.level, "debug");
    }
}
