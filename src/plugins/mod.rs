//! DNS plugins collection
//!
//! This module contains concrete implementations of DNS plugins.
//! Each plugin implements the Plugin trait and provides specific
//! DNS query processing functionality.
//!
//! # Available Plugins
//!
//! - **forward**: Forward queries to upstream DNS servers
//! - **cache**: Cache DNS responses with TTL-based expiration and LRU eviction
//! - **hosts**: Resolve from local hosts file mappings
//! - **domain_matcher**: Match domains against patterns with wildcard support
//! - **ip_matcher**: Match response IPs against CIDR ranges
//! - **geoip**: Geographic IP address matching
//! - **geosite**: Geographic domain name matching
//! - **advanced**: Upstream control/utility plugins (TTL rewrite, blackhole, etc.)
//!
//! # Example
//!
//! ```rust,no_run
//! use lazydns::plugins::ForwardPlugin;
//! use lazydns::plugin::{Plugin, Context};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let plugin = ForwardPlugin::new(vec!["8.8.8.8:53".to_string()]);
//! let plugin: Arc<dyn Plugin> = Arc::new(plugin);
//! # Ok(())
//! # }
//! ```

pub mod acl;
#[cfg(feature = "web")]
pub mod audit;
pub mod cache;
#[cfg(feature = "cron")]
pub mod cron;
pub mod dataset;
pub mod domain_validator;
pub mod executable;
pub mod flow;
pub mod forward;
pub mod geoip;
pub mod geosite;
// utils module moved to crate-level `src/utils.rs`

// Re-export plugins
pub use acl::{AclAction, QueryAclPlugin};
#[cfg(feature = "web")]
pub use audit::{AUDIT_LOGGER, AuditEvent, AuditLogger, QueryLogEntry, SecurityEventType};
pub use cache::CachePlugin;
pub use dataset::{ArbitraryPlugin, DomainSetPlugin, HostsPlugin, IpSetPlugin};
pub use flow::{
    AcceptPlugin, GotoPlugin, JumpPlugin, PreferIpv4Plugin, PreferIpv6Plugin, RejectPlugin,
    ReturnPlugin,
};
pub use forward::{ForwardPlugin, LoadBalanceStrategy};
pub use geoip::GeoIpPlugin;
pub use geosite::GeoSitePlugin;
// Hosts and Arbitrary moved to `plugins::dataset`

// Re-export matcher plugins (mostly deprecated, see condition builders)

// Re-export executable plugins
pub use executable::{
    BlackholePlugin, DebugPrintPlugin, DropRespPlugin, DualSelectorPlugin, Edns0OptPlugin,
    Edns0Option, FallbackPlugin, IpPreference, MarkPlugin, NftSetPlugin, QuerySummaryPlugin,
    RateLimitPlugin, RedirectPlugin, ReverseLookupPlugin, RosAddrlistPlugin, SequencePlugin,
    SequenceStep, SleepPlugin, TtlPlugin,
};

#[cfg(feature = "cron")]
pub use cron::CronPlugin;

// Re-add tests module at file end to satisfy lints
#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Plugin;
    use std::sync::Arc;

    #[test]
    fn test_forward_plugin_accessible() {
        // Verify ForwardPlugin can be created
        let plugin = ForwardPlugin::new(vec!["8.8.8.8:53".to_string()]);
        assert_eq!(plugin.name(), "forward");
    }

    #[test]
    fn test_cache_plugin_accessible() {
        // Verify CachePlugin can be created
        let plugin = CachePlugin::new(100);
        assert_eq!(plugin.name(), "cache");
    }

    #[test]
    fn test_hosts_plugin_accessible() {
        // Verify HostsPlugin can be created
        let plugin = HostsPlugin::new();
        assert_eq!(plugin.name(), "hosts");
    }

    #[test]
    fn test_ratelimit_plugin_accessible() {
        // Verify RateLimitPlugin can be created
        let plugin = RateLimitPlugin::new(10, 60);
        assert_eq!(plugin.name(), "rate_limit"); // Note: actual name is "rate_limit" with underscore
    }

    #[test]
    fn test_advanced_plugins_accessible() {
        // Verify advanced plugin types are accessible
        // BlackholePlugin is implemented in `plugins::executable::black_hole` and
        // provides `new_from_strs` constructor.
        let _blackhole = BlackholePlugin::new_from_strs(Vec::<&str>::new()).unwrap();
        let _ttl = TtlPlugin::new(300, 0, 0);
        let _return = ReturnPlugin::new();
    }

    #[test]
    fn test_load_balance_strategy() {
        // Verify LoadBalanceStrategy enum is accessible
        let _rr = LoadBalanceStrategy::RoundRobin;
        let _random = LoadBalanceStrategy::Random;
        let _fastest = LoadBalanceStrategy::Fastest;

        assert_eq!(
            LoadBalanceStrategy::RoundRobin,
            LoadBalanceStrategy::RoundRobin
        );
    }

    #[test]
    fn test_acl_action() {
        // Verify AclAction enum is accessible
        let _allow = AclAction::Allow;
        let _deny = AclAction::Deny;
    }

    #[test]
    fn test_plugins_implement_trait() {
        // Verify plugins can be used as trait objects
        let forward: Arc<dyn Plugin> = Arc::new(ForwardPlugin::new(vec!["8.8.8.8:53".to_string()]));
        let cache: Arc<dyn Plugin> = Arc::new(CachePlugin::new(100));
        let hosts: Arc<dyn Plugin> = Arc::new(HostsPlugin::new());

        assert_eq!(forward.name(), "forward");
        assert_eq!(cache.name(), "cache");
        assert_eq!(hosts.name(), "hosts");
    }
}
