//! Redirect plugin
//!
//! Redirects DNS queries to a different domain

use crate::plugin::{Context, Plugin};
use crate::{RegisterPlugin, Result};
use async_trait::async_trait;
use std::fmt;
use tracing::debug;

struct Rule {
    from: String,
    to: String,
}

impl Rule {
    fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
        }
    }

    fn matches(&self, qname: &str) -> bool {
        let from_lower = self.from.to_lowercase();
        let qname_lower = qname.to_lowercase();

        if let Some(suffix) = from_lower.strip_prefix("*.") {
            // Require a dot boundary so "evilexample.com" does not match "*.example.com".
            qname_lower == suffix || qname_lower.ends_with(&format!(".{suffix}"))
        } else {
            qname_lower == from_lower
        }
    }

    fn apply(&self, qname: &str) -> String {
        let from_lower = self.from.to_lowercase();
        let qname_lower = qname.to_lowercase();
        let to_lower = self.to.to_lowercase();

        if let (Some(from_suffix), Some(to_suffix)) =
            (from_lower.strip_prefix("*."), to_lower.strip_prefix("*."))
            && let Some(mut prefix) = qname_lower.strip_suffix(from_suffix)
        {
            if prefix.ends_with('.') && to_suffix.starts_with('.') {
                prefix = &prefix[..prefix.len() - 1];
            }
            return format!("{}{}", prefix, to_suffix);
        }

        // Preserve original case from config.
        self.to.clone()
    }
}

/// Plugin that redirects queries from one domain to another
///
/// # Example
///
/// ```rust
/// use lazydns::plugins::executable::RedirectPlugin;
///
/// // Redirect example.com to example.net
/// let plugin = RedirectPlugin::new("example.com", "example.net");
///
/// // Redirect with wildcard
/// let mut plugin = RedirectPlugin::new("*.old.com", "*.new.com");
/// ```
#[derive(RegisterPlugin)]
pub struct RedirectPlugin {
    rules: Vec<Rule>,
}

impl RedirectPlugin {
    /// Create a redirect plugin with a single rule.
    pub fn new(from_domain: impl Into<String>, to_domain: impl Into<String>) -> Self {
        Self {
            rules: vec![Rule::new(from_domain, to_domain)],
        }
    }

    fn add_rule(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.rules.push(Rule::new(from, to));
    }
}

impl fmt::Debug for RedirectPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedirectPlugin")
            .field("rules", &self.rules.len())
            .finish()
    }
}

#[async_trait]
impl Plugin for RedirectPlugin {
    fn name(&self) -> &str {
        "redirect"
    }

