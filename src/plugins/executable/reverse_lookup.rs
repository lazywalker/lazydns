use crate::config::PluginConfig;
use crate::dns::{Message, RData, ResourceRecord};
use crate::plugin::{BackgroundTask, Context, Plugin};
use crate::{RegisterPlugin, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use serde_yaml::Value;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Configuration arguments for `ReverseLookupPlugin`.
///
/// - `handle_ptr`: whether the plugin should attempt to answer PTR queries
///   directly from the cache.
/// - `ttl`: maximum TTL (in seconds) to honor when storing answers from
///   responses; saved entries will use the minimum of record TTL and this
///   configured value.
#[derive(Debug, Clone)]
pub struct ReverseLookupArgs {
    pub handle_ptr: bool,
    pub ttl: u32,
}

impl Default for ReverseLookupArgs {
    fn default() -> Self {
        Self {
            handle_ptr: true,
            ttl: 7200,
        }
    }
}

/// Cache entry stored in the reverse lookup table: `(fqdn, expiration)`.
type Entry = (String, Instant);

/// Reverse lookup plugin.
///
/// The plugin collects A/AAAA answers from responses (via `save_ips_after`)
/// and keeps a small in-memory mapping from IP -> owner name. When a PTR
/// query is received and `handle_ptr` is enabled, the plugin will attempt to
/// translate the reverse name into an IP and reply from the cache if a valid
/// (non-expired) entry exists.
#[derive(Clone, RegisterPlugin)]
pub struct ReverseLookupPlugin {
    cache: Arc<DashMap<String, Entry>>,
    args: ReverseLookupArgs,
}

// Auto-register plugin builder for config-based construction

impl ReverseLookupPlugin {
    pub fn new(args: ReverseLookupArgs) -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            args,
        }
    }

    /// Parse a PTR-style qname into an `IpAddr`.
    ///
    /// Supports IPv4 reverse names under `in-addr.arpa` and IPv6 nibble
    /// reverse names under `ip6.arpa`. Returns `None` if parsing fails.
    ///
    /// This is a helper used by `execute` to detect the query target.
    fn parse_ptr(qname: &str) -> Option<IpAddr> {
        let s = qname.trim_end_matches('.').to_ascii_lowercase();
        if s.ends_with(".in-addr.arpa") {
            let without = s.trim_end_matches(".in-addr.arpa").trim_end_matches('.');
            let parts: Vec<&str> = without.split('.').collect();
            if parts.len() == 4 {
                let rev: Vec<&str> = parts.into_iter().rev().collect();
                let ip = rev.join(".");
                return ip.parse::<IpAddr>().ok();
            }
        } else if s.ends_with(".ip6.arpa") {
            // attempt to parse nibble format: labels are nibbles in reverse order
            let without = s.trim_end_matches(".ip6.arpa").trim_end_matches('.');
            let labels: Vec<&str> = without.split('.').collect();
            let mut hex = String::new();
            for label in labels.into_iter().rev() {
                if label.len() != 1 {
                    return None;
                }
                hex.push_str(label);
            }
            // group into bytes
            if !hex.len().is_multiple_of(2) {
                hex.push('0');
            }
            let bytes_res: std::result::Result<Vec<u8>, std::num::ParseIntError> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect();
            if let Ok(bytes) = bytes_res
                && bytes.len() == 16
            {
                use std::net::Ipv6Addr;
                let mut arr = [0u8; 16];
                arr.copy_from_slice(&bytes);
                return Some(IpAddr::V6(Ipv6Addr::from(arr)));
            }
        }
        None
    }

    fn lookup(&self, ip: &IpAddr) -> Option<String> {
        let key = ip.to_string();
        if let Some(v) = self.cache.get(&key) {
            if v.value().1 > Instant::now() {
                return Some(v.value().0.clone());
            } else {
                // expired
                self.cache.remove(&key);
            }
        }
        None
    }

    fn save_from_response(&self, req: &Message, resp: &Message) {
        if resp.answer_count() == 0 {
            return;
        }
        let now = Instant::now();
        for rr in resp.answers() {
            match rr.rdata() {
                RData::A(ipv4) => {
                    let ip = IpAddr::V4(*ipv4);
                    let ttl = rr.ttl().min(self.args.ttl);
                    let exp = now + Duration::from_secs(ttl as u64);
                    let name = if req.question_count() == 1 {
                        req.questions()[0].qname().to_string()
                    } else {
                        rr.name().to_string()
                    };
                    self.cache.insert(ip.to_string(), (name, exp));
                }
                RData::AAAA(ipv6) => {
                    let ip = IpAddr::V6(*ipv6);
                    let ttl = rr.ttl().min(self.args.ttl);
                    let exp = now + Duration::from_secs(ttl as u64);
                    let name = if req.question_count() == 1 {
                        req.questions()[0].qname().to_string()
                    } else {
                        rr.name().to_string()
                    };
                    self.cache.insert(ip.to_string(), (name, exp));
                }
                _ => {}
            }
        }
    }
}

