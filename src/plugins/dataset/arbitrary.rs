// Arbitrary plugin moved under plugins::dataset
// (content copied from src/plugins/executable/arbitrary.rs)

use crate::RegisterPlugin;
use crate::Result;
use crate::dns::{Message, RData, ResourceRecord};
use crate::plugin::{Context, Plugin};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Debug, Deserialize, Clone)]
pub struct ArbitraryArgs {
    pub rules: Option<Vec<String>>,
    pub files: Option<Vec<String>>,
}

#[derive(RegisterPlugin)]
pub struct ArbitraryPlugin {
    map: HashMap<String, Vec<ResourceRecord>>,
}

impl ArbitraryPlugin {
    pub fn new(args: ArbitraryArgs) -> Result<Self> {
        let mut m: HashMap<String, Vec<ResourceRecord>> = HashMap::new();
        if let Some(rules) = args.rules {
            for (i, line) in rules.into_iter().enumerate() {
                if let Some(rr) = Self::parse_rr_line(&line) {
                    m.entry(rr.name().to_string()).or_default().push(rr);
                } else {
                    return Err(crate::Error::FileParse {
                        path: format!("inline rule #{}", i),
                        reason: format!("failed to parse: {}", line),
                    });
                }
            }
        }
        if let Some(files) = args.files {
            for file in files {
                let b = fs::read_to_string(&file).map_err(|e| crate::Error::FileParse {
                    path: file.clone(),
                    reason: format!("failed to read: {}", e),
                })?;
                for (i, line) in b.lines().enumerate() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                        continue;
                    }
                    if let Some(rr) = Self::parse_rr_line(line) {
                        m.entry(rr.name().to_string()).or_default().push(rr);
                    } else {
                        return Err(crate::Error::FileParse {
                            path: file.clone(),
                            reason: format!("failed to parse line {}", i + 1),
                        });
                    }
                }
            }
        }
        Ok(Self { map: m })
    }

    fn parse_rr_line(line: &str) -> Option<ResourceRecord> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            return None;
        }
        let name = parts[0].trim_end_matches('.').to_string();
        let (typ_idx, rdata_idx) = if parts[1].eq_ignore_ascii_case("IN") && parts.len() >= 4 {
            (2, 3)
        } else {
            (1, 2)
        };
        let typ = parts.get(typ_idx)?;
        let rdata = parts.get(rdata_idx)?;
        let rr = match typ.to_uppercase().as_str() {
            "A" => {
                let ip = Ipv4Addr::from_str(rdata).ok()?;
                ResourceRecord::new(
                    name,
                    crate::dns::types::RecordType::A,
                    crate::dns::types::RecordClass::IN,
                    300,
                    RData::A(ip),
                )
            }
            "AAAA" => {
                let ip = Ipv6Addr::from_str(rdata).ok()?;
                ResourceRecord::new(
                    name,
                    crate::dns::types::RecordType::AAAA,
                    crate::dns::types::RecordClass::IN,
                    300,
                    RData::AAAA(ip),
                )
            }
            "CNAME" => ResourceRecord::new(
                name,
                crate::dns::types::RecordType::CNAME,
                crate::dns::types::RecordClass::IN,
                300,
                RData::CNAME(rdata.to_string()),
            ),
            _ => {
                return None;
            }
        };
        Some(rr)
    }
}

impl fmt::Debug for ArbitraryPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Arbitrary")
            .field("rules_count", &self.map.len())
            .finish()
    }
}

#[async_trait]
impl Plugin for ArbitraryPlugin {
    fn name(&self) -> &str {
        "arbitrary"
    }

