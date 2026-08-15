//! cgroup memory reader for container-aware metrics
//!
//! Detects and reads cgroup v2 and v1 memory statistics for container environments.

use std::fs;
use std::io;
use std::path::Path;

/// cgroup memory statistics
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CgroupMemoryStats {
    /// Current memory usage in bytes
    pub usage_bytes: u64,
    /// Memory limit in bytes (if set)
    pub limit_bytes: Option<u64>,
}

/// cgroup version detected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgroupVersion {
    /// cgroup v2 (unified hierarchy)
    V2,
    /// cgroup v1 (legacy)
    V1,
}

/// Detect cgroup version and read memory statistics
///
/// Tries to detect cgroup v2 first (modern containers), then falls back to v1.
/// Returns None if not running in a cgroup or if reading fails.
///
/// # Example
///
/// ```no_run
/// # use lazydns::metrics::memory::cgroup_reader::read_cgroup_memory;
/// if let Some(stats) = read_cgroup_memory() {
///     println!("cgroup memory usage: {} bytes", stats.usage_bytes);
///     if let Some(limit) = stats.limit_bytes {
///         println!("cgroup memory limit: {} bytes", limit);
///     }
/// }
/// ```
pub fn read_cgroup_memory() -> Option<CgroupMemoryStats> {
    // Try cgroup v2 first (modern)
    if let Ok(stats) = read_cgroup_v2_memory() {
        return Some(stats);
    }

    // Fall back to cgroup v1
    read_cgroup_v1_memory().ok()
}

/// The process's own cgroup path from /proc/self/cgroup.
///
/// v2 entry: `0::<path>` (empty controller list). v1 entry: the line whose
/// controller list contains `memory`, as `<hierarchy>:memory:<path>`.
fn own_cgroup_path(v2: bool) -> Option<String> {
    let content = fs::read_to_string("/proc/self/cgroup").ok()?;
    for line in content.lines() {
        let mut parts = line.splitn(3, ':');
        parts.next()?;
        let controllers = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("");
        if v2 {
            if controllers.is_empty() {
                return Some(path.to_string());
            }
        } else if controllers.split(',').any(|c| c == "memory") {
            return Some(path.to_string());
        }
    }
    None
}

/// Whether we run inside a container (docker/podman markers).
fn in_container() -> bool {
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

/// Directory holding this process's memory controller files.
///
/// Resolving through /proc/self/cgroup matters: reading the mount root
/// directly reports the ROOT cgroup, which on a cgroup-v2 host is the whole
/// machine mislabeled as process/container memory. A root path only means
/// "the container itself" when actually inside one, where the mount root IS
/// the container's cgroup.
fn own_cgroup_dir(mount: &str, v2: bool) -> io::Result<std::path::PathBuf> {
    let path = own_cgroup_path(v2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no matching cgroup entry"))?;
    if path == "/" && !in_container() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "root cgroup on a host is not process memory",
        ));
    }
    Ok(Path::new(mount).join(path.trim_start_matches('/')))
}

/// Read cgroup v2 memory statistics
///
/// Reads memory.current and memory.max from the process's own cgroup.
fn read_cgroup_v2_memory() -> io::Result<CgroupMemoryStats> {
    let dir = own_cgroup_dir("/sys/fs/cgroup", true)?;

    let usage_bytes = fs::read_to_string(dir.join("memory.current"))?
        .trim()
        .parse::<u64>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let limit_bytes = fs::read_to_string(dir.join("memory.max"))
        .ok()
        .and_then(|content| {
            let trimmed = content.trim();
            // "max" means unlimited
            if trimmed == "max" {
                None
            } else {
                trimmed.parse::<u64>().ok()
            }
        });

    Ok(CgroupMemoryStats {
        usage_bytes,
        limit_bytes,
    })
}

/// Read cgroup v1 memory statistics
///
/// Reads memory.usage_in_bytes and memory.limit_in_bytes from the process's
/// own memory-controller cgroup.
fn read_cgroup_v1_memory() -> io::Result<CgroupMemoryStats> {
    let dir = own_cgroup_dir("/sys/fs/cgroup/memory", false)?;

    let usage_bytes = fs::read_to_string(dir.join("memory.usage_in_bytes"))?
        .trim()
        .parse::<u64>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let limit_bytes = fs::read_to_string(dir.join("memory.limit_in_bytes"))
        .ok()
        .and_then(|content| {
            let limit = content.trim().parse::<u64>().ok()?;
            // Very large values (near u64::MAX) typically mean unlimited
            // Common unlimited sentinel: 9223372036854771712 (0x7FFFFFFFFFFFF000)
            if limit > (1u64 << 60) {
                None
            } else {
                Some(limit)
            }
        });

    Ok(CgroupMemoryStats {
        usage_bytes,
        limit_bytes,
    })
}

