use crate::plugin::traits::{Matcher, Shutdown};
use crate::plugin::{Context, Plugin};
use crate::{RegisterPlugin, Result, ShutdownPlugin, config::PluginConfig, dns::RData};
use async_trait::async_trait;
use ipnet::IpNet;
use parking_lot::RwLock;
use std::fmt;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// IP set data provider plugin
///
/// Loads IP addresses and CIDR ranges from files and provides them for matching.
/// Supports auto-reload when files change.
///
/// # Example
///
/// ```rust
/// use lazydns::plugins::IpSetPlugin;
///
/// let plugin = IpSetPlugin::new("local-ips")
///     .with_files(vec!["china-ip-list.txt".to_string()])
///     .with_auto_reload(true);
/// ```
#[derive(Clone, RegisterPlugin, ShutdownPlugin)]
pub struct IpSetPlugin {
    /// Name/tag for this IP set
    name: String,
    /// Files to load IPs from
    files: Vec<PathBuf>,
    /// Inline IP addresses/networks
    ips: Vec<String>,
    /// Whether to auto-reload files
    auto_reload: bool,
    /// Loaded IP networks (stored in shared state)
    networks: Arc<RwLock<Vec<IpNet>>>,
    /// Plugin tag from YAML configuration
    tag: Option<String>,
    /// Optional file watcher handle for auto-reload
    watcher: Arc<parking_lot::Mutex<Option<crate::utils::FileWatcherHandle>>>,
}

impl IpSetPlugin {
    /// Create a new IP set plugin
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            files: Vec::new(),
            ips: Vec::new(),
            auto_reload: false,
            networks: Arc::new(RwLock::new(Vec::new())),
            tag: None,
            watcher: Arc::new(parking_lot::Mutex::new(None)),
        }
    }

    /// Add files to load IPs from
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files.into_iter().map(PathBuf::from).collect();
        self
    }

    /// Add inline IP addresses/networks
    pub fn with_ips(mut self, ips: Vec<String>) -> Self {
        self.ips = ips;
        self
    }

    /// Enable auto-reload
    pub fn with_auto_reload(mut self, enabled: bool) -> Self {
        self.auto_reload = enabled;
        self
    }

    /// Start file watcher if auto-reload is enabled
    pub fn start_file_watcher(&self) {
        if !self.auto_reload || self.files.is_empty() {
            return;
        }

        let name = self.name.clone();
        let files = self.files.clone();
        let networks_weak = Arc::downgrade(&self.networks);

        debug!(
            name = %name,
            auto_reload = true,
            files = ?files,
            "file auto-reload status"
        );

        const DEBOUNCE_MS: u64 = 200;

        let handle = crate::utils::spawn_file_watcher(
            name.clone(),
            files.clone(),
            DEBOUNCE_MS,
            move |path, files| {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                // Reload the IP networks
                info!(name = %name, file = file_name, "scheduled reload: invoking callback");

                let start = std::time::Instant::now();
                let mut new_networks = Vec::new();

                for file_path in files {
                    if let Ok(content) = fs::read_to_string(file_path) {
                        for line in content.lines() {
                            let line = line.trim();
                            if line.is_empty() || line.starts_with('#') {
                                continue;
                            }

                            match line.parse::<IpNet>() {
                                Ok(net) => new_networks.push(net),
                                Err(_) => {
                                    if let Ok(ip) = line.parse::<IpAddr>() {
                                        let net = match ip {
                                            IpAddr::V4(addr) => {
                                                IpNet::new(addr.into(), 32).unwrap()
                                            }
                                            IpAddr::V6(addr) => {
                                                IpNet::new(addr.into(), 128).unwrap()
                                            }
                                        };
                                        new_networks.push(net);
                                    }
                                }
                            }
                        }
                    }
                }

                let count = new_networks.len();

                // Upgrade weak reference to Arc, if plugin still exists
                if let Some(networks) = networks_weak.upgrade() {
                    // Replace old networks and explicitly drop old value to free memory immediately
                    {
                        let mut writer = networks.write();
                        let _ = std::mem::replace(&mut *writer, new_networks);
                        // Explicitly drop the old networks when writer scope ends
                    }
                    // Hint to allocator to release freed memory back to the OS after reload (platform guarded)
                    crate::utils::malloc_trim_hint();
                } else {
                    warn!(name = %name, "ip_set plugin dropped, skipping reload");
                    return;
                }

                let duration = start.elapsed();

                info!(
                    name = %name,
                    IPs = 0,
                    files = files.len(),
                    netlist = count,
                    "successfully loaded IPs and files"
                );

                info!(name = %name, filename = file_name, duration = ?duration, "scheduled auto-reload completed");
            },
        );

        // Store handle so we can stop it on shutdown
        let mut guard = self.watcher.lock();
        *guard = Some(handle);
    }

    /// Load IP networks from all configured files
    pub fn load_networks(&self) -> Result<()> {
        let mut networks = Vec::new();

        // Load from files first
        for file_path in &self.files {
            match self.load_ip_file(file_path) {
                Ok(file_networks) => {
                    debug!(
                        file = ?file_path,
                        count = file_networks.len(),
                        "Loaded IP networks from file"
                    );
                    networks.extend(file_networks);
                }
                Err(e) => {
                    error!(
                        file = ?file_path,
                        error = %e,
                        "Failed to load IP file"
                    );
                    // Continue loading other files
                }
            }
        }

        // Then load from inline IPs
        for ip_str in &self.ips {
            match ip_str.parse::<IpNet>() {
                Ok(net) => networks.push(net),
                Err(_) => {
                    // Try parsing as single IP
                    if let Ok(ip) = ip_str.parse::<IpAddr>() {
                        let net = match ip {
                            IpAddr::V4(addr) => IpNet::new(addr.into(), 32).unwrap(),
                            IpAddr::V6(addr) => IpNet::new(addr.into(), 128).unwrap(),
                        };
                        networks.push(net);
                    } else {
                        debug!(ip = %ip_str, "Skipping invalid IP/CIDR in ips");
                    }
                }
            }
        }

        let count = networks.len();
        // Replace old networks and explicitly drop old value to free memory immediately
        {
            let mut writer = self.networks.write();
            let _ = std::mem::replace(&mut *writer, networks);
            // Explicitly drop the old networks when writer scope ends
        }

        // Hint to allocator to release freed memory back to the OS after loading (platform guarded)
        crate::utils::malloc_trim_hint();

        info!(
            name = %self.name,
            count = count,
            files = self.files.len(),
            ips = self.ips.len(),
            "IP set loaded"
        );

        Ok(())
    }

    /// Load IP networks from a single file
    fn load_ip_file(&self, path: &Path) -> Result<Vec<IpNet>> {
        let content = fs::read_to_string(path)
            .map_err(|e| crate::Error::Config(format!("Failed to read file {:?}: {}", path, e)))?;

        let mut networks = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse as CIDR or single IP
            match line.parse::<IpNet>() {
                Ok(net) => networks.push(net),
                Err(_) => {
                    // Try parsing as single IP
                    if let Ok(ip) = line.parse::<IpAddr>() {
                        // Convert single IP to /32 or /128 network
                        let net = match ip {
                            IpAddr::V4(addr) => IpNet::new(addr.into(), 32).unwrap(),
                            IpAddr::V6(addr) => IpNet::new(addr.into(), 128).unwrap(),
                        };
                        networks.push(net);
                    } else {
                        debug!(line = line, "Skipping invalid IP/CIDR line");
                    }
                }
            }
        }

        Ok(networks)
    }

    /// Check if an IP matches the set
    pub fn matches(&self, ip: &IpAddr) -> bool {
        let networks = self.networks.read();
        for net in networks.iter() {
            if net.contains(ip) {
                return true;
            }
        }
        false
    }

    /// Get the IP networks for use by other plugins
    pub fn get_networks(&self) -> Arc<RwLock<Vec<IpNet>>> {
        Arc::clone(&self.networks)
    }
}

