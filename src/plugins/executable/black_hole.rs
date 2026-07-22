use crate::config::PluginConfig;
use crate::dns::{Message, RData, ResourceRecord};
use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, RegisterPlugin, Result};
use async_trait::async_trait;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

const PLUGIN_BLACKHOLE_IDENTIFIER: &str = "blackhole";

/// Black hole plugin: returns configured A/AAAA answers for a query
#[derive(RegisterPlugin, RegisterExecPlugin)]
pub struct BlackholePlugin {
    ipv4: Vec<Ipv4Addr>,
    ipv6: Vec<Ipv6Addr>,
}

impl BlackholePlugin {
    /// Create from iterator of address strings
    pub fn new_from_strs<I, S>(ips: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut ipv4 = Vec::new();
        let mut ipv6 = Vec::new();
        for s in ips {
            let s = s.as_ref();
            if let Ok(a4) = s.parse::<Ipv4Addr>() {
                ipv4.push(a4);
            } else if let Ok(a6) = s.parse::<Ipv6Addr>() {
                ipv6.push(a6);
            } else {
                return Err(crate::Error::InvalidAddress {
                    input: s.to_string(),
                });
            }
        }
        Ok(Self { ipv4, ipv6 })
    }

    fn make_response_for_a(&self, req: &Message) -> Option<Message> {
        if req.question_count() != 1 || self.ipv4.is_empty() {
            return None;
        }
        let q = &req.questions()[0];
        if q.qtype() != crate::dns::types::RecordType::A {
            return None;
        }
        let mut r = Message::new();
        r.set_id(req.id());
        r.set_response(true);
        r.add_question(q.clone());
        for ip in &self.ipv4 {
            r.add_answer(ResourceRecord::new(
                q.qname(),
                crate::dns::types::RecordType::A,
                crate::dns::types::RecordClass::IN,
                300,
                RData::A(*ip),
            ));
        }
        Some(r)
    }

    fn make_response_for_aaaa(&self, req: &Message) -> Option<Message> {
        if req.question_count() != 1 || self.ipv6.is_empty() {
            return None;
        }
        let q = &req.questions()[0];
        if q.qtype() != crate::dns::types::RecordType::AAAA {
            return None;
        }
        let mut r = Message::new();
        r.set_id(req.id());
        r.set_response(true);
        r.add_question(q.clone());
        for ip in &self.ipv6 {
            r.add_answer(ResourceRecord::new(
                q.qname(),
                crate::dns::types::RecordType::AAAA,
                crate::dns::types::RecordClass::IN,
                300,
                RData::AAAA(*ip),
            ));
        }
        Some(r)
    }
}

impl fmt::Debug for BlackholePlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlackHolePlugin")
            .field("ipv4_count", &self.ipv4.len())
            .field("ipv6_count", &self.ipv6.len())
            .finish()
    }
}

#[async_trait]
impl Plugin for BlackholePlugin {
    fn name(&self) -> &str {
        PLUGIN_BLACKHOLE_IDENTIFIER
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        let req = ctx.request();
        if let Some(resp) = self
            .make_response_for_a(req)
            .or_else(|| self.make_response_for_aaaa(req))
        {
            #[cfg(feature = "web")]
            // Log blocked domain query event
            if let Some(q) = req.questions().first() {
                let qname = q.qname().to_string();
                let client_ip = ctx.get_metadata::<std::net::IpAddr>("client_ip").copied();

                crate::plugins::AUDIT_LOGGER
                    .log_security_event(
                        crate::plugins::SecurityEventType::BlockedDomainQuery,
                        format!("Blocked domain query: {}", qname),
                        client_ip,
                        Some(qname),
                    )
                    .await;
            }

            ctx.set_response(Some(resp));
        }
        Ok(())
    }

    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>> {
        let args = config.effective_args();
        use serde_yaml::Value;

        // Blackhole plugin can accept a list of IPs or be empty (defaults to common sinkhole IPs)
        let ips: Vec<String> = if let Some(Value::Sequence(seq)) = args.get("ips") {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        } else {
            Vec::new() // Empty will use default sinkhole IPs
        };

        let plugin = BlackholePlugin::new_from_strs(ips)?;
        Ok(Arc::new(plugin))
    }

    fn aliases() -> &'static [&'static str] {
        &["sinkhole", "black_hole", "null_dns"]
    }
}

