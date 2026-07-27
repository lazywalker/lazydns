//! lazydns - A DNS server implementation in Rust
//!
//! This is a Rust implementation of mosdns, aiming for 100% feature parity or better.
//!
//! # Features
//! - Full DNS protocol support (UDP, TCP, DoH, DoT, DoQ)
//! - Flexible plugin architecture
//! - High performance with Rust's zero-cost abstractions
//! - Comprehensive caching
//! - GeoIP/GeoSite support
//! - Full test and documentation coverage

mod cli;
use crate::cli::parse_args;
use lazydns::config::Config;
#[cfg(feature = "log")]
use lazydns::config::LogConfig;
#[cfg(feature = "log")]
use lazydns::init_logging;
use lazydns::plugin::PluginBuilder;
use lazydns::server::ServerLauncher;
use std::sync::Arc;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::RwLock;
use tokio::time::{Duration, timeout};
use tracing::{debug, error, info};

// Command-line arguments are parsed in `src/cli.rs` using `pico-args`.

async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sighup = signal(SignalKind::hangup())?;

        tokio::select! {
            res = tokio::signal::ctrl_c() => {
                info!("Received Ctrl-C signal");
                res?;
            },
            _ = sigterm.recv() => {
                info!("Received SIGTERM signal");
            },
            _ = sighup.recv() => {
                info!("Received SIGHUP signal");
            },
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse command line arguments (prints help and returns early if requested)
    let args = match parse_args() {
        Some(a) => a,
        None => return Ok(()),
    };

    // Change working directory if specified (do this before loading config so relative
    // paths inside the config file behave as expected)
    if let Some(dir) = &args.dir {
        if let Err(e) = std::env::set_current_dir(dir) {
            error!("Failed to change working directory to {}: {}", dir, e);
            return Err(anyhow::anyhow!("Failed to change working directory"));
        }
        info!("Working directory changed to: {}", dir);
    }

    // Load configuration (before initializing logging so config can control logs)
    let config = match Config::from_file(&args.config) {
        Ok(config) => {
            println!("Configuration loaded successfully");
            config
        }
        Err(e) => {
            return Err(e.into());
        }
    };

    // Validate configuration
    if let Err(e) = config.validate() {
        eprintln!("Configuration validation warning: {}", e);
    }

    // Initialize logging: precedence handled in effective_log_spec
    #[cfg(feature = "log")]
    {
        // Apply CLI verbosity override to config level
        let log_spec = effective_log_spec(&config.log, args.verbose);

        // Use the new convenience method for clearer code
        let lazy_config = config.log.to_lazylog(log_spec);
        if let Err(e) = init_logging(&lazy_config) {
            eprintln!("Failed to initialize logging: {}", e);
        }
    }

    info!("lazydns starting...");
    info!("Version: {}", env!("CARGO_PKG_VERSION"));
    info!("Configuration file: {}", args.config);
    #[cfg(feature = "log")]
    debug!("Logging configuration: {}", config.log.summary());

    // Ensure rustls has a process-level CryptoProvider installed (ring)
    #[cfg(feature = "rustls")]
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Initialize event bus BEFORE plugins load (audit plugin needs it)
    #[cfg(feature = "web")]
    if config.web.enabled {
        lazydns::plugins::audit::init_event_bus(config.web.event_bus_capacity);
        info!("Event bus initialized for WebUI");
    }

    // Build plugins from configuration
    let mut builder = PluginBuilder::new();
    let mut plugin_count = 0;

    for plugin_config in &config.plugins {
        match builder.build(plugin_config) {
            Ok(_plugin) => {
                info!(
                    "Loaded plugin: {} (type: {})",
                    plugin_config.effective_name(),
                    plugin_config.plugin_type
                );
                plugin_count += 1;
            }
            Err(e) => {
                error!(
                    "Failed to load plugin {}: {}",
                    plugin_config.effective_name(),
                    e
                );
            }
        }
    }

    info!("Loaded {} plugins", plugin_count);

    // Resolve inter-plugin references (such as fallback primary/secondary plugin names)
    if let Err(e) = builder.resolve_references(&config.plugins) {
        error!("Failed to resolve plugin references: {}", e);
    }

    // Get the plugin registry for servers to use
    let registry = Arc::new(builder.get_registry());
    debug!(plugins = ?registry.plugin_names(), "Plugin registry contents");

    // Launch all configured servers using ServerLauncher
    let launcher = ServerLauncher::new(Arc::clone(&registry));
    let mut startup_receivers = launcher.launch_all(&config.plugins).await;

    // Launch admin API server if enabled
    let config_arc = Arc::new(RwLock::new(config.clone()));
    if let Some(rx) = launcher.launch_admin_server(Arc::clone(&config_arc)).await {
        startup_receivers.push(rx);
    }
    if let Some(rx) = launcher
        .launch_monitoring_server(Arc::clone(&config_arc))
        .await
    {
        startup_receivers.push(rx);
    }
    if let Some(rx) = launcher.launch_web_server(Arc::clone(&config_arc)).await {
        startup_receivers.push(rx);
    }

    // Wait for all servers to start listening
    for rx in startup_receivers {
        let _ = rx.await; // Ignore errors - servers may have exited
    }

    // Give async server tasks a moment to start and output their listening messages
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Start background tasks for plugins that need them
    let _background_tasks = builder.start_background_tasks();

    info!("lazydns initialized successfully");

    // Wait for shutdown signal (Ctrl-C, SIGTERM, SIGHUP)
    if let Err(e) = wait_for_shutdown_signal().await {
        error!("Error waiting for shutdown signal: {}", e);
    }
    info!("Shutdown signal received, beginning graceful shutdown...");

    // Shutdown all plugins that implement the Shutdown trait, with timeout
    match timeout(Duration::from_secs(30), builder.shutdown_all()).await {
        Ok(Ok(())) => info!("Shutdown finished successfully"),
        Ok(Err(e)) => error!("Error during plugin shutdown: {}", e),
        Err(_) => error!("Shutdown timed out after 30s"),
    }

    // Monitoring server listens to OS signals itself for graceful shutdown.

    info!("lazydns exited normally");

    Ok(())
}

#[cfg(feature = "log")]
/// Determine the effective log specification, considering config and CLI overrides.
fn effective_log_spec(config: &LogConfig, cli_verbose: u8) -> String {
    // RUST_LOG takes precedence over everything
    if let Ok(rust_log) = std::env::var("RUST_LOG")
        && !rust_log.is_empty()
    {
        return rust_log;
    }

    // CLI verbose flag overrides config level when > 0
    if cli_verbose > 0 {
        return match cli_verbose {
            1 => format!("{},lazydns=debug", config.level),
            2 => format!("{},lazydns=trace", config.level),
            _ => "trace".to_string(),
        };
    }

    // Use config level with crate-specific override
    if config.level.is_empty() {
        "info,lazydns=info".to_string()
    } else {
        format!("{},lazydns={}", config.level, config.level)
    }
}