/// Public API and helpers for `ReverseLookupPlugin`.
impl ReverseLookupPlugin {
    /// Clean up expired entries from the cache
    ///
    /// This removes all entries whose expiration time has passed.
    /// Returns the number of entries removed.
    pub fn cleanup(&self) -> usize {
        let now = Instant::now();
        let before = self.cache.len();
        self.cache.retain(|_, entry| entry.1 > now);
        let after = self.cache.len();
        before.saturating_sub(after)
    }

    /// Get the current cache size
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

impl BackgroundTask for ReverseLookupPlugin {
    fn background_task_interval(&self) -> Duration {
        Duration::from_secs(300) // 5 minutes
    }

    fn run_background_task(&self) {
        let removed = self.cleanup();
        if removed > 0 {
            tracing::debug!(
                "ReverseLookupPlugin cleanup: removed {} expired entries",
                removed
            );
        }
    }

    fn background_task_name(&self) -> &str {
        "reverse_lookup_cleanup"
    }
}

impl fmt::Debug for ReverseLookupPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReverseLookup").finish()
    }
}

#[async_trait]
impl Plugin for ReverseLookupPlugin {
    fn name(&self) -> &str {
        "reverse_lookup"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn spawn_background_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        Some(Arc::new(self.clone()).spawn_background_task())
    }

    fn init(config: &PluginConfig) -> crate::Result<std::sync::Arc<dyn crate::plugin::Plugin>> {
        // Parse args with sensible defaults
        let args_map = config.effective_args();

        let mut args = ReverseLookupArgs::default();

        if let Some(Value::Bool(b)) = args_map.get("handle_ptr") {
            args.handle_ptr = *b;
        }

        if let Some(v) = args_map.get("ttl") {
            match v {
                Value::Number(n) => {
                    if let Some(u) = n.as_u64() {
                        args.ttl = u as u32;
                    }
                }
                Value::String(s) => {
                    if let Ok(u) = s.parse::<u32>() {
                        args.ttl = u;
                    }
                }
                _ => {}
            }
        }

        Ok(std::sync::Arc::new(ReverseLookupPlugin::new(args)))
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        // If PTR and configured, attempt to reply from cache
        let req = ctx.request();
        if self.args.handle_ptr
            && let Some(q) = req.questions().first()
            && q.qtype() == crate::dns::types::RecordType::PTR
            && let Some(ip) = Self::parse_ptr(q.qname())
            && let Some(fqdn) = self.lookup(&ip)
        {
            let mut r = Message::new();
            r.set_id(req.id());
            r.set_response(true);
            r.add_question(q.clone());
            r.add_answer(ResourceRecord::new(
                q.qname(),
                crate::dns::types::RecordType::PTR,
                crate::dns::types::RecordClass::IN,
                5,
                RData::PTR(fqdn),
            ));
            ctx.set_response(Some(r));
            return Ok(());
        }

        // Otherwise do nothing here; saving of IPs is expected to be triggered
        // by sequence runner after response is available via `save_ips_after`.
        Ok(())
    }
}

impl ReverseLookupPlugin {
    /// Helper to be called by sequence runner after response is populated.
    ///
    /// This method extracts A/AAAA answers from `resp` and records mappings
    /// from IP -> owner name in the internal cache. It is intended to be
    /// invoked by higher-level executors after the response is available.
    pub fn save_ips_after(&self, req: &Message, resp: &Message) {
        self.save_from_response(req, resp);
    }

    /// Expose lookup for tests: return the cached fqdn for an IP if present.
    ///
    /// Returns `Some(fqdn)` when the entry exists and has not yet expired.
    pub fn lookup_cached(&self, ip: &IpAddr) -> Option<String> {
        self.lookup(ip)
    }

    pub fn quick_setup(_s: &str) -> Self {
        Self::new(ReverseLookupArgs::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, Question, RData, RecordClass, RecordType, ResourceRecord};
    use std::net::Ipv4Addr;

    #[test]
    fn test_reverse_lookup_args_default() {
        let args = ReverseLookupArgs::default();
        assert!(args.handle_ptr);
        assert_eq!(args.ttl, 7200);
    }

    #[test]
    fn test_reverse_lookup_new() {
        let plugin = ReverseLookupPlugin::new(ReverseLookupArgs::default());
        assert_eq!(plugin.name(), "reverse_lookup");
    }

    #[test]
    fn test_reverse_lookup_debug() {
        let plugin = ReverseLookupPlugin::new(ReverseLookupArgs::default());
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("ReverseLookup"));
    }

    #[test]
    fn test_reverse_lookup_quick_setup_default() {
        let plugin = ReverseLookupPlugin::quick_setup("");
        assert_eq!(plugin.name(), "reverse_lookup");
    }

    #[tokio::test]
    async fn test_parse_ipv4_ptr() {
        let s = "1.2.3.4.in-addr.arpa.";
        let ip = ReverseLookupPlugin::parse_ptr(s).unwrap();
        assert_eq!(ip.to_string(), "4.3.2.1");
    }

