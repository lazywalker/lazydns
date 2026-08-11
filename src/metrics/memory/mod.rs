//! Memory metrics collection module
//!
//! Collects process memory usage metrics (RSS, VMS, cgroup) and exposes them as
//! Prometheus gauges. Designed for production monitoring with container support.

pub mod cgroup_reader;
pub mod proc_reader;

use once_cell::sync::Lazy;
use prometheus::{IntGauge, Opts};
use std::time::Duration;
use tokio::time;
use tracing::{debug, info, trace, warn};

use crate::metrics::METRICS_REGISTRY;

/// Prometheus gauge for resident memory (RSS) in bytes from /proc
///
/// This metric reports the Resident Set Size - the portion of the process's
/// memory held in RAM (excluding swapped out memory).
pub static PROCESS_RESIDENT_MEMORY_BYTES: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::with_opts(
        Opts::new(
            "lazydns_process_resident_memory_bytes",
            "Process resident memory (RSS) from /proc/self/status in bytes",
        )
        .const_label("source", "proc"),
    )
    .expect("Failed to create lazydns_process_resident_memory_bytes gauge");

    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register lazydns_process_resident_memory_bytes");

    gauge
});

/// Prometheus gauge for virtual memory (VMS) in bytes from /proc
///
/// This metric reports the total virtual memory size of the process,
/// including all mapped memory regions.
pub static PROCESS_VIRTUAL_MEMORY_BYTES: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::with_opts(
        Opts::new(
            "lazydns_process_virtual_memory_bytes",
            "Process virtual memory (VmSize) from /proc/self/status in bytes",
        )
        .const_label("source", "proc"),
    )
    .expect("Failed to create lazydns_process_virtual_memory_bytes gauge");

    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register lazydns_process_virtual_memory_bytes");

    gauge
});

/// Prometheus gauge for cgroup memory usage in bytes
///
/// This metric reports the current memory usage from cgroup limits
/// (v2 or v1). Preferred over /proc metrics in containerized environments
/// as it reflects the container's view of memory usage.
pub static PROCESS_CGROUP_MEMORY_BYTES: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::with_opts(
        Opts::new(
            "lazydns_process_cgroup_memory_bytes",
            "Process memory usage from cgroup (container-aware) in bytes",
        )
        .const_label("source", "cgroup"),
    )
    .expect("Failed to create lazydns_process_cgroup_memory_bytes gauge");

    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register lazydns_process_cgroup_memory_bytes");

    gauge
});

/// Prometheus gauge for cgroup memory limit in bytes
///
/// This metric reports the memory limit set by cgroup. Only populated
/// when running in a container with a memory limit.
pub static PROCESS_CGROUP_MEMORY_LIMIT_BYTES: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::with_opts(
        Opts::new(
            "lazydns_process_cgroup_memory_limit_bytes",
            "Process memory limit from cgroup in bytes (0 = unlimited)",
        )
        .const_label("source", "cgroup"),
    )
    .expect("Failed to create lazydns_process_cgroup_memory_limit_bytes gauge");

    METRICS_REGISTRY
        .register(Box::new(gauge.clone()))
        .expect("Failed to register lazydns_process_cgroup_memory_limit_bytes");

    gauge
});

/// Start memory metrics collection task
///
/// Spawns a background tokio task that periodically samples process memory
/// and updates Prometheus metrics. Prioritizes cgroup metrics when available
/// (container environments) and falls back to /proc metrics.
pub fn start_memory_metrics_collector(
    config: crate::config::MemoryMetricsConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if !config.enabled {
            debug!("Memory metrics collection is disabled");
            return;
        }

        info!(
            "Starting memory metrics collector (interval: {}ms)",
            config.interval_ms
        );

        // Detect cgroup version on startup
        let cgroup_version = cgroup_reader::detect_cgroup_version();
        match cgroup_version {
            Some(v) => info!("Detected cgroup version: {:?}", v),
            None => info!("No cgroup detected, using /proc metrics only"),
        }

        let mut interval = time::interval(Duration::from_millis(config.interval_ms));
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            collect_memory_metrics(cgroup_version.is_some());
        }
    })
}

