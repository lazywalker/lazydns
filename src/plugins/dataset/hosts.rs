// Hosts plugin moved under plugins::dataset
// Original implementation preserved.

// (file content copied from src/plugins/hosts.rs)

//! Hosts file plugin
//!
//! This plugin resolves DNS queries using a hosts file, similar to `/etc/hosts`.
//! It provides local DNS resolution for specific domain names.
//!
//! # Features
//!
//! - **Static mappings**: Map domain names to IP addresses
//! - **Multiple addresses**: Support multiple IPs per domain
//! - **Case-insensitive**: Domain name matching is case-insensitive
//! - **Fast lookup**: HashMap-based O(1) lookups
//! - **Auto-reload**: Watch files for changes and reload automatically
//!
//! # Example
//!
//! ```rust
//! use lazydns::plugins::HostsPlugin;
//! use lazydns::plugin::Plugin;
//! use std::sync::Arc;
//! use std::net::{Ipv4Addr, Ipv6Addr};
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let plugin = HostsPlugin::new();
//! plugin.add_host("localhost".to_string(), Ipv4Addr::new(127, 0, 0, 1).into());
//! plugin.add_host("localhost".to_string(), Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1).into());
//!
//! let arc_plugin: Arc<dyn Plugin> = Arc::new(plugin);
//! # Ok(())
//! # }
//! ```

use crate::config::PluginConfig;
use crate::dns::{Message, Question, RData, RecordType, ResourceRecord};
use crate::error::Error;
use crate::plugin::{Context, Plugin, traits::Shutdown};
use crate::{RegisterPlugin, Result, ShutdownPlugin};
use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info, warn};

// Auto-register using the register macro

/// Core hosts parsing and lookup store
///
/// Maps domain names to IP addresses, similar to `/etc/hosts` file.
#[derive(Clone)]
pub struct Hosts {
    /// Domain name to list of IP addresses
    hosts: Arc<RwLock<HashMap<String, Vec<IpAddr>>>>,
}

impl Hosts {
    pub fn new() -> Self {
        Self {
            hosts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add_host(&self, domain: String, ip: IpAddr) {
        let domain_lower = domain.trim_end_matches('.').to_lowercase();
        self.hosts.write().entry(domain_lower).or_default().push(ip);
    }

    pub fn remove_host(&self, domain: &str) -> bool {
        let domain_lower = domain.trim_end_matches('.').to_lowercase();
        self.hosts.write().remove(&domain_lower).is_some()
    }

    pub fn get_ips(&self, domain: &str) -> Option<Vec<IpAddr>> {
        let domain_lower = domain.trim_end_matches('.').to_lowercase();
        self.hosts.read().get(&domain_lower).cloned()
    }

    pub fn len(&self) -> usize {
        self.hosts.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.hosts.read().is_empty()
    }

    pub fn clear(&self) {
        self.hosts.write().clear();
    }

    pub fn load_from_string(&self, content: &str) -> Result<()> {
        let mut new_hosts = HashMap::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let mut ips: Vec<IpAddr> = Vec::new();
            let mut hostnames: Vec<&str> = Vec::new();

            for &token in &parts {
                if let Ok(ip) = token.parse::<IpAddr>() {
                    ips.push(ip);
                } else {
                    hostnames.push(token);
                }
            }

            if ips.is_empty() {
                return Err(Error::FileParse {
                    path: "hosts".to_string(),
                    reason: format!("No valid IP found in line: {}", line),
                });
            }

            for &hostname in &hostnames {
                let domain_lower = hostname.to_lowercase();
                new_hosts
                    .entry(domain_lower)
                    .or_insert_with(Vec::new)
                    .extend(ips.iter().cloned());
            }
        }

        // Replace old hosts and explicitly drop old value to free memory immediately
        {
            let mut writer = self.hosts.write();
            let _ = std::mem::replace(&mut *writer, new_hosts);
            // Explicitly drop the old hosts when writer scope ends
        }
        Ok(())
    }
}

impl Default for Hosts {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Hosts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hosts")
            .field("entries", &self.hosts.read().len())
            .finish()
    }
}