    #[test]
    fn test_parse_ipv4_ptr_no_trailing_dot() {
        let s = "1.0.0.127.in-addr.arpa";
        let ip = ReverseLookupPlugin::parse_ptr(s).unwrap();
        assert_eq!(ip.to_string(), "127.0.0.1");
    }

    #[test]
    fn test_parse_ipv4_ptr_invalid() {
        let s = "1.2.3.in-addr.arpa"; // Only 3 octets
        assert!(ReverseLookupPlugin::parse_ptr(s).is_none());
    }

    #[test]
    fn test_parse_ipv6_ptr() {
        // ::1 in nibble format
        let s = "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.ip6.arpa";
        let ip = ReverseLookupPlugin::parse_ptr(s);
        assert!(ip.is_some());
    }

    #[test]
    fn test_parse_unknown_suffix() {
        let s = "example.com";
        assert!(ReverseLookupPlugin::parse_ptr(s).is_none());
    }

    #[test]
    fn test_save_ips_after_and_lookup_cached() {
        // Build request with single question
        let mut req = Message::new();
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        // Build response with one A answer
        let mut resp = Message::new();
        resp.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A(Ipv4Addr::new(1, 2, 3, 4)),
        ));

        let rl = ReverseLookupPlugin::new(ReverseLookupArgs::default());

        // Ensure no entry initially
        let ip = std::net::IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        assert!(rl.lookup_cached(&ip).is_none());

        // Invoke save hook
        rl.save_ips_after(&req, &resp);

        // Now the lookup should return the fqdn from the request
        let got = rl.lookup_cached(&ip).expect("expected cached name");
        assert_eq!(got, "example.com");
    }

    #[test]
    fn test_save_ips_after_ipv6() {
        let mut req = Message::new();
        req.add_question(Question::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
        ));

        let mut resp = Message::new();
        resp.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            RData::AAAA("2001:db8::1".parse().unwrap()),
        ));

        let rl = ReverseLookupPlugin::new(ReverseLookupArgs::default());
        rl.save_ips_after(&req, &resp);

        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        assert!(rl.lookup_cached(&ip).is_some());
    }

    #[tokio::test]
    async fn test_reverse_lookup_answers_ptr() {
        use std::time::Duration;
        use std::time::Instant;

        // Create plugin and pre-fill its cache for IP 10.0.0.1 -> example.com
        let args = ReverseLookupArgs::default();
        let plugin = ReverseLookupPlugin::new(args);

        let key = "10.0.0.1".to_string();
        let entry: (String, Instant) = (
            "example.com".to_string(),
            Instant::now() + Duration::from_secs(60),
        );
        plugin.cache.insert(key.clone(), entry);

        // Build a PTR question for the corresponding reverse name
        let mut msg = Message::new();
        msg.add_question(Question::new(
            "1.0.0.10.in-addr.arpa",
            RecordType::PTR,
            RecordClass::IN,
        ));

        let mut ctx = Context::new(msg);
        plugin.execute(&mut ctx).await.unwrap();

        let resp = ctx.response().unwrap();
        assert_eq!(resp.answer_count(), 1);
        assert_eq!(
            resp.answers()[0].rdata(),
            &RData::PTR("example.com".to_string())
        );
    }

    #[tokio::test]
    async fn test_reverse_lookup_ptr_not_found() {
        let plugin = ReverseLookupPlugin::new(ReverseLookupArgs::default());

        let mut msg = Message::new();
        msg.add_question(Question::new(
            "1.0.0.10.in-addr.arpa",
            RecordType::PTR,
            RecordClass::IN,
        ));

        let mut ctx = Context::new(msg);
        plugin.execute(&mut ctx).await.unwrap();

        // No response should be set when not found in cache
        assert!(ctx.response().is_none());
    }

    #[tokio::test]
    async fn test_reverse_lookup_ptr_disabled() {
        use std::time::Duration;
        use std::time::Instant;

        let args = ReverseLookupArgs {
            handle_ptr: false,
            ..Default::default()
        };
        let plugin = ReverseLookupPlugin::new(args);

        // Pre-fill cache
        plugin.cache.insert(
            "10.0.0.1".to_string(),
            (
                "example.com".to_string(),
                Instant::now() + Duration::from_secs(60),
            ),
        );

        let mut msg = Message::new();
        msg.add_question(Question::new(
            "1.0.0.10.in-addr.arpa",
            RecordType::PTR,
            RecordClass::IN,
        ));

        let mut ctx = Context::new(msg);
        plugin.execute(&mut ctx).await.unwrap();

        // Should not answer when handle_ptr is false
        assert!(ctx.response().is_none());
    }

    #[tokio::test]
    async fn test_reverse_lookup_non_ptr_query() {
        let plugin = ReverseLookupPlugin::new(ReverseLookupArgs::default());

        let mut msg = Message::new();
        msg.add_question(Question::new(
            "example.com",
            RecordType::A, // Not a PTR query
            RecordClass::IN,
        ));

        let mut ctx = Context::new(msg);
        plugin.execute(&mut ctx).await.unwrap();

        // Should not set response for non-PTR queries
        assert!(ctx.response().is_none());
    }
}
