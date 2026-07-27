//! Global upstream health registry
//!
//! Allows forward plugins to register their upstream health data for WebUI access.

use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::Instant;

/// Snapshot of upstream health data
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamHealthSnapshot {
    /// Upstream address
    pub address: String,
    /// Optional tag/name
    pub tag: Option<String>,
    /// Plugin that owns this upstream
    pub plugin: String,
    /// Status (healthy, unhealthy, unknown)
    pub status: String,
    /// Success rate percentage (0-100)
    pub success_rate: f64,
    /// Average response time in milliseconds
    pub avg_response_time_ms: f64,
    /// Total queries
    pub queries: u64,
    /// Successful queries
    pub successes: u64,
    /// Failed queries
    pub failures: u64,
    /// Last successful query time (ISO 8601 format)
    pub last_success: Option<String>,
}

/// Entry in the registry
struct RegistryEntry {
    address: String,
    tag: Option<String>,
    plugin: String,
    /// Callback to get current health data
    health_fn: Box<dyn Fn() -> UpstreamHealthData + Send + Sync>,
}

/// Health data returned by the callback
pub struct UpstreamHealthData {
    pub queries: u64,
    pub successes: u64,
    pub failures: u64,
    pub avg_response_time_us: u64,
    pub last_success: Option<Instant>,
}

/// Global registry for upstream health data
pub struct UpstreamRegistry {
    entries: RwLock<HashMap<String, RegistryEntry>>,
}

impl UpstreamRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Register an upstream
    ///
    /// The key should be unique (such as "plugin_name:address")
    pub fn register<F>(
        &self,
        key: String,
        address: String,
        tag: Option<String>,
        plugin: String,
        health_fn: F,
    ) where
        F: Fn() -> UpstreamHealthData + Send + Sync + 'static,
    {
        let entry = RegistryEntry {
            address,
            tag,
            plugin,
            health_fn: Box::new(health_fn),
        };
        self.entries.write().insert(key, entry);
    }

    /// Unregister an upstream
    pub fn unregister(&self, key: &str) {
        self.entries.write().remove(key);
    }

    /// Get all upstream health snapshots
    pub fn get_all(&self) -> Vec<UpstreamHealthSnapshot> {
        let entries = self.entries.read();
        entries
            .values()
            .map(|entry| {
                let data = (entry.health_fn)();
                let success_rate = if data.queries > 0 {
                    (data.successes as f64 / data.queries as f64) * 100.0
                } else {
                    100.0
                };

                let status = if data.queries == 0 {
                    "unknown".to_string()
                } else if success_rate >= 95.0 {
                    "healthy".to_string()
                } else if success_rate >= 50.0 {
                    "degraded".to_string()
                } else {
                    "unhealthy".to_string()
                };

                // Convert Instant to approximate ISO 8601 timestamp
                // We can only compute elapsed time from tokio::Instant
                let last_success = data.last_success.map(|instant| {
                    let elapsed_secs = instant.elapsed().as_secs();
                    // Format as relative time for now (such as "5s ago", "2m ago")
                    if elapsed_secs < 60 {
                        format!("{}s ago", elapsed_secs)
                    } else if elapsed_secs < 3600 {
                        format!("{}m ago", elapsed_secs / 60)
                    } else if elapsed_secs < 86400 {
                        format!("{}h ago", elapsed_secs / 3600)
                    } else {
                        format!("{}d ago", elapsed_secs / 86400)
                    }
                });

                UpstreamHealthSnapshot {
                    address: entry.address.clone(),
                    tag: entry.tag.clone(),
                    plugin: entry.plugin.clone(),
                    status,
                    success_rate,
                    avg_response_time_ms: data.avg_response_time_us as f64 / 1000.0,
                    queries: data.queries,
                    successes: data.successes,
                    failures: data.failures,
                    last_success,
                }
            })
            .collect()
    }
}

impl Default for UpstreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global upstream registry instance
static UPSTREAM_REGISTRY: once_cell::sync::Lazy<Arc<UpstreamRegistry>> =
    once_cell::sync::Lazy::new(|| Arc::new(UpstreamRegistry::new()));

/// Get the global upstream registry
pub fn upstream_registry() -> Arc<UpstreamRegistry> {
    Arc::clone(&UPSTREAM_REGISTRY)
}

/// Register an upstream to the global registry
pub fn register_upstream<F>(
    key: String,
    address: String,
    tag: Option<String>,
    plugin: String,
    health_fn: F,
) where
    F: Fn() -> UpstreamHealthData + Send + Sync + 'static,
{
    UPSTREAM_REGISTRY.register(key, address, tag, plugin, health_fn);
}

/// Unregister an upstream from the global registry
pub fn unregister_upstream(key: &str) {
    UPSTREAM_REGISTRY.unregister(key);
}

/// Get all upstream health snapshots from the global registry
pub fn get_all_upstream_health() -> Vec<UpstreamHealthSnapshot> {
    UPSTREAM_REGISTRY.get_all()
}