/// Hosts plugin wrapper: lifecycle, file watching and Plugin impl
#[derive(RegisterPlugin, ShutdownPlugin)]
pub struct HostsPlugin {
    hosts: Arc<Hosts>,
    files: Vec<PathBuf>,
    auto_reload: bool,
    /// Optional file watcher handle for auto-reload
    watcher: Arc<parking_lot::Mutex<Option<crate::utils::FileWatcherHandle>>>,
}

impl HostsPlugin {
    pub fn new() -> Self {
        Self {
            hosts: Arc::new(Hosts::new()),
            files: Vec::new(),
            auto_reload: false,
            watcher: Arc::new(parking_lot::Mutex::new(None)),
        }
    }

    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files.into_iter().map(PathBuf::from).collect();
        self
    }

    pub fn with_auto_reload(mut self, enabled: bool) -> Self {
        self.auto_reload = enabled;
        self
    }

    pub fn load_hosts(&self) -> Result<()> {
        let mut combined = String::new();
        for file_path in &self.files {
            match std::fs::read_to_string(file_path) {
                Ok(c) => {
                    combined.push_str(&c);
                    combined.push('\n');
                }
                Err(e) => warn!(file = ?file_path, error = %e, "Failed to read hosts file"),
            }
        }

        if !combined.is_empty() {
            self.hosts.load_from_string(&combined)?;
            // Release unused memory back to the OS after initial load (platform guarded)
            crate::utils::malloc_trim_hint();
        }

        info!(
            entries = self.hosts.len(),
            files = self.files.len(),
            "Hosts loaded (wrapper)"
        );
        Ok(())
    }

    pub fn start_file_watcher(&self) {
        if !self.auto_reload || self.files.is_empty() {
            return;
        }

        let files = self.files.clone();
        let hosts_weak = Arc::downgrade(&self.hosts);

        debug!(auto_reload = true, files = ?files, "file auto-reload status");

        const DEBOUNCE_MS: u64 = 200;

        let handle = crate::utils::spawn_file_watcher(
            "hosts",
            files.clone(),
            DEBOUNCE_MS,
            move |_path, files| {
                let file_name = files
                    .first()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                let start = std::time::Instant::now();
                let mut combined = String::new();

                for file_path in files {
                    if let Ok(content) = std::fs::read_to_string(file_path) {
                        combined.push_str(&content);
                        combined.push('\n');
                    } else {
                        warn!(file = ?file_path, "Failed to read hosts file");
                    }
                }

                // Upgrade weak reference to Arc, if plugin still exists
                if let Some(hosts) = hosts_weak.upgrade() {
                    if !combined.is_empty() {
                        if let Err(e) = hosts.load_from_string(&combined) {
                            warn!(error = %e, "Failed to parse hosts file during auto-reload");
                        } else {
                            // Release unused memory back to the OS after reload (platform guarded)
                            crate::utils::malloc_trim_hint();
                        }
                    }
                } else {
                    warn!("hosts plugin dropped, skipping reload");
                    return;
                }

                let duration = start.elapsed();
                info!(filename = file_name, duration = ?duration, "scheduled auto-reload completed");
            },
        );

        // Store handle so we can stop it on shutdown
        let mut guard = self.watcher.lock();
        *guard = Some(handle);
    }

    fn create_response(&self, question: &Question, ips: &[IpAddr]) -> Message {
        let mut response = Message::new();
        response.set_response(true);
        response.set_authoritative(true);
        response.set_recursion_available(false);

        response.add_question(question.clone());

        let qtype = question.qtype();
        let qname = question.qname().to_string();
        let qclass = question.qclass();

        for ip in ips {
            let record = match (ip, qtype) {
                (IpAddr::V4(ipv4), RecordType::A) => Some(ResourceRecord::new(
                    qname.clone(),
                    RecordType::A,
                    qclass,
                    3600,
                    RData::A(*ipv4),
                )),
                (IpAddr::V6(ipv6), RecordType::AAAA) => Some(ResourceRecord::new(
                    qname.clone(),
                    RecordType::AAAA,
                    qclass,
                    3600,
                    RData::AAAA(*ipv6),
                )),
                _ => None,
            };

            if let Some(r) = record {
                response.add_answer(r);
            }
        }

        response
    }
}