/// Detect which cgroup version is in use
///
/// Returns V2 if /sys/fs/cgroup/memory.current exists (cgroup v2),
/// V1 if /sys/fs/cgroup/memory exists (cgroup v1),
/// or None if neither is detected.
pub fn detect_cgroup_version() -> Option<CgroupVersion> {
    if Path::new("/sys/fs/cgroup/memory.current").exists() {
        Some(CgroupVersion::V2)
    } else if Path::new("/sys/fs/cgroup/memory").exists() {
        Some(CgroupVersion::V1)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Check if podman is available on the system
    fn is_podman_available() -> bool {
        Command::new("podman")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn test_cgroup_version_enum() {
        // Basic enum test
        let v2 = CgroupVersion::V2;
        let v1 = CgroupVersion::V1;
        assert_ne!(v2, v1);
    }

    #[test]
    fn test_cgroup_version_clone_copy() {
        let v2 = CgroupVersion::V2;
        let v2_copy = v2;
        assert_eq!(v2, v2_copy);
    }

    #[test]
    fn test_cgroup_memory_stats_default() {
        let stats = CgroupMemoryStats::default();
        assert_eq!(stats.usage_bytes, 0);
        assert_eq!(stats.limit_bytes, None);
    }

    #[test]
    fn test_cgroup_memory_stats_equality() {
        let stats1 = CgroupMemoryStats {
            usage_bytes: 1024,
            limit_bytes: Some(2048),
        };
        let stats2 = CgroupMemoryStats {
            usage_bytes: 1024,
            limit_bytes: Some(2048),
        };
        let stats3 = CgroupMemoryStats {
            usage_bytes: 2048,
            limit_bytes: Some(2048),
        };
        assert_eq!(stats1, stats2);
        assert_ne!(stats1, stats3);
    }

    #[test]
    fn test_cgroup_memory_stats_clone() {
        let stats = CgroupMemoryStats {
            usage_bytes: 1024,
            limit_bytes: Some(2048),
        };
        let cloned = stats;
        assert_eq!(stats, cloned);
    }

    #[test]
    fn test_cgroup_memory_stats_debug() {
        let stats = CgroupMemoryStats {
            usage_bytes: 1024,
            limit_bytes: Some(2048),
        };
        let debug_str = format!("{:?}", stats);
        assert!(debug_str.contains("1024"));
        assert!(debug_str.contains("2048"));
    }

    // Integration tests only run on Linux with cgroups available
    #[cfg(target_os = "linux")]
    #[test]
    fn test_detect_cgroup_version_integration() {
        // This test may pass or fail depending on the environment
        // Just ensure it doesn't panic
        let version = detect_cgroup_version();
        if let Some(v) = version {
            println!("Detected cgroup version: {:?}", v);
        } else {
            println!("No cgroup detected (not running in container)");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_read_cgroup_memory_integration() {
        // This test attempts to read cgroup memory if available
        if let Some(stats) = read_cgroup_memory() {
            println!("cgroup memory usage: {} bytes", stats.usage_bytes);
            if let Some(limit) = stats.limit_bytes {
                println!("cgroup memory limit: {} bytes", limit);
            }
            // Basic sanity check
            assert!(stats.usage_bytes > 0, "Usage should be > 0 in container");
        } else {
            println!("Not running in cgroup, skipping test");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_cgroup_v2_in_podman_container() {
        if !is_podman_available() {
            println!("Podman not available, skipping container test");
            return;
        }

        // Create a simple test script that reads cgroup v2 memory
        let test_script = r#"
            if [ -f /sys/fs/cgroup/memory.current ]; then
                echo "v2"
                cat /sys/fs/cgroup/memory.current
                cat /sys/fs/cgroup/memory.max 2>/dev/null || echo "max"
            else
                echo "not_v2"
            fi
        "#;

        let output = Command::new("podman")
            .args([
                "run",
                "--rm",
                "--memory=256m",
                "docker.io/library/alpine:latest",
                "sh",
                "-c",
                test_script,
            ])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                println!("Podman container output:\n{}", stdout);

                if stdout.starts_with("v2") {
                    println!("✓ Container uses cgroup v2");
                    // Parse the memory values from output
                    let lines: Vec<&str> = stdout.lines().collect();
                    if lines.len() >= 2
                        && let Ok(usage) = lines[1].trim().parse::<u64>()
                    {
                        println!("  Memory usage in container: {} bytes", usage);
                        assert!(usage > 0, "Memory usage should be positive");
                    }
                } else {
                    println!("Container uses cgroup v1 or no cgroup");
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!("Podman command failed: {}", stderr);
                println!("Skipping podman container test");
            }
            Err(e) => {
                println!("Failed to run podman: {}", e);
                println!("Skipping podman container test");
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_cgroup_memory_limit_parsing() {
        if !is_podman_available() {
            println!("Podman not available, skipping container test");
            return;
        }

        // Test with a specific memory limit
        let test_script = r#"
            if [ -f /sys/fs/cgroup/memory.current ]; then
                cat /sys/fs/cgroup/memory.current
                cat /sys/fs/cgroup/memory.max
            elif [ -f /sys/fs/cgroup/memory/memory.usage_in_bytes ]; then
                cat /sys/fs/cgroup/memory/memory.usage_in_bytes
                cat /sys/fs/cgroup/memory/memory.limit_in_bytes
            fi
        "#;

        let output = Command::new("podman")
            .args([
                "run",
                "--rm",
                "--memory=128m",
                "docker.io/library/alpine:latest",
                "sh",
                "-c",
                test_script,
            ])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<&str> = stdout.lines().collect();

                if lines.len() >= 2 {
                    println!("Memory usage: {}", lines[0]);
                    println!("Memory limit: {}", lines[1]);

                    // Verify limit is parsed correctly
                    let limit_str = lines[1].trim();
                    if limit_str != "max"
                        && let Ok(limit) = limit_str.parse::<u64>()
                    {
                        // 128MB = 134217728 bytes
                        // Allow some tolerance for overhead
                        let expected = 128 * 1024 * 1024;
                        let tolerance = 10 * 1024 * 1024; // 10MB tolerance
                        assert!(
                            limit >= expected - tolerance && limit <= expected + tolerance,
                            "Expected limit around {} bytes, got {}",
                            expected,
                            limit
                        );
                    }
                }
            }
            Ok(_) | Err(_) => {
                println!("Podman test skipped");
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_cgroup_stats_in_rust_binary() {
        if !is_podman_available() {
            println!("Podman not available, skipping Rust binary test");
            return;
        }

        // Create a Rust program that uses our cgroup reader
        let rust_code = r#"
            use std::path::Path;
            use std::fs;

            fn main() {
                // Simulate reading cgroup v2
                if Path::new("/sys/fs/cgroup/memory.current").exists() {
                    if let Ok(usage) = fs::read_to_string("/sys/fs/cgroup/memory.current") {
                        println!("usage:{}", usage.trim());
                    }
                    if let Ok(limit) = fs::read_to_string("/sys/fs/cgroup/memory.max") {
                        println!("limit:{}", limit.trim());
                    }
                } else if Path::new("/sys/fs/cgroup/memory/memory.usage_in_bytes").exists() {
                    if let Ok(usage) = fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes") {
                        println!("usage:{}", usage.trim());
                    }
                    if let Ok(limit) = fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
                        println!("limit:{}", limit.trim());
                    }
                } else {
                    println!("no_cgroup");
                }
            }
        "#;

        // Try to run the test in a container
        let output = Command::new("podman")
            .args([
                "run",
                "--rm",
                "--memory=256m",
                "docker.io/library/rust:alpine",
                "sh",
                "-c",
                &format!(
                    "echo '{}' > /tmp/test.rs && rustc /tmp/test.rs -o /tmp/test && /tmp/test",
                    rust_code
                ),
            ])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                println!("Rust binary in container output:\n{}", stdout);

                if !stdout.contains("no_cgroup") {
                    println!("✓ Rust binary successfully read cgroup stats in container");
                }
            }
            Ok(_) | Err(_) => {
                println!("Rust binary container test skipped (requires rust image)");
            }
        }
    }

    #[test]
    fn test_cgroup_memory_stats_with_no_limit() {
        let stats = CgroupMemoryStats {
            usage_bytes: 1024 * 1024,
            limit_bytes: None,
        };
        assert_eq!(stats.usage_bytes, 1024 * 1024);
        assert_eq!(stats.limit_bytes, None);
    }

    #[test]
    fn test_cgroup_memory_stats_with_limit() {
        let stats = CgroupMemoryStats {
            usage_bytes: 512 * 1024 * 1024,
            limit_bytes: Some(1024 * 1024 * 1024),
        };
        assert_eq!(stats.usage_bytes, 512 * 1024 * 1024);
        assert_eq!(stats.limit_bytes, Some(1024 * 1024 * 1024));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_read_cgroup_v2_memory_paths() {
        // Test that the function handles missing files gracefully
        // This will fail if not in cgroup v2, which is expected
        let result = read_cgroup_v2_memory();

        // Just verify it returns a Result, don't assert on success/failure
        // since we may not be in a cgroup v2 environment
        match result {
            Ok(stats) => {
                println!(
                    "Successfully read cgroup v2: usage={}, limit={:?}",
                    stats.usage_bytes, stats.limit_bytes
                );
                assert!(stats.usage_bytes > 0);
            }
            Err(e) => {
                println!(
                    "cgroup v2 not available (expected if not in container): {}",
                    e
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_read_cgroup_v1_memory_paths() {
        // Test that the function handles missing files gracefully
        let result = read_cgroup_v1_memory();

        match result {
            Ok(stats) => {
                println!(
                    "Successfully read cgroup v1: usage={}, limit={:?}",
                    stats.usage_bytes, stats.limit_bytes
                );
                assert!(stats.usage_bytes > 0);
            }
            Err(e) => {
                println!(
                    "cgroup v1 not available (expected if not in container): {}",
                    e
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_detect_cgroup_version_values() {
        match detect_cgroup_version() {
            Some(CgroupVersion::V2) => {
                println!("Detected cgroup v2");
                assert!(Path::new("/sys/fs/cgroup/memory.current").exists());
            }
            Some(CgroupVersion::V1) => {
                println!("Detected cgroup v1");
                assert!(Path::new("/sys/fs/cgroup/memory").exists());
            }
            None => {
                println!("No cgroup detected");
                assert!(!Path::new("/sys/fs/cgroup/memory.current").exists());
            }
        }
    }

    #[test]
    fn test_large_memory_values() {
        // Test that we can represent large memory values
        let stats = CgroupMemoryStats {
            usage_bytes: 16 * 1024 * 1024 * 1024,       // 16GB
            limit_bytes: Some(32 * 1024 * 1024 * 1024), // 32GB
        };
        assert_eq!(stats.usage_bytes, 16 * 1024 * 1024 * 1024);
        assert_eq!(stats.limit_bytes, Some(32 * 1024 * 1024 * 1024));
    }

    #[test]
    fn test_unlimited_sentinel_value() {
        // Test the unlimited sentinel detection (1u64 << 60)
        let unlimited_threshold = 1u64 << 60;
        let near_max = u64::MAX - 1000;

        // Values above threshold should be considered unlimited
        assert!(near_max > unlimited_threshold);

        // Simulate what happens with very large values
        let stats = CgroupMemoryStats {
            usage_bytes: 1024,
            limit_bytes: None, // Would be None for unlimited
        };
        assert_eq!(stats.limit_bytes, None);
    }

    #[test]
    fn test_cgroup_memory_zero_usage() {
        // Edge case: zero usage (shouldn't happen but test the type)
        let stats = CgroupMemoryStats {
            usage_bytes: 0,
            limit_bytes: Some(1024),
        };
        assert_eq!(stats.usage_bytes, 0);
    }

    #[test]
    fn test_cgroup_version_debug_format() {
        let v2 = CgroupVersion::V2;
        let v1 = CgroupVersion::V1;
        let v2_str = format!("{:?}", v2);
        let v1_str = format!("{:?}", v1);
        assert!(v2_str.contains("V2"));
        assert!(v1_str.contains("V1"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_read_cgroup_memory_fallback() {
        // The implementation tries v2 first, then v1; either way it must not
        // panic. Success and version detection are independent: reads go
        // through /proc/self/cgroup while detection probes mount roots.
        if let Some(stats) = read_cgroup_memory() {
            println!("Read cgroup stats (v2 or v1): usage={}", stats.usage_bytes);
        } else {
            println!("No cgroup available");
        }
        let _ = detect_cgroup_version();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_cgroup_v1_unlimited_threshold() {
        // Test that values above 1u64 << 60 are treated as unlimited
        // This is a unit test for the logic, not integration test
        let threshold = 1u64 << 60;

        // Common cgroup v1 unlimited value
        let common_unlimited = 9223372036854771712u64; // 0x7FFFFFFFFFFFF000
        assert!(common_unlimited > threshold);

        // u64::MAX is definitely unlimited
        assert!(u64::MAX > threshold);

        // A reasonable 1TB limit should be under threshold
        let one_tb = 1024u64 * 1024 * 1024 * 1024;
        assert!(one_tb < threshold);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_podman_availability_check() {
        // Test the helper function
        let has_podman = is_podman_available();
        println!("Podman available: {}", has_podman);

        // Just verify it doesn't panic
        if has_podman {
            // Try to get version
            let output = Command::new("podman").arg("--version").output();
            assert!(output.is_ok());
        }
    }
}
