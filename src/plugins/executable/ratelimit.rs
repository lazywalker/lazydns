//! Rate limiting plugin for DNS queries
//!
//! Implements rate limiting to prevent DoS attacks and resource exhaustion.

use crate::config::PluginConfig;
use crate::dns::ResponseCode;
use crate::plugin::{BackgroundTask, Context, Plugin, RETURN_FLAG};
use crate::{RegisterPlugin, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use std::any::Any;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

// Auto-register plugin builder

/// Rate limiter entry tracking request history
#[derive(Debug)]
struct RateLimitEntry {
    /// Number of requests in current window
    count: u32,
    /// Window start time
    window_start: Instant,
}

/// Rate limiting plugin
///
/// Limits the number of queries from a single IP address within a time window.
///
/// # YAML configuration example
///
/// ```yaml
/// # plugins: (top-level list of plugins)
/// - name: my_rate_limiter
///   type: rate_limit
///   args:
///     max_queries: 100     # max queries per window per IP (integer)
///     window_secs: 60      # window length in seconds (integer)
/// ```
///
/// # Rust example
///
/// ```rust
/// use lazydns::plugins::RateLimitPlugin;
/// use lazydns::plugin::Plugin;
///
/// // Allow 100 queries per 60 seconds per IP
/// let rate_limiter = RateLimitPlugin::new(100, 60);
/// assert_eq!(rate_limiter.name(), "rate_limit");
/// ```
#[derive(Debug, Clone, RegisterPlugin)]
pub struct RateLimitPlugin {
    /// Maximum queries per window
    max_queries: u32,
    /// Time window in seconds
    window_secs: u64,
    /// Storage for rate limit tracking
    limits: Arc<DashMap<IpAddr, RateLimitEntry>>,
}

impl RateLimitPlugin {
    /// Create a new rate limit plugin
    ///
    /// # Arguments
    ///
    /// * `max_queries` - Maximum number of queries allowed per window
    /// * `window_secs` - Time window in seconds
    pub fn new(max_queries: u32, window_secs: u64) -> Self {
        Self {
            max_queries,
            window_secs,
            limits: Arc::new(DashMap::new()),
        }
    }

    /// Check if a request should be rate limited
    fn should_limit(&self, client_ip: IpAddr) -> bool {
        let now = Instant::now();
        let window_duration = Duration::from_secs(self.window_secs);

        let mut should_limit = false;

        self.limits
            .entry(client_ip)
            .and_modify(|entry| {
                // Check if we need to reset the window
                if now.duration_since(entry.window_start) >= window_duration {
                    entry.count = 1;
                    entry.window_start = now;
                } else {
                    entry.count += 1;
                    if entry.count > self.max_queries {
                        should_limit = true;
                    }
                }
            })
            .or_insert(RateLimitEntry {
                count: 1,
                window_start: now,
            });

        should_limit
    }

    /// Clean up old entries
    pub fn cleanup(&self) {
        let now = Instant::now();
        let window_duration = Duration::from_secs(self.window_secs);

        self.limits
            .retain(|_, entry| now.duration_since(entry.window_start) < window_duration * 2);
    }

    /// Spawn a background cleanup task that periodically removes expired entries
    ///
    /// This prevents the DashMap from growing unboundedly when there are many
    /// unique client IPs. The cleanup runs every `window_secs * 2` seconds.
    ///
    /// # Returns
    ///
    /// A JoinHandle that can be used to abort the cleanup task if needed.
    pub fn spawn_cleanup_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let cleanup_interval = Duration::from_secs(self.window_secs.max(30) * 2);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(cleanup_interval);
            loop {
                interval.tick().await;
                let before = self.limits.len();
                self.cleanup();
                let after = self.limits.len();
                if before > after {
                    debug!(
                        "RateLimitPlugin cleanup: removed {} entries ({} -> {})",
                        before - after,
                        before,
                        after
                    );
                }
            }
        })
    }
}

impl BackgroundTask for RateLimitPlugin {
    fn background_task_interval(&self) -> Duration {
        Duration::from_secs(self.window_secs.max(30) * 2)
    }

    fn run_background_task(&self) {
        let before = self.limits.len();
        self.cleanup();
        let after = self.limits.len();
        if before > after {
            debug!(
                "RateLimitPlugin cleanup: removed {} entries ({} -> {})",
                before - after,
                before,
                after
            );
        }
    }

    fn background_task_name(&self) -> &str {
        "ratelimit_cleanup"
    }
}

#[async_trait]
impl Plugin for RateLimitPlugin {
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        // Try to get client IP from metadata (set by server)
        let client_ip: IpAddr = match ctx.get_metadata::<IpAddr>("client_ip") {
            Some(ip) => *ip,
            None => {
                // Skip rate limiting if no client IP available
                return Ok(());
            }
        };