    fn init(config: &crate::config::types::PluginConfig) -> Result<std::sync::Arc<dyn Plugin>> {
        use serde_yaml::Value;
        use std::sync::Arc;

        let args = config.effective_args();
        let Some(Value::Sequence(seq)) = args.get("rules") else {
            return Err(crate::Error::Config(
                "redirect plugin requires 'rules' array".to_string(),
            ));
        };
        if seq.is_empty() {
            return Err(crate::Error::Config(
                "redirect requires at least one rule".to_string(),
            ));
        }

        let mut plugin = RedirectPlugin {
            rules: Vec::with_capacity(seq.len()),
        };

        for entry in seq {
            let (from, to) = match entry {
                Value::String(s) => {
                    let parts: Vec<&str> = s.split_whitespace().collect();
                    if parts.len() != 2 {
                        return Err(crate::Error::Config(format!(
                            "redirect rule must be 'from to', got: {s}"
                        )));
                    }
                    (parts[0].to_string(), parts[1].to_string())
                }
                Value::Mapping(map) => {
                    let from = map
                        .get(Value::String("from".to_string()))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            crate::Error::Config("redirect rule missing 'from'".to_string())
                        })?;
                    let to = map
                        .get(Value::String("to".to_string()))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            crate::Error::Config("redirect rule missing 'to'".to_string())
                        })?;
                    (from.to_string(), to.to_string())
                }
                other => {
                    return Err(crate::Error::Config(format!(
                        "unsupported redirect rule format: {other:?}"
                    )));
                }
            };
            plugin.add_rule(from, to);
        }

        Ok(Arc::new(plugin))
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        let request = ctx.request_mut();

        if let Some(question) = request.questions_mut().first_mut() {
            let qname = question.qname().to_string();

            // first match wins
            if let Some(rule) = self.rules.iter().find(|r| r.matches(&qname)) {
                let new_qname = rule.apply(&qname);
                debug!("Redirecting query from {} to {}", qname, new_qname);
                question.set_qname(new_qname);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::types::{RecordClass, RecordType};
    use crate::dns::{Message, Question};

    #[tokio::test]
    async fn test_redirect_exact() {
        let plugin = RedirectPlugin::new("example.com", "example.net");

        let mut request = Message::new();
        request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);

        plugin.execute(&mut ctx).await.unwrap();

        let request = ctx.request();
        assert_eq!(request.questions().first().unwrap().qname(), "example.net");
    }

    #[tokio::test]
    async fn test_redirect_wildcard() {
        let plugin = RedirectPlugin::new("*.old.com", "*.new.com");

        let mut request = Message::new();
        request.add_question(Question::new("www.old.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);

        plugin.execute(&mut ctx).await.unwrap();

        let request = ctx.request();
        assert_eq!(request.questions().first().unwrap().qname(), "www.new.com");
    }

    #[tokio::test]
    async fn test_redirect_no_match() {
        let plugin = RedirectPlugin::new("example.com", "example.net");

        let mut request = Message::new();
        request.add_question(Question::new(
            "different.com",
            RecordType::A,
            RecordClass::IN,
        ));
        let mut ctx = Context::new(request);

        plugin.execute(&mut ctx).await.unwrap();

        // Should remain unchanged
        let request = ctx.request();
        assert_eq!(
            request.questions().first().unwrap().qname(),
            "different.com"
        );
    }

    /// Regression: a wildcard `*.old.com` must not match a name that merely
    /// ends with the suffix without a label boundary. Previously
    /// `evilold.com` matched `*.old.com` via `ends_with("old.com")`.
    #[tokio::test]
    async fn test_redirect_wildcard_requires_label_boundary() {
        let plugin = RedirectPlugin::new("*.old.com", "*.new.com");

        // Proper subdomain matches.
        let mut request = Message::new();
        request.add_question(Question::new("www.old.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);
        plugin.execute(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.request().questions().first().unwrap().qname(),
            "www.new.com"
        );

        // Suffix collision without a dot boundary must NOT match.
        let mut request = Message::new();
        request.add_question(Question::new("evilold.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);
        plugin.execute(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.request().questions().first().unwrap().qname(),
            "evilold.com"
        );
    }

    #[tokio::test]
    async fn test_redirect_case_insensitive() {
        let plugin = RedirectPlugin::new("Example.COM", "example.net");

        let mut request = Message::new();
        request.add_question(Question::new("EXAMPLE.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);

        plugin.execute(&mut ctx).await.unwrap();

        let request = ctx.request();
        assert_eq!(request.questions().first().unwrap().qname(), "example.net");
    }

    // Regression: init previously took only seq[0] and silently dropped the rest.
    #[tokio::test]
    async fn test_init_multiple_rules() {
        use crate::config::types::PluginConfig;

        let args = serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([(
            serde_yaml::Value::String("rules".into()),
            serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("a.test a.out".into()),
                serde_yaml::Value::String("b.test b.out".into()),
            ]),
        )]));

        let cfg = PluginConfig {
            tag: None,
            plugin_type: "redirect".into(),
            args,
        };

        let plugin = <RedirectPlugin as crate::plugin::Plugin>::init(&cfg).unwrap();

        for (qname, expect) in [("a.test", "a.out"), ("b.test", "b.out")] {
            let mut request = Message::new();
            request.add_question(Question::new(qname, RecordType::A, RecordClass::IN));
            let mut ctx = Context::new(request);
            plugin.execute(&mut ctx).await.unwrap();
            assert_eq!(ctx.request().questions().first().unwrap().qname(), expect);
        }
    }

    // first match wins when rules overlap
    #[tokio::test]
    async fn test_init_multiple_rules_first_match_wins() {
        use crate::config::types::PluginConfig;

        let args = serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([(
            serde_yaml::Value::String("rules".into()),
            serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("*.old.com first.out".into()),
                serde_yaml::Value::String("www.old.com second.out".into()),
            ]),
        )]));

        let cfg = PluginConfig {
            tag: None,
            plugin_type: "redirect".into(),
            args,
        };

        let plugin = <RedirectPlugin as crate::plugin::Plugin>::init(&cfg).unwrap();

        let mut request = Message::new();
        request.add_question(Question::new("www.old.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);
        plugin.execute(&mut ctx).await.unwrap();
        assert_eq!(
            ctx.request().questions().first().unwrap().qname(),
            "first.out"
        );
    }
}
