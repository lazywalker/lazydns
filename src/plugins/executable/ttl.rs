use crate::config::PluginConfig;
use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, Result};
use async_trait::async_trait;
use std::fmt;
use std::sync::Arc;

#[derive(RegisterExecPlugin)]
pub struct TtlPlugin {
    fix: u32,
    min: u32,
    max: u32,
}

impl TtlPlugin {
    pub fn new(fix: u32, min: u32, max: u32) -> Self {
        Self { fix, min, max }
    }

    fn apply(&self, ctx: &mut Context) {
        if let Some(resp) = ctx.response_mut() {
            if self.fix > 0 {
                for rr in resp.answers_mut().iter_mut() {
                    rr.set_ttl(self.fix);
                }
                for rr in resp.authority_mut().iter_mut() {
                    rr.set_ttl(self.fix);
                }
                for rr in resp.additional_mut().iter_mut() {
                    rr.set_ttl(self.fix);
                }
            } else {
                if self.min > 0 {
                    for rr in resp.answers_mut().iter_mut() {
                        if rr.ttl() < self.min {
                            rr.set_ttl(self.min);
                        }
                    }
                    for rr in resp.authority_mut().iter_mut() {
                        if rr.ttl() < self.min {
                            rr.set_ttl(self.min);
                        }
                    }
                    for rr in resp.additional_mut().iter_mut() {
                        if rr.ttl() < self.min {
                            rr.set_ttl(self.min);
                        }
                    }
                }
                if self.max > 0 {
                    for rr in resp.answers_mut().iter_mut() {
                        if rr.ttl() > self.max {
                            rr.set_ttl(self.max);
                        }
                    }
                    for rr in resp.authority_mut().iter_mut() {
                        if rr.ttl() > self.max {
                            rr.set_ttl(self.max);
                        }
                    }
                    for rr in resp.additional_mut().iter_mut() {
                        if rr.ttl() > self.max {
                            rr.set_ttl(self.max);
                        }
                    }
                }
            }
        }
    }
}

impl fmt::Debug for TtlPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtlPlugin").finish()
    }
}

#[async_trait]
impl Plugin for TtlPlugin {
    fn name(&self) -> &str {
        "ttl"
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        self.apply(ctx);
        Ok(())
    }

    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>> {
        let args = config.effective_args();

        let ttl = args.get("ttl").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
        let min = args.get("min").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
        let max = args.get("max").and_then(|v| v.as_i64()).unwrap_or(0) as u32;

        // preserve old behavior: bare `ttl:` with no other args defaults to 300s
        let (fix, min, max) = if ttl == 0 && min == 0 && max == 0 {
            (300, 0, 0)
        } else {
            (ttl, min, max)
        };
        Ok(Arc::new(TtlPlugin::new(fix, min, max)))
    }
}