impl fmt::Debug for IpSetPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IpSetPlugin")
            .field("name", &self.name)
            .field("files", &self.files)
            .field("auto_reload", &self.auto_reload)
            .field("network_count", &self.networks.read().len())
            .finish()
    }
}

#[async_trait]
impl Plugin for IpSetPlugin {
    fn name(&self) -> &str {
        "ip_set"
    }

    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        // Store the IP set in context metadata for other plugins to use
        ctx.set_metadata(format!("ip_set_{}", self.name), Arc::clone(&self.networks));
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn init(config: &PluginConfig) -> Result<std::sync::Arc<dyn Plugin>> {
        let args = config.effective_args();
        use serde_yaml::Value;

        let tag = args.get("tag").and_then(|v| v.as_str()).unwrap_or("");
        let name = if !tag.is_empty() {
            tag.to_string()
        } else {
            config.effective_name().to_string()
        };

        let mut plugin = IpSetPlugin::new(name);
        plugin.tag = config.tag.clone();

        if let Some(files_val) = args.get("files") {
            match files_val {
                Value::Sequence(seq) => {
                    let files: Vec<String> = seq
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    plugin = plugin.with_files(files);
                }
                Value::String(s) => {
                    plugin = plugin.with_files(vec![s.clone()]);
                }
                _ => {}
            }
        }

        if let Some(Value::Bool(b)) = args.get("auto_reload") {
            plugin = plugin.with_auto_reload(*b);
        }

        // Parse inline IP addresses (ips parameter)
        if let Some(ips_val) = args.get("ips") {
            let ips: Vec<String> = match ips_val {
                Value::Sequence(seq) => seq
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
            plugin = plugin.with_ips(ips);
        }

        // Load and start watcher as per legacy behavior
        if let Err(e) = plugin.load_networks() {
            tracing::warn!(error = %e, "Failed to load IPs during init, continuing");
        }
        plugin.start_file_watcher();

        Ok(Arc::new(plugin))
    }
}

impl Matcher for IpSetPlugin {
    fn matches_context(&self, ctx: &crate::plugin::Context) -> bool {
        if let Some(response) = ctx.response() {
            for record in response.answers() {
                if let Some(ip) = {
                    let rdata = record.rdata();
                    match rdata {
                        RData::A(ipv4) => Some(IpAddr::V4(*ipv4)),
                        RData::AAAA(ipv6) => Some(IpAddr::V6(*ipv6)),
                        _ => None,
                    }
                } && self.matches(&ip)
                {
                    return true;
                }
            }
        }
        false
    }
}

#[async_trait]
impl Shutdown for IpSetPlugin {
    async fn shutdown(&self) -> Result<()> {
        let handle = {
            let mut guard = self.watcher.lock();
            guard.take()
        };
        if let Some(h) = handle {
            h.stop().await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_ip_set_creation() {
        let plugin = IpSetPlugin::new("test");
        assert_eq!(plugin.name, "test");
        assert!(plugin.files.is_empty());
        assert!(!plugin.auto_reload);
    }

    #[test]
    fn test_ip_set_builder() {
        let plugin = IpSetPlugin::new("test")
            .with_files(vec!["file1.txt".to_string()])
            .with_auto_reload(true);

        assert_eq!(plugin.files.len(), 1);
        assert!(plugin.auto_reload);
    }

    #[test]
    fn test_ip_matching() {
        let plugin = IpSetPlugin::new("test");
        {
            let mut networks = plugin.networks.write();
            networks.push("192.168.0.0/16".parse().unwrap());
            networks.push("10.0.0.0/8".parse().unwrap());
        }

        assert!(plugin.matches(&"192.168.1.1".parse().unwrap()));
        assert!(plugin.matches(&"10.5.5.5".parse().unwrap()));
        assert!(!plugin.matches(&"8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn test_load_ip_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "# Comment line").unwrap();
        writeln!(file, "192.168.0.0/16").unwrap();
        writeln!(file, "10.0.0.1").unwrap();
        writeln!(file, "2001:db8::/32").unwrap();
        // trailing newline not needed
        file.flush().unwrap();

        let plugin = IpSetPlugin::new("test");
        let networks = plugin.load_ip_file(file.path()).unwrap();

        assert_eq!(networks.len(), 3);
    }

    #[tokio::test]
    async fn test_ip_set_plugin_execution() {
        let plugin = IpSetPlugin::new("test");
        let request = crate::dns::Message::new();
        let mut ctx = Context::new(request);

        plugin.execute(&mut ctx).await.unwrap();

        // Verify metadata was set
        assert!(ctx.has_metadata("ip_set_test"));
    }

    #[test]
    fn test_init_with_ips() {
        use serde_yaml::Value;

        let mut config_args = serde_yaml::Mapping::new();
        config_args.insert(
            Value::String("ips".to_string()),
            Value::Sequence(vec![
                Value::String("1.1.1.1".to_string()),
                Value::String("192.168.0.0/16".to_string()),
                Value::String("2001:db8::/32".to_string()),
            ]),
        );

        let config = crate::config::PluginConfig {
            tag: Some("test".to_string()),
            plugin_type: "ip_set".to_string(),
            args: Value::Mapping(config_args),
        };

        let plugin_arc = IpSetPlugin::init(&config).expect("Failed to init");
        let plugin = plugin_arc.as_any().downcast_ref::<IpSetPlugin>().unwrap();

        // Verify IPs were loaded
        let networks = plugin.networks.read();
        assert_eq!(networks.len(), 3);

        // Verify they match correctly
        assert!(plugin.matches(&"1.1.1.1".parse().unwrap()));
        assert!(plugin.matches(&"192.168.1.1".parse().unwrap()));
        assert!(plugin.matches(&"2001:db8:0:0:0:0:0:1".parse().unwrap()));
        assert!(!plugin.matches(&"8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn test_init_with_ips_single_string() {
        use serde_yaml::Value;

        let mut config_args = serde_yaml::Mapping::new();
        config_args.insert(
            Value::String("ips".to_string()),
            Value::String("10.0.0.0/8".to_string()),
        );

        let config = crate::config::PluginConfig {
            tag: Some("test".to_string()),
            plugin_type: "ip_set".to_string(),
            args: Value::Mapping(config_args),
        };

        let plugin_arc = IpSetPlugin::init(&config).expect("Failed to init");
        let plugin = plugin_arc.as_any().downcast_ref::<IpSetPlugin>().unwrap();

        assert!(plugin.matches(&"10.5.5.5".parse().unwrap()));
        assert!(!plugin.matches(&"192.168.1.1".parse().unwrap()));
    }
}
