//! Memory stats reader for process memory metrics
//!
//! Reads memory information from /proc/self/status on Linux systems
//! and from mach APIs on macOS systems.

use std::io;

#[cfg(target_os = "linux")]
use std::fs;

/// Memory statistics from process
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcMemoryStats {
    /// Resident Set Size (RSS) in bytes - physical memory used
    pub rss_bytes: u64,
    /// Virtual Memory Size (VmSize) in bytes - total virtual memory
    pub vms_bytes: u64,
}

/// Read memory statistics
///
/// On Linux: Parses VmRSS and VmSize fields from /proc/self/status.
/// On macOS: Uses mach APIs to get RSS and VMS.
/// Returns Ok(ProcMemoryStats) on success, or an error if reading fails.
///
/// # Example
///
/// ```no_run
/// # use lazydns::metrics::memory::proc_reader::read_proc_memory;
/// let stats = read_proc_memory().expect("Failed to read memory stats");
/// println!("RSS: {} bytes", stats.rss_bytes);
/// println!("VMS: {} bytes", stats.vms_bytes);
/// ```
pub fn read_proc_memory() -> io::Result<ProcMemoryStats> {
    #[cfg(target_os = "linux")]
    {
        let content = fs::read_to_string("/proc/self/status")?;
        parse_proc_status(&content)
    }

    #[cfg(target_os = "macos")]
    {
        read_macos_memory()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Memory stats reading is not supported on this platform",
        ))
    }
}

#[cfg(target_os = "macos")]
fn read_macos_memory() -> io::Result<ProcMemoryStats> {
    use std::mem;

    // Using libc directly for task_info calls, but through safer wrappers where possible
    #[allow(deprecated)]
    unsafe {
        let mut info: libc::mach_task_basic_info = mem::zeroed();
        let mut info_count: u32 =
            (mem::size_of::<libc::mach_task_basic_info>() / mem::size_of::<u32>()) as u32;

        #[allow(non_upper_case_globals)]
        const TASK_BASIC_INFO: u32 = 4;

        // Call the libc task_info API (uses deprecated mach_task_self() but stable)
        let status = libc::task_info(
            libc::mach_task_self(),
            TASK_BASIC_INFO,
            &mut info as *mut _ as *mut i32,
            &mut info_count,
        );

        if status != libc::KERN_SUCCESS {
            return Err(io::Error::other(format!(
                "Failed to call task_info: status {}",
                status
            )));
        }

        Ok(ProcMemoryStats {
            rss_bytes: info.resident_size,
            vms_bytes: info.virtual_size,
        })
    }
}

/// Parse /proc/self/status content (Linux only)
///
/// Extracts VmRSS and VmSize from the status file content.
/// Both values are expected to be in kB and are converted to bytes.
#[cfg(target_os = "linux")]
fn parse_proc_status(content: &str) -> io::Result<ProcMemoryStats> {
    let mut stats = ProcMemoryStats::default();

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            stats.rss_bytes = parse_memory_kb(value)?;
        } else if let Some(value) = line.strip_prefix("VmSize:") {
            stats.vms_bytes = parse_memory_kb(value)?;
        }
    }

    Ok(stats)
}

/// Parse memory value in kB format (such as "  12345 kB") (Linux only)
///
/// Extracts the numeric value and converts to bytes (multiply by 1024).
#[cfg(target_os = "linux")]
fn parse_memory_kb(value: &str) -> io::Result<u64> {
    // Trim whitespace and "kB" suffix
    let trimmed = value.trim().trim_end_matches("kB").trim();

    trimmed.parse::<u64>().map(|kb| kb * 1024).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse memory value '{}': {}", trimmed, e),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_memory_kb() {
        assert_eq!(parse_memory_kb("  12345 kB").unwrap(), 12345 * 1024);
        assert_eq!(parse_memory_kb("100 kB").unwrap(), 100 * 1024);
        assert_eq!(parse_memory_kb("0 kB").unwrap(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_memory_kb_invalid() {
        assert!(parse_memory_kb("invalid").is_err());
        assert!(parse_memory_kb("").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_proc_status() {
        let content = r#"
Name:	lazydns
VmSize:	   524288 kB
VmRSS:	    102400 kB
VmData:	    51200 kB
"#;
        let stats = parse_proc_status(content).unwrap();
        assert_eq!(stats.vms_bytes, 524288 * 1024);
        assert_eq!(stats.rss_bytes, 102400 * 1024);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_proc_status_partial() {
        let content = "VmRSS:	    10240 kB\n";
        let stats = parse_proc_status(content).unwrap();
        assert_eq!(stats.rss_bytes, 10240 * 1024);
        assert_eq!(stats.vms_bytes, 0); // Not present in input
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_proc_status_empty() {
        let stats = parse_proc_status("").unwrap();
        assert_eq!(stats.rss_bytes, 0);
        assert_eq!(stats.vms_bytes, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_read_proc_memory_integration() {
        // Integration test that actually reads /proc/self/status
        let result = read_proc_memory();
        assert!(
            result.is_ok(),
            "Failed to read /proc/self/status: {:?}",
            result
        );

        let stats = result.unwrap();
        // Sanity checks: RSS and VMS should be non-zero and RSS <= VMS
        assert!(stats.rss_bytes > 0, "RSS should be > 0");
        assert!(stats.vms_bytes > 0, "VMS should be > 0");
        assert!(stats.rss_bytes <= stats.vms_bytes, "RSS should be <= VMS");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_read_macos_memory_integration() {
        // Integration test that actually reads memory stats on macOS
        let result = read_proc_memory();
        assert!(
            result.is_ok(),
            "Failed to read macOS memory stats: {:?}",
            result
        );

        let stats = result.unwrap();
        // Sanity checks: RSS and VMS should be non-zero and RSS <= VMS
        assert!(stats.rss_bytes > 0, "RSS should be > 0");
        assert!(stats.vms_bytes > 0, "VMS should be > 0");
        assert!(stats.rss_bytes <= stats.vms_bytes, "RSS should be <= VMS");
    }
}
