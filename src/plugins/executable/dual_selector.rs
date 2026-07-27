//! Dual selector plugin
//!
//! Selects between IPv4 and IPv6 responses based on preference

use crate::dns::types::RecordType;
use crate::plugin::{Context, Plugin};
use crate::{RegisterPlugin, Result};
use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;
use tracing::debug;

// Auto-register using the register macro

/// IP version preference for dual selector
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpPreference {
    /// Prefer IPv4 (A records)
    IPv4,
    /// Prefer IPv6 (AAAA records)
    IPv6,
    /// Prefer IPv4, but allow IPv6 if no IPv4
    IPv4PreferIPv6Fallback,
    /// Prefer IPv6, but allow IPv4 if no IPv6
    IPv6PreferIPv4Fallback,
    /// Keep both IPv4 and IPv6
    Both,
}

/// Plugin that filters DNS responses based on IP version preference
///
/// # Example
///
/// ```rust
/// use lazydns::plugins::executable::{DualSelectorPlugin, IpPreference};
///
/// // Prefer IPv4 only
/// let plugin = DualSelectorPlugin::new(IpPreference::IPv4);
///
/// // Prefer IPv6 with IPv4 fallback
/// let plugin = DualSelectorPlugin::new(IpPreference::IPv6PreferIPv4Fallback);
/// ```
#[derive(RegisterPlugin)]
pub struct DualSelectorPlugin {
    /// IP version preference
    preference: IpPreference,
}

impl DualSelectorPlugin {
    /// Create a new dual selector plugin
    pub fn new(preference: IpPreference) -> Self {
        Self { preference }
    }

    /// Create a plugin that prefers IPv4
    pub fn prefer_ipv4() -> Self {
        Self {
            preference: IpPreference::IPv4,
        }
    }

    /// Create a plugin that prefers IPv6
    pub fn prefer_ipv6() -> Self {
        Self {
            preference: IpPreference::IPv6,
        }
    }
}

impl fmt::Debug for DualSelectorPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DualSelectorPlugin")
            .field("preference", &self.preference)
            .finish()
    }
}

#[async_trait]
impl Plugin for DualSelectorPlugin {
    fn name(&self) -> &str {
        "dual_selector"
    }

    fn init(config: &crate::config::types::PluginConfig) -> Result<std::sync::Arc<dyn Plugin>> {
        let args = config.effective_args();
        let pref_str = args
            .get("preference")
            .and_then(|v| v.as_str())
            .unwrap_or("both")
            .to_lowercase()
            .replace('-', "_");

        let preference = match pref_str.as_str() {
            "ipv4" => IpPreference::IPv4,
            "ipv6" => IpPreference::IPv6,
            "ipv4_prefer_ipv6_fallback" | "ipv4_prefer_ipv6" => {
                IpPreference::IPv4PreferIPv6Fallback
            }
            "ipv6_prefer_ipv4_fallback" | "ipv6_prefer_ipv4" => {
                IpPreference::IPv6PreferIPv4Fallback
            }
            "both" => IpPreference::Both,
            other => {
                return Err(crate::Error::Config(format!(
                    "dual_selector: unknown preference '{}'",
                    other
                )));
            }
        };

        Ok(Arc::new(DualSelectorPlugin::new(preference)))
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        if let Some(response) = ctx.response_mut() {
            let answers = response.answers_mut();

            // Count IPv4 and IPv6 records
            let has_ipv4 = answers.iter().any(|r| r.rtype() == RecordType::A);
            let has_ipv6 = answers.iter().any(|r| r.rtype() == RecordType::AAAA);

            match self.preference {
                IpPreference::IPv4 => {
                    // Keep only A records
                    answers.retain(|r| r.rtype() != RecordType::AAAA);
                    debug!("Dual selector: Keeping only IPv4 records");
                }
                IpPreference::IPv6 => {
                    // Keep only AAAA records
                    answers.retain(|r| r.rtype() != RecordType::A);
                    debug!("Dual selector: Keeping only IPv6 records");
                }
                IpPreference::IPv4PreferIPv6Fallback => {
                    if has_ipv4 {
                        answers.retain(|r| r.rtype() != RecordType::AAAA);
                        debug!("Dual selector: Preferring IPv4, removing IPv6");
                    } else {
                        debug!("Dual selector: No IPv4, keeping IPv6");
                    }
                }
                IpPreference::IPv6PreferIPv4Fallback => {
                    if has_ipv6 {
                        answers.retain(|r| r.rtype() != RecordType::A);
                        debug!("Dual selector: Preferring IPv6, removing IPv4");
                    } else {
                        debug!("Dual selector: No IPv6, keeping IPv4");
                    }
                }
                IpPreference::Both => {
                    // Keep everything
                    debug!("Dual selector: Keeping both IPv4 and IPv6");
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::types::RecordClass;
    use crate::dns::{Message, RData, ResourceRecord};

    #[tokio::test]
    async fn test_dual_selector_ipv4_only() {
        let plugin = DualSelectorPlugin::prefer_ipv4();

        let request = Message::new();
        let mut ctx = Context::new(request);

        let mut response = Message::new();
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A("192.0.2.1".parse().unwrap()),
        ));
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            RData::AAAA("2001:db8::1".parse().unwrap()),
        ));
        ctx.set_response(Some(response));

        plugin.execute(&mut ctx).await.unwrap();

        let response = ctx.response().unwrap();
        assert_eq!(response.answers().len(), 1);
        assert_eq!(response.answers()[0].rtype(), RecordType::A);
    }

    #[tokio::test]
    async fn test_dual_selector_ipv6_only() {
        let plugin = DualSelectorPlugin::prefer_ipv6();

        let request = Message::new();
        let mut ctx = Context::new(request);

        let mut response = Message::new();
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A("192.0.2.1".parse().unwrap()),
        ));
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            RData::AAAA("2001:db8::1".parse().unwrap()),
        ));
        ctx.set_response(Some(response));

        plugin.execute(&mut ctx).await.unwrap();

        let response = ctx.response().unwrap();
        assert_eq!(response.answers().len(), 1);
        assert_eq!(response.answers()[0].rtype(), RecordType::AAAA);
    }

    #[tokio::test]
    async fn test_dual_selector_ipv4_prefer_ipv6_fallback() {
        let plugin = DualSelectorPlugin::new(IpPreference::IPv4PreferIPv6Fallback);

        // Test with only IPv6
        let request = Message::new();
        let mut ctx = Context::new(request);

        let mut response = Message::new();
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            RData::AAAA("2001:db8::1".parse().unwrap()),
        ));
        ctx.set_response(Some(response));

        plugin.execute(&mut ctx).await.unwrap();

        let response = ctx.response().unwrap();
        assert_eq!(response.answers().len(), 1);
        assert_eq!(response.answers()[0].rtype(), RecordType::AAAA);
    }

    #[tokio::test]
    async fn test_dual_selector_both() {
        let plugin = DualSelectorPlugin::new(IpPreference::Both);

        let request = Message::new();
        let mut ctx = Context::new(request);

        let mut response = Message::new();
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A("192.0.2.1".parse().unwrap()),
        ));
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            RData::AAAA("2001:db8::1".parse().unwrap()),
        ));
        ctx.set_response(Some(response));

        plugin.execute(&mut ctx).await.unwrap();

        let response = ctx.response().unwrap();
        assert_eq!(response.answers().len(), 2);
    }
}