/// Collect and update memory metrics (single sample)
///
/// This function is called periodically by the collector task.
/// It reads memory stats from cgroup (if available) and /proc,
/// then updates the Prometheus gauges.
fn collect_memory_metrics(has_cgroup: bool) {
    // Priority 1: Try cgroup metrics (container-aware)
    if has_cgroup {
        if let Some(cgroup_stats) = cgroup_reader::read_cgroup_memory() {
            PROCESS_CGROUP_MEMORY_BYTES.set(cgroup_stats.usage_bytes as i64);

            if let Some(limit) = cgroup_stats.limit_bytes {
                PROCESS_CGROUP_MEMORY_LIMIT_BYTES.set(limit as i64);
            } else {
                PROCESS_CGROUP_MEMORY_LIMIT_BYTES.set(0);
            }

            trace!(
                "Updated cgroup memory metrics: usage={}MB, limit={}",
                cgroup_stats.usage_bytes / (1024 * 1024),
                cgroup_stats
                    .limit_bytes
                    .map(|l| format!("{}MB", l / (1024 * 1024)))
                    .unwrap_or_else(|| "unlimited".to_string())
            );
        } else {
            warn!("Failed to read cgroup memory stats");
        }
    }

    // Priority 2: Always collect process memory metrics
    match proc_reader::read_proc_memory() {
        Ok(proc_stats) => {
            PROCESS_RESIDENT_MEMORY_BYTES.set(proc_stats.rss_bytes as i64);
            PROCESS_VIRTUAL_MEMORY_BYTES.set(proc_stats.vms_bytes as i64);

            trace!(
                "Updated process memory metrics: RSS={}MB, VMS={}MB",
                proc_stats.rss_bytes / (1024 * 1024),
                proc_stats.vms_bytes / (1024 * 1024)
            );
        }
        Err(e) => {
            debug!("Failed to read process memory stats: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_metrics_config_default() {
        let config = crate::config::MemoryMetricsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.interval_ms, 5000);
    }

    #[test]
    fn test_memory_metrics_config_disabled() {
        let config = crate::config::MemoryMetricsConfig {
            enabled: false,
            interval_ms: 10000,
        };
        assert!(!config.enabled);
        assert_eq!(config.interval_ms, 10000);
    }

    #[test]
    fn test_metrics_are_registered() {
        // Accessing the lazy statics should register them
        let _ = &*PROCESS_RESIDENT_MEMORY_BYTES;
        let _ = &*PROCESS_VIRTUAL_MEMORY_BYTES;
        let _ = &*PROCESS_CGROUP_MEMORY_BYTES;
        let _ = &*PROCESS_CGROUP_MEMORY_LIMIT_BYTES;

        // Verify they appear in the registry
        let metrics = METRICS_REGISTRY.gather();
        let metric_names: Vec<_> = metrics.iter().map(|m| m.name()).collect();

        assert!(metric_names.contains(&"lazydns_process_resident_memory_bytes"));
        assert!(metric_names.contains(&"lazydns_process_virtual_memory_bytes"));
        assert!(metric_names.contains(&"lazydns_process_cgroup_memory_bytes"));
        assert!(metric_names.contains(&"lazydns_process_cgroup_memory_limit_bytes"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_collect_memory_metrics_integration() {
        // Force metric registration
        let _ = &*PROCESS_RESIDENT_MEMORY_BYTES;
        let _ = &*PROCESS_VIRTUAL_MEMORY_BYTES;

        // Collect metrics once
        let has_cgroup = cgroup_reader::detect_cgroup_version().is_some();
        collect_memory_metrics(has_cgroup);

        // Verify /proc metrics are populated
        let rss = PROCESS_RESIDENT_MEMORY_BYTES.get();
        let vms = PROCESS_VIRTUAL_MEMORY_BYTES.get();

        assert!(rss > 0, "RSS should be > 0");
        assert!(vms > 0, "VMS should be > 0");
        assert!(rss <= vms, "RSS should be <= VMS");
    }

    #[tokio::test]
    async fn test_start_memory_metrics_collector_disabled() {
        let config = crate::config::MemoryMetricsConfig {
            enabled: false,
            interval_ms: 5000,
        };
        let handle = start_memory_metrics_collector(config);

        // Give it a moment to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should complete quickly since disabled
        handle.abort();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_start_memory_metrics_collector_enabled() {
        let config = crate::config::MemoryMetricsConfig {
            enabled: true,
            interval_ms: 100,
        };

        let handle = start_memory_metrics_collector(config);

        // Let it run for a few cycles
        tokio::time::sleep(Duration::from_millis(350)).await;

        // Metrics should be updated
        let rss = PROCESS_RESIDENT_MEMORY_BYTES.get();
        assert!(rss > 0, "RSS should be updated");

        handle.abort();
    }
}