impl Default for HostsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for HostsPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostsPlugin")
            .field("entries", &self.hosts.len())
            .finish()
    }
}

#[async_trait]
impl Shutdown for HostsPlugin {
    async fn shutdown(&self) -> Result<()> {
        // Stop file watcher if running
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

impl Deref for HostsPlugin {
    type Target = Hosts;

    fn deref(&self) -> &Hosts {
        &self.hosts
    }
}

#[async_trait]
impl Plugin for HostsPlugin {
    async fn execute(&self, context: &mut Context) -> Result<()> {
        if context.response().is_some() {
            return Ok(());
        }

        let question = match context.request().questions().first() {
            Some(q) => q,
            None => return Ok(()),
        };

        let qtype = question.qtype();
        if qtype != RecordType::A && qtype != RecordType::AAAA {
            return Ok(());
        }

        let domain = question.qname();
        if let Some(ips) = self.get_ips(domain) {
            debug!("Hosts plugin: Found {} IPs for {}", ips.len(), domain);

            let mut response = self.create_response(question, &ips);
            response.set_id(context.request().id());
            response.set_response_code(crate::dns::ResponseCode::NoError);

            context.set_response(Some(response));
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "hosts"
    }

    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>> {
        let args = config.effective_args();
        use serde_yaml::Value;

        let mut plugin = HostsPlugin::new();

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

        if let Err(e) = plugin.load_hosts() {
            tracing::warn!(error = %e, "Failed to load hosts during init, continuing");
        }

        plugin.start_file_watcher();

        Ok(Arc::new(plugin))
    }
}

// Tests preserved in dataset hosts module
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::RecordClass;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_hosts_new() {
        let hosts = Hosts::new();
        assert!(hosts.is_empty());
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_hosts_default() {
        let hosts = Hosts::default();
        assert!(hosts.is_empty());
    }

    #[test]
    fn test_hosts_debug() {
        let hosts = Hosts::new();
        hosts.add_host("example.com".to_string(), Ipv4Addr::new(1, 2, 3, 4).into());
        let debug_str = format!("{:?}", hosts);
        assert!(debug_str.contains("Hosts"));
        assert!(debug_str.contains("entries"));
    }

    #[test]
    fn test_add_host_ipv4() {
        let hosts = Hosts::new();
        let ip = Ipv4Addr::new(127, 0, 0, 1);
        hosts.add_host("localhost".to_string(), ip.into());

        let ips = hosts.get_ips("localhost").unwrap();
        assert_eq!(ips.len(), 1);
        assert_eq!(ips[0], IpAddr::V4(ip));
    }

    #[test]
    fn test_add_host_ipv6() {
        let hosts = Hosts::new();
        let ip = Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1);
        hosts.add_host("localhost".to_string(), ip.into());

        let ips = hosts.get_ips("localhost").unwrap();
        assert_eq!(ips.len(), 1);
        assert_eq!(ips[0], IpAddr::V6(ip));
    }

    #[test]
    fn test_add_host_multiple_ips() {
        let hosts = Hosts::new();
        let ip4 = Ipv4Addr::new(127, 0, 0, 1);
        let ip6 = Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1);

        hosts.add_host("localhost".to_string(), ip4.into());
        hosts.add_host("localhost".to_string(), ip6.into());

        let ips = hosts.get_ips("localhost").unwrap();
        assert_eq!(ips.len(), 2);
    }

    #[test]
    fn test_case_insensitive() {
        let hosts = Hosts::new();
        let ip = Ipv4Addr::new(93, 184, 216, 34);

        hosts.add_host("Example.COM".to_string(), ip.into());

        assert!(hosts.get_ips("example.com").is_some());
        assert!(hosts.get_ips("EXAMPLE.COM").is_some());
        assert!(hosts.get_ips("Example.Com").is_some());
        assert!(hosts.get_ips("Example.Com.").is_some());
    }

    #[test]
    fn test_trailing_dot_normalized() {
        let hosts = Hosts::new();
        let ip = Ipv4Addr::new(1, 2, 3, 4);

        hosts.add_host("example.com.".to_string(), ip.into());

        assert!(hosts.get_ips("example.com").is_some());
        assert!(hosts.get_ips("example.com.").is_some());
    }

    #[test]
    fn test_remove_host() {
        let hosts = Hosts::new();
        let ip = Ipv4Addr::new(1, 2, 3, 4);

        hosts.add_host("example.com".to_string(), ip.into());
        assert!(!hosts.is_empty());

        let removed = hosts.remove_host("example.com");
        assert!(removed);
        assert!(hosts.is_empty());
    }

    #[test]
    fn test_remove_host_not_found() {
        let hosts = Hosts::new();
        let removed = hosts.remove_host("nonexistent.com");
        assert!(!removed);
    }

    #[test]
    fn test_get_ips_not_found() {
        let hosts = Hosts::new();
        assert!(hosts.get_ips("nonexistent.com").is_none());
    }

    #[test]
    fn test_clear() {
        let hosts = Hosts::new();
        hosts.add_host("a.com".to_string(), Ipv4Addr::new(1, 1, 1, 1).into());
        hosts.add_host("b.com".to_string(), Ipv4Addr::new(2, 2, 2, 2).into());
        assert_eq!(hosts.len(), 2);

        hosts.clear();
        assert!(hosts.is_empty());
    }

    #[test]
    fn test_load_from_string_simple() {
        let hosts = Hosts::new();
        let content = "127.0.0.1 localhost";
        hosts.load_from_string(content).unwrap();

        let ips = hosts.get_ips("localhost").unwrap();
        assert_eq!(ips.len(), 1);
        assert_eq!(ips[0], IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn test_load_from_string_multiple_hosts_per_line() {
        let hosts = Hosts::new();
        let content = "127.0.0.1 localhost loopback";
        hosts.load_from_string(content).unwrap();

        assert!(hosts.get_ips("localhost").is_some());
        assert!(hosts.get_ips("loopback").is_some());
    }

    #[test]
    fn test_load_from_string_comments() {
        let hosts = Hosts::new();
        let content = r#"
# This is a comment
127.0.0.1 localhost
# Another comment
"#;
        hosts.load_from_string(content).unwrap();
        assert_eq!(hosts.len(), 1);
    }

    #[test]
    fn test_load_from_string_empty_lines() {
        let hosts = Hosts::new();
        let content = r#"

127.0.0.1 localhost

192.168.1.1 router

"#;
        hosts.load_from_string(content).unwrap();
        assert_eq!(hosts.len(), 2);
    }

    #[test]
    fn test_load_from_string_ipv6() {
        let hosts = Hosts::new();
        let content = "::1 localhost";
        hosts.load_from_string(content).unwrap();

        let ips = hosts.get_ips("localhost").unwrap();
        assert_eq!(ips.len(), 1);
        assert!(matches!(ips[0], IpAddr::V6(_)));
    }

    #[test]
    fn test_load_from_string_no_ip_error() {
        let hosts = Hosts::new();
        let content = "not-an-ip localhost";
        let result = hosts.load_from_string(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_from_string_replaces_previous() {
        let hosts = Hosts::new();
        hosts.add_host("old.com".to_string(), Ipv4Addr::new(1, 1, 1, 1).into());

        let content = "2.2.2.2 new.com";
        hosts.load_from_string(content).unwrap();

        // Old entry should be gone
        assert!(hosts.get_ips("old.com").is_none());
        assert!(hosts.get_ips("new.com").is_some());
    }

    #[test]
    fn test_hosts_plugin_new() {
        let plugin = HostsPlugin::new();
        assert!(plugin.hosts.is_empty());
        assert!(plugin.files.is_empty());
        assert!(!plugin.auto_reload);
    }

    #[test]
    fn test_hosts_plugin_default() {
        let plugin = HostsPlugin::default();
        assert!(plugin.hosts.is_empty());
    }

    #[test]
    fn test_hosts_plugin_with_files() {
        let plugin = HostsPlugin::new().with_files(vec!["/etc/hosts".to_string()]);
        assert_eq!(plugin.files.len(), 1);
    }

    #[test]
    fn test_hosts_plugin_with_auto_reload() {
        let plugin = HostsPlugin::new().with_auto_reload(true);
        assert!(plugin.auto_reload);
    }

    #[test]
    fn test_hosts_plugin_add_host() {
        let plugin = HostsPlugin::new();
        plugin.add_host("example.com".to_string(), Ipv4Addr::new(1, 2, 3, 4).into());
        assert!(!plugin.hosts.is_empty());
    }

    #[test]
    fn test_hosts_plugin_deref() {
        let plugin = HostsPlugin::new();
        plugin.add_host("test.com".to_string(), Ipv4Addr::new(1, 1, 1, 1).into());

        // Test Deref trait
        let hosts: &Hosts = &plugin;
        assert!(hosts.get_ips("test.com").is_some());
    }

    #[test]
    fn test_hosts_plugin_debug() {
        let plugin = HostsPlugin::new();
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("HostsPlugin"));
    }

    #[test]
    fn test_hosts_plugin_start_watcher_no_files() {
        let plugin = HostsPlugin::new().with_auto_reload(true);
        // Should not panic when no files
        plugin.start_file_watcher();
    }

    #[test]
    fn test_hosts_plugin_start_watcher_disabled() {
        let plugin = HostsPlugin::new()
            .with_files(vec!["/etc/hosts".to_string()])
            .with_auto_reload(false);
        // Should not start watcher when disabled
        plugin.start_file_watcher();
    }

    #[tokio::test]
    async fn test_hosts_plugin_shutdown_stops_watcher() {
        use tempfile::NamedTempFile;
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let plugin = HostsPlugin::new()
            .with_files(vec![path.clone()])
            .with_auto_reload(true);

        plugin.start_file_watcher();

        // Give watcher a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Shutdown should stop watcher without panic
        Shutdown::shutdown(&plugin).await.unwrap();
    }

    #[tokio::test]
    async fn test_hosts_plugin_execute_found_ipv4() {
        let plugin = HostsPlugin::new();
        plugin.add_host("test.local".to_string(), Ipv4Addr::new(10, 0, 0, 1).into());

        let mut request = Message::new();
        request.add_question(Question::new("test.local", RecordType::A, RecordClass::IN));

        let mut ctx = Context::new(request);
        plugin.execute(&mut ctx).await.unwrap();

        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();
        assert!(!resp.answers().is_empty());
    }

    #[tokio::test]
    async fn test_hosts_plugin_execute_found_ipv6() {
        let plugin = HostsPlugin::new();
        plugin.add_host(
            "test.local".to_string(),
            Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1).into(),
        );

        let mut request = Message::new();
        request.add_question(Question::new(
            "test.local",
            RecordType::AAAA,
            RecordClass::IN,
        ));

        let mut ctx = Context::new(request);
        plugin.execute(&mut ctx).await.unwrap();

        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();
        assert!(!resp.answers().is_empty());
    }

    #[tokio::test]
    async fn test_hosts_plugin_execute_not_found() {
        let plugin = HostsPlugin::new();

        let mut request = Message::new();
        request.add_question(Question::new(
            "unknown.local",
            RecordType::A,
            RecordClass::IN,
        ));

        let mut ctx = Context::new(request);
        plugin.execute(&mut ctx).await.unwrap();

        // No response set when not found
        assert!(ctx.response().is_none());
    }

    #[tokio::test]
    async fn test_hosts_plugin_execute_wrong_type() {
        let plugin = HostsPlugin::new();
        plugin.add_host("test.local".to_string(), Ipv4Addr::new(10, 0, 0, 1).into());

        // Query for AAAA but only IPv4 is available
        let mut request = Message::new();
        request.add_question(Question::new(
            "test.local",
            RecordType::AAAA,
            RecordClass::IN,
        ));

        let mut ctx = Context::new(request);
        plugin.execute(&mut ctx).await.unwrap();

        // Should return empty response (no matching records)
        if let Some(resp) = ctx.response() {
            assert!(resp.answers().is_empty());
        }
    }
}