impl ExecPlugin for TtlPlugin {
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != "ttl" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'ttl'",
                prefix
            )));
        }

        let plugin = if exec_str.contains('-') {
            let parts: Vec<&str> = exec_str.splitn(2, '-').collect();
            let l = parts[0].parse::<u32>().unwrap_or(0);
            let u = parts[1].parse::<u32>().unwrap_or(0);
            TtlPlugin::new(0, l, u)
        } else {
            let f = exec_str.parse::<u32>().unwrap_or(0);
            TtlPlugin::new(f, 0, 0)
        };

        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;
    use crate::dns::types::{RecordClass, RecordType};
    use crate::dns::{Message, RData, ResourceRecord};

    fn make_a_record(name: &str, ttl: u32) -> ResourceRecord {
        ResourceRecord::new(
            name,
            RecordType::A,
            RecordClass::IN,
            ttl,
            crate::dns::RData::A(std::net::Ipv4Addr::new(1, 2, 3, 4)),
        )
    }

    #[tokio::test]
    async fn test_ttl_fix() {
        let plugin = TtlPlugin::new(10, 0, 0);
        let mut msg = Message::new();
        msg.add_answer(make_a_record("a", 300));
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answers()[0].ttl(), 10);
    }

    #[tokio::test]
    async fn test_ttl_min_clamp() {
        let plugin = TtlPlugin::new(0, 50, 0);
        let mut msg = Message::new();
        msg.add_answer(make_a_record("a", 30));
        msg.add_authority(make_a_record("auth", 10));
        msg.add_additional(make_a_record("add", 20));

        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answers()[0].ttl(), 50);
        assert_eq!(resp.authority()[0].ttl(), 50);
        assert_eq!(resp.additional()[0].ttl(), 50);
    }

    #[tokio::test]
    async fn test_ttl_max_clamp() {
        let plugin = TtlPlugin::new(0, 0, 100);
        let mut msg = Message::new();
        msg.add_answer(make_a_record("a", 200));
        msg.add_authority(make_a_record("auth", 250));
        msg.add_additional(make_a_record("add", 500));

        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answers()[0].ttl(), 100);
        assert_eq!(resp.authority()[0].ttl(), 100);
        assert_eq!(resp.additional()[0].ttl(), 100);
    }

    #[tokio::test]
    async fn test_ttl_min_max_range() {
        let plugin = TtlPlugin::new(0, 30, 100);
        let mut msg = Message::new();
        msg.add_answer(make_a_record("low", 10));
        msg.add_answer(make_a_record("mid", 50));
        msg.add_answer(make_a_record("high", 200));

        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answers()[0].ttl(), 30);
        assert_eq!(resp.answers()[1].ttl(), 50);
        assert_eq!(resp.answers()[2].ttl(), 100);
    }

    #[tokio::test]
    async fn test_ttl_plugin_rewrites_records() {
        let mut response = Message::new();
        response.set_response(true);
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A(Ipv4Addr::new(192, 0, 2, 1)),
        ));
        response.add_additional(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            450,
            RData::AAAA(Ipv6Addr::LOCALHOST),
        ));

        let mut ctx = Context::new(Message::new());
        ctx.set_response(Some(response));

        let plugin = TtlPlugin::new(60, 0, 0);
        plugin.execute(&mut ctx).await.unwrap();

        let resp = ctx.response().unwrap();
        assert!(resp.answers().iter().all(|r| r.ttl() == 60));
        assert!(resp.additional().iter().all(|r| r.ttl() == 60));
    }

    #[test]
    fn test_exec_plugin_quick_setup() {
        // Test that ExecPlugin::quick_setup works correctly
        let plugin = <TtlPlugin as ExecPlugin>::quick_setup("ttl", "60").unwrap();
        assert_eq!(plugin.name(), "ttl");

        // Test invalid prefix
        let result = <TtlPlugin as ExecPlugin>::quick_setup("invalid", "60");
        assert!(result.is_err());
    }

    /// Regression: `init` previously ignored `min`/`max` and always built a
    /// fixed-TTL plugin. Verify that min/max from config are honored.
    #[tokio::test]
    async fn test_init_honors_min_max() {
        use crate::config::types::PluginConfig;
        use serde_yaml::Value;

        let mut args = serde_yaml::Mapping::new();
        args.insert(Value::String("min".into()), Value::Number(50.into()));
        args.insert(Value::String("max".into()), Value::Number(100.into()));

        let cfg = PluginConfig {
            tag: None,
            plugin_type: "ttl".into(),
            args: Value::Mapping(args),
        };

        let plugin = <TtlPlugin as crate::plugin::Plugin>::init(&cfg).unwrap();

        // A TTL of 30 should be clamped up to min=50.
        let mut msg = Message::new();
        msg.add_answer(make_a_record("a", 30));
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        assert_eq!(ctx.response().unwrap().answers()[0].ttl(), 50);

        // A TTL of 300 should be clamped down to max=100.
        let mut msg = Message::new();
        msg.add_answer(make_a_record("b", 300));
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        assert_eq!(ctx.response().unwrap().answers()[0].ttl(), 100);
    }
}