impl ExecPlugin for BlackholePlugin {
    /// Parse a quick configuration string for blackhole plugin.
    ///
    /// Accepts a comma-separated list of IP addresses.
    /// Examples: "192.0.2.1", "192.0.2.1,2001:db8::1", "0.0.0.0"
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        // Accept the main name and all aliases
        if prefix != PLUGIN_BLACKHOLE_IDENTIFIER && !Self::aliases().contains(&prefix) {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected one of {:?}",
                prefix,
                Self::aliases()
            )));
        }

        // Parse comma-separated IP addresses
        let ips: Vec<String> = exec_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let plugin = BlackholePlugin::new_from_strs(ips)?;
        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::types::{RecordClass, RecordType};

    #[test]
    fn test_black_hole_new_ipv4_only() {
        let plugin = BlackholePlugin::new_from_strs(["192.0.2.1", "192.0.2.2"]).unwrap();
        assert_eq!(plugin.ipv4.len(), 2);
        assert_eq!(plugin.ipv6.len(), 0);
    }

    #[test]
    fn test_black_hole_new_ipv6_only() {
        let plugin = BlackholePlugin::new_from_strs(["2001:db8::1", "2001:db8::2"]).unwrap();
        assert_eq!(plugin.ipv4.len(), 0);
        assert_eq!(plugin.ipv6.len(), 2);
    }

    #[test]
    fn test_black_hole_new_mixed() {
        let plugin = BlackholePlugin::new_from_strs(["192.0.2.1", "2001:db8::1"]).unwrap();
        assert_eq!(plugin.ipv4.len(), 1);
        assert_eq!(plugin.ipv6.len(), 1);
    }

    #[test]
    fn test_black_hole_new_empty() {
        let plugin = BlackholePlugin::new_from_strs(Vec::<String>::new()).unwrap();
        assert_eq!(plugin.ipv4.len(), 0);
        assert_eq!(plugin.ipv6.len(), 0);
    }

    #[test]
    fn test_black_hole_new_invalid_ip() {
        let result = BlackholePlugin::new_from_strs(["not-an-ip"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_black_hole_debug() {
        let plugin = BlackholePlugin::new_from_strs(["1.2.3.4", "::1"]).unwrap();
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("BlackHolePlugin"));
        assert!(debug_str.contains("ipv4_count"));
        assert!(debug_str.contains("ipv6_count"));
    }

    #[test]
    fn test_black_hole_name() {
        let plugin = BlackholePlugin::new_from_strs(["0.0.0.0"]).unwrap();
        assert_eq!(plugin.name(), "blackhole");
    }

    #[test]
    fn test_black_hole_aliases() {
        let aliases = BlackholePlugin::aliases();
        assert!(aliases.contains(&"sinkhole"));
        assert!(aliases.contains(&"black_hole"));
        assert!(aliases.contains(&"null_dns"));
    }

    #[tokio::test]
    async fn test_black_hole_a() {
        let plugin = BlackholePlugin::new_from_strs(["192.0.2.1"]).unwrap();
        let mut req = Message::new();
        req.add_question(crate::dns::question::Question::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
        ));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answer_count(), 1);
        if let RData::A(ip) = resp.answers()[0].rdata() {
            assert_eq!(*ip, Ipv4Addr::new(192, 0, 2, 1));
        } else {
            panic!("expected A");
        }
    }

    #[tokio::test]
    async fn test_black_hole_aaaa() {
        let plugin = BlackholePlugin::new_from_strs(["2001:db8::1"]).unwrap();
        let mut req = Message::new();
        req.add_question(crate::dns::question::Question::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
        ));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answer_count(), 1);
        match resp.answers()[0].rdata() {
            RData::AAAA(_) => {}
            _ => panic!("expected AAAA"),
        }
    }

    #[tokio::test]
    async fn test_black_hole_multiple_answers() {
        let plugin =
            BlackholePlugin::new_from_strs(["192.0.2.1", "192.0.2.2", "192.0.2.3"]).unwrap();
        let mut req = Message::new();
        req.add_question(crate::dns::question::Question::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
        ));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answer_count(), 3);
    }

    #[tokio::test]
    async fn test_black_hole_no_matching_type() {
        let plugin = BlackholePlugin::new_from_strs(["192.0.2.1"]).unwrap(); // Only IPv4
        let mut req = Message::new();
        req.add_question(crate::dns::question::Question::new(
            "example.com",
            RecordType::AAAA, // But query is for AAAA
            RecordClass::IN,
        ));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        // Should not set response
        assert!(ctx.response().is_none());
    }

    #[tokio::test]
    async fn test_black_hole_no_questions() {
        let plugin = BlackholePlugin::new_from_strs(["192.0.2.1"]).unwrap();
        let req = Message::new(); // No questions
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        // Should not set response
        assert!(ctx.response().is_none());
    }

    #[tokio::test]
    async fn test_black_hole_empty_config() {
        let plugin = BlackholePlugin::new_from_strs(Vec::<String>::new()).unwrap();
        let mut req = Message::new();
        req.add_question(crate::dns::question::Question::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
        ));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        // No IPs configured, should not set response
        assert!(ctx.response().is_none());
    }

    #[test]
    fn test_exec_plugin_quick_setup() {
        // Test that ExecPlugin::quick_setup works correctly
        let plugin =
            <BlackholePlugin as ExecPlugin>::quick_setup("blackhole", "192.0.2.1").unwrap();
        assert_eq!(plugin.name(), "blackhole");

        // Test invalid prefix
        let result = <BlackholePlugin as ExecPlugin>::quick_setup("invalid", "192.0.2.1");
        assert!(result.is_err());

        // Test multiple IPs
        let plugin =
            <BlackholePlugin as ExecPlugin>::quick_setup("blackhole", "192.0.2.1,2001:db8::1")
                .unwrap();
        assert_eq!(plugin.name(), "blackhole");
    }

    #[test]
    fn test_exec_plugin_quick_setup_aliases() {
        // Test that aliases work
        let plugin = <BlackholePlugin as ExecPlugin>::quick_setup("sinkhole", "0.0.0.0").unwrap();
        assert_eq!(plugin.name(), "blackhole");

        let plugin = <BlackholePlugin as ExecPlugin>::quick_setup("black_hole", "0.0.0.0").unwrap();
        assert_eq!(plugin.name(), "blackhole");

        let plugin = <BlackholePlugin as ExecPlugin>::quick_setup("null_dns", "0.0.0.0").unwrap();
        assert_eq!(plugin.name(), "blackhole");
    }

    #[test]
    fn test_exec_plugin_quick_setup_empty_ips() {
        let plugin = <BlackholePlugin as ExecPlugin>::quick_setup("blackhole", "").unwrap();
        assert_eq!(plugin.name(), "blackhole");
    }

    #[test]
    fn test_exec_plugin_quick_setup_invalid_ip() {
        let result = <BlackholePlugin as ExecPlugin>::quick_setup("blackhole", "not-an-ip");
        assert!(result.is_err());
    }
}