        if self.should_limit(client_ip) {
            warn!("Rate limit exceeded for IP: {}", client_ip);

            #[cfg(feature = "web")]
            // Log security event
            crate::plugins::AUDIT_LOGGER
                .log_security_event(
                    crate::plugins::SecurityEventType::RateLimitExceeded,
                    format!("Rate limit exceeded for client {}", client_ip),
                    Some(client_ip),
                    None,
                )
                .await;

            // Create a REFUSED response. Echo the request's question section and
            // id so the client gets a well-formed reply, and set RETURN_FLAG so
            // downstream plugins (ttl/forward/...) cannot overwrite the refusal.
            let mut response = crate::dns::Message::new();
            response.set_id(ctx.request().id());
            response.set_response(true);
            response.set_response_code(ResponseCode::Refused);
            let request_questions = ctx.request().questions().to_vec();
            *response.questions_mut() = request_questions;

            ctx.set_response(Some(response));
            ctx.set_metadata(RETURN_FLAG, true);
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "rate_limit"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn spawn_background_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        Some(Arc::new(self.clone()).spawn_background_task())
    }

    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>> {
        use serde_yaml::Value;
        use std::sync::Arc;

        // defaults
        let mut max_queries: u32 = 100;
        let mut window_secs: u64 = 60;

        let args = config.effective_args();
        if let Some(v) = args.get("max_queries") {
            match v {
                Value::Number(n) => {
                    if let Some(u) = n.as_u64() {
                        max_queries = u as u32;
                    } else if let Some(i) = n.as_i64() {
                        max_queries = i as u32;
                    } else {
                        return Err(crate::Error::Config(
                            "Invalid 'max_queries' value".to_string(),
                        ));
                    }
                }
                Value::String(s) => {
                    max_queries = s.parse::<u32>().map_err(|_| {
                        crate::Error::Config("Invalid 'max_queries' string value".to_string())
                    })?;
                }
                _ => {
                    return Err(crate::Error::Config(
                        "Invalid 'max_queries' type".to_string(),
                    ));
                }
            }
        }

        if let Some(v) = args.get("window_secs") {
            match v {
                Value::Number(n) => {
                    if let Some(u) = n.as_u64() {
                        window_secs = u;
                    } else if let Some(i) = n.as_i64() {
                        window_secs = i as u64;
                    } else {
                        return Err(crate::Error::Config(
                            "Invalid 'window_secs' value".to_string(),
                        ));
                    }
                }
                Value::String(s) => {
                    window_secs = s.parse::<u64>().map_err(|_| {
                        crate::Error::Config("Invalid 'window_secs' string value".to_string())
                    })?;
                }
                _ => {
                    return Err(crate::Error::Config(
                        "Invalid 'window_secs' type".to_string(),
                    ));
                }
            }
        }

        Ok(Arc::new(RateLimitPlugin::new(max_queries, window_secs)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;

    #[test]
    fn test_rate_limit_creation() {
        let limiter = RateLimitPlugin::new(10, 60);
        assert_eq!(limiter.max_queries, 10);
        assert_eq!(limiter.window_secs, 60);
    }

    #[test]
    fn test_rate_limit_allows_within_limit() {
        let limiter = RateLimitPlugin::new(5, 60);
        let ip: IpAddr = "192.168.1.1".parse().unwrap();

        for _ in 0..5 {
            assert!(!limiter.should_limit(ip));
        }
    }

    #[test]
    fn test_rate_limit_blocks_over_limit() {
        let limiter = RateLimitPlugin::new(3, 60);
        let ip: IpAddr = "192.168.1.2".parse().unwrap();

        // First 3 should pass
        for _ in 0..3 {
            assert!(!limiter.should_limit(ip));
        }

        // 4th should be blocked
        assert!(limiter.should_limit(ip));
    }

    #[test]
    fn test_rate_limit_different_ips() {
        let limiter = RateLimitPlugin::new(2, 60);
        let ip1: IpAddr = "192.168.1.1".parse().unwrap();
        let ip2: IpAddr = "192.168.1.2".parse().unwrap();

        // Each IP should have independent limits
        assert!(!limiter.should_limit(ip1));
        assert!(!limiter.should_limit(ip2));
        assert!(!limiter.should_limit(ip1));
        assert!(!limiter.should_limit(ip2));

        // Both should now be at limit
        assert!(limiter.should_limit(ip1));
        assert!(limiter.should_limit(ip2));
    }

    #[test]
    fn test_cleanup() {
        let limiter = RateLimitPlugin::new(10, 1);
        let ip: IpAddr = "192.168.1.3".parse().unwrap();

        limiter.should_limit(ip);
        assert_eq!(limiter.limits.len(), 1);

        limiter.cleanup();
        assert_eq!(limiter.limits.len(), 1); // Still there

        std::thread::sleep(std::time::Duration::from_secs(3));
        limiter.cleanup();
        assert_eq!(limiter.limits.len(), 0); // Cleaned up
    }

    #[tokio::test]
    async fn test_rate_limit_plugin_execute() {
        let limiter = RateLimitPlugin::new(2, 60);
        let mut ctx = Context::new(Message::new());
        ctx.set_metadata("client_ip", "192.168.1.1".parse::<IpAddr>().unwrap());

        // First two should pass
        limiter.execute(&mut ctx).await.unwrap();
        assert!(ctx.response().is_none());

        limiter.execute(&mut ctx).await.unwrap();
        assert!(ctx.response().is_none());

        // Third should be rate limited
        limiter.execute(&mut ctx).await.unwrap();
        assert!(ctx.response().is_some());
        assert_eq!(
            ctx.response().unwrap().response_code(),
            ResponseCode::Refused
        );
    }
}

#[cfg(test)]
mod builder_init_tests {
    use super::*;
    use serde_yaml::Mapping;
    use serde_yaml::Value;

    #[test]
    fn test_init_from_config() {
        let mut args_map = Mapping::new();
        args_map.insert(
            Value::String("max_queries".to_string()),
            Value::Number(100.into()),
        );
        args_map.insert(
            Value::String("window_secs".to_string()),
            Value::Number(60.into()),
        );

        let cfg = crate::config::types::PluginConfig {
            tag: None,
            plugin_type: "rate_limit".to_string(),
            args: Value::Mapping(args_map),
        };

        let plugin = RateLimitPlugin::init(&cfg).expect("init");
        assert_eq!(plugin.name(), "rate_limit");
        if let Some(rl) = plugin.as_ref().as_any().downcast_ref::<RateLimitPlugin>() {
            assert_eq!(rl.max_queries, 100);
            assert_eq!(rl.window_secs, 60);
        } else {
            panic!("unexpected plugin type");
        }
    }
}