    fn init(config: &crate::config::types::PluginConfig) -> Result<Arc<dyn Plugin>> {
        use serde_yaml;
        use std::sync::Arc;

        let args: ArbitraryArgs = serde_yaml::from_value(config.args.clone())
            .map_err(|e| crate::Error::Config(format!("failed to parse arbitrary args: {}", e)))?;
        Ok(Arc::new(ArbitraryPlugin::new(args)?))
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        if let Some(q) = ctx.request().questions().first() {
            let key = q.qname().trim_end_matches('.').to_string();
            if let Some(rrs) = self.map.get(&key) {
                let mut msg = Message::new();
                msg.set_id(ctx.request().id());
                msg.set_response(true);
                msg.add_question(q.clone());
                for rr in rrs {
                    msg.add_answer(rr.clone());
                }
                ctx.set_response(Some(msg));
            }
        }
        // No question in the request: do nothing. Previously this returned an
        // arbitrary record from the map (HashMap order is non-deterministic),
        // leaking internal configuration to any client sending an empty query
        // (health checks, scanners). A DNS response must echo the query's
        // question section, which is impossible here, so leave the response
        // unset and let downstream handle it.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;
    use crate::dns::types::{RecordClass, RecordType};
    use crate::dns::{Question, RData};

    #[tokio::test]
    async fn test_arbitrary_rules() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com A 192.0.2.1".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        let mut req = Message::new();
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answer_count(), 1);
        if let RData::A(ip) = resp.answers()[0].rdata() {
            assert_eq!(*ip, Ipv4Addr::new(192, 0, 2, 1));
        } else {
            panic!("expected A");
        }
    }

    #[tokio::test]
    async fn test_arbitrary_aaaa_record() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com AAAA 2001:db8::1".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        let mut req = Message::new();
        req.add_question(Question::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
        ));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answer_count(), 1);
        match resp.answers()[0].rdata() {
            RData::AAAA(_) => {}
            _ => panic!("expected AAAA"),
        }
    }

    #[tokio::test]
    async fn test_arbitrary_cname_record() {
        let args = ArbitraryArgs {
            rules: Some(vec!["alias.com CNAME target.com".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        let mut req = Message::new();
        req.add_question(Question::new(
            "alias.com",
            RecordType::CNAME,
            RecordClass::IN,
        ));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();
        assert_eq!(resp.answer_count(), 1);
        match resp.answers()[0].rdata() {
            RData::CNAME(name) => assert_eq!(name, "target.com"),
            _ => panic!("expected CNAME"),
        }
    }

    #[tokio::test]
    async fn test_arbitrary_with_in_class() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com IN A 10.0.0.1".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        assert!(!plugin.map.is_empty());
    }

    #[tokio::test]
    async fn test_arbitrary_trailing_dot() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com. A 192.0.2.2".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        let mut req = Message::new();
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        assert!(ctx.response().is_some());
    }

    #[test]
    fn test_arbitrary_parse_invalid_rule() {
        let args = ArbitraryArgs {
            rules: Some(vec!["invalid".to_string()]), // Too few parts
            files: None,
        };
        let result = ArbitraryPlugin::new(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_arbitrary_parse_invalid_ip() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com A not-an-ip".to_string()]),
            files: None,
        };
        let result = ArbitraryPlugin::new(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_arbitrary_parse_unknown_type() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com MX mail.example.com".to_string()]),
            files: None,
        };
        let result = ArbitraryPlugin::new(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_arbitrary_empty_rules() {
        let args = ArbitraryArgs {
            rules: Some(vec![]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        assert!(plugin.map.is_empty());
    }

    #[test]
    fn test_arbitrary_none_rules() {
        let args = ArbitraryArgs {
            rules: None,
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        assert!(plugin.map.is_empty());
    }

    #[test]
    fn test_arbitrary_debug() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com A 1.2.3.4".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("Arbitrary"));
        assert!(debug_str.contains("rules_count"));
    }

    #[tokio::test]
    async fn test_arbitrary_no_question() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com A 1.2.3.4".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        let req = Message::new(); // No question
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        // A request without a question section must not produce a response:
        // doing so would leak an arbitrary configured record (the previous
        // behavior returned a HashMap's first entry, which is unpredictable
        // and unrelated to the query).
        assert!(
            ctx.response().is_none(),
            "no response should be synthesized for a question-less request"
        );
    }

    #[tokio::test]
    async fn test_arbitrary_not_found() {
        let args = ArbitraryArgs {
            rules: Some(vec!["example.com A 1.2.3.4".to_string()]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        let mut req = Message::new();
        req.add_question(Question::new("other.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        // Not found in map
        assert!(ctx.response().is_none());
    }

    #[test]
    fn test_arbitrary_multiple_records_same_domain() {
        let args = ArbitraryArgs {
            rules: Some(vec![
                "example.com A 1.1.1.1".to_string(),
                "example.com A 2.2.2.2".to_string(),
            ]),
            files: None,
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        assert_eq!(plugin.map.get("example.com").unwrap().len(), 2);
    }

    #[test]
    fn test_arbitrary_file_not_found() {
        let args = ArbitraryArgs {
            rules: None,
            files: Some(vec!["/nonexistent/path.txt".to_string()]),
        };
        let result = ArbitraryPlugin::new(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_arbitrary_file_with_comments() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp.as_file(), "# This is a comment").unwrap();
        writeln!(tmp.as_file(), "; Another comment").unwrap();
        writeln!(tmp.as_file()).unwrap();
        writeln!(tmp.as_file(), "example.com A 1.2.3.4").unwrap();

        let args = ArbitraryArgs {
            rules: None,
            files: Some(vec![tmp.path().to_str().unwrap().to_string()]),
        };
        let plugin = ArbitraryPlugin::new(args).unwrap();
        assert_eq!(plugin.map.len(), 1);
    }

    #[test]
    fn test_arbitrary_file_invalid_line() {
        use std::io::Write;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp.as_file(), "invalid").unwrap();

        let args = ArbitraryArgs {
            rules: None,
            files: Some(vec![tmp.path().to_str().unwrap().to_string()]),
        };
        let result = ArbitraryPlugin::new(args);
        assert!(result.is_err());
    }
}
