//! Query Access Control List (ACL) plugin
//!
//! Provides IP-based access control for DNS queries.

use crate::RegisterPlugin;
use crate::Result;
use crate::dns::ResponseCode;
use crate::plugin::{Context, Plugin};
use async_trait::async_trait;
use ipnet::IpNet;
use std::net::IpAddr;
use tracing::{debug, warn};

/// Default localhost IP address for fallback when client IP is missing
const LOCALHOST_IPV4: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1));

/// ACL action to take when a rule matches
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclAction {
    /// Allow the query to proceed
    Allow,
    /// Deny the query (return REFUSED)
    Deny,
}

/// ACL rule matching an IP range
#[derive(Debug, Clone)]
pub struct AclRule {
    /// IP network to match
    network: IpNet,
    /// Action to take on match
    action: AclAction,
}

impl AclRule {
    /// Create a new ACL rule
    pub fn new(network: IpNet, action: AclAction) -> Self {
        Self { network, action }
    }

    /// Check if an IP matches this rule
    fn matches(&self, ip: &IpAddr) -> bool {
        self.network.contains(ip)
    }
}

/// Query Access Control List plugin
///
/// Controls access to the DNS server based on client IP address.
///
/// # Example
///
/// ```rust
/// use lazydns::plugins::acl::{QueryAclPlugin, AclAction};
/// use ipnet::IpNet;
///
/// let mut acl = QueryAclPlugin::new(AclAction::Deny); // Default deny
/// acl.add_rule("192.168.0.0/16".parse().unwrap(), AclAction::Allow);
/// acl.add_rule("10.0.0.0/8".parse().unwrap(), AclAction::Allow);
/// ```
#[derive(Debug, RegisterPlugin)]
pub struct QueryAclPlugin {
    /// List of ACL rules (evaluated in order)
    rules: Vec<AclRule>,
    /// Default action if no rules match
    default_action: AclAction,
}

impl QueryAclPlugin {
    /// Create a new ACL plugin
    ///
    /// # Arguments
    ///
    /// * `default_action` - Action to take when no rules match
    pub fn new(default_action: AclAction) -> Self {
        Self {
            rules: Vec::new(),
            default_action,
        }
    }

    /// Add an ACL rule
    ///
    /// Rules are evaluated in the order they are added.
    pub fn add_rule(&mut self, network: IpNet, action: AclAction) {
        self.rules.push(AclRule::new(network, action));
    }

    /// Create an allow-list ACL (deny by default, allow specific networks)
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::acl::QueryAclPlugin;
    ///
    /// let acl = QueryAclPlugin::allow_list(vec![
    ///     "192.168.0.0/16".parse().unwrap(),
    ///     "10.0.0.0/8".parse().unwrap(),
    /// ]);
    /// ```
    pub fn allow_list(networks: Vec<IpNet>) -> Self {
        let mut acl = Self::new(AclAction::Deny);
        for network in networks {
            acl.add_rule(network, AclAction::Allow);
        }
        acl
    }

    /// Create a deny-list ACL (allow by default, deny specific networks)
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::acl::QueryAclPlugin;
    ///
    /// let acl = QueryAclPlugin::deny_list(vec![
    ///     "192.168.100.0/24".parse().unwrap(), // Block this subnet
    /// ]);
    /// ```
    pub fn deny_list(networks: Vec<IpNet>) -> Self {
        let mut acl = Self::new(AclAction::Allow);
        for network in networks {
            acl.add_rule(network, AclAction::Deny);
        }
        acl
    }

    /// Evaluate ACL for a given IP address
    fn evaluate(&self, ip: &IpAddr) -> AclAction {
        // Check rules in order
        for rule in &self.rules {
            if rule.matches(ip) {
                return rule.action;
            }
        }

        // No match, use default
        self.default_action
    }
}

#[async_trait]
impl Plugin for QueryAclPlugin {
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        // Get client IP from metadata, fall back to localhost constant
        let client_ip: IpAddr = match ctx.get_metadata::<IpAddr>("client_ip") {
            Some(ip) => *ip,
            None => {
                warn!("No client IP in metadata, using localhost");
                LOCALHOST_IPV4
            }
        };

        // Evaluate ACL
        let action = self.evaluate(&client_ip);

        match action {
            AclAction::Allow => {
                debug!("ACL: Allowed query from {}", client_ip);
                Ok(())
            }
            AclAction::Deny => {
                warn!("ACL: Denied query from {}", client_ip);

                #[cfg(feature = "web")]
                // Log ACL denied security event
                if let Some(q) = ctx.request().questions().first() {
                    let qname = q.qname().to_string();

                    crate::plugins::AUDIT_LOGGER
                        .log_security_event(
                            crate::plugins::SecurityEventType::AclDenied,
                            format!("Query denied by ACL rule from {}", client_ip),
                            Some(client_ip),
                            Some(qname),
                        )
                        .await;
                }

                // Create REFUSED response
                let mut response = crate::dns::Message::new();
                response.set_id(ctx.request().id());
                response.set_response(true);
                response.set_response_code(ResponseCode::Refused);

                ctx.set_response(Some(response));
                Ok(())
            }
        }
    }

    fn name(&self) -> &str {
        "query_acl"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn init(config: &crate::config::PluginConfig) -> Result<std::sync::Arc<dyn Plugin>> {
        let args = config.effective_args();
        use serde_yaml::Value;

        // Parse default_action parameter (optional, defaults to "deny")
        let default_action = match args.get("default") {
            Some(Value::String(action_str)) => match action_str.to_lowercase().as_str() {
                "allow" => AclAction::Allow,
                "deny" => AclAction::Deny,
                _ => {
                    return Err(crate::Error::InvalidConfigValue {
                        field: "default".to_string(),
                        value: action_str.clone(),
                        reason: "expected 'allow' or 'deny'".to_string(),
                    });
                }
            },
            Some(_) => {
                return Err(crate::Error::InvalidConfigValue {
                    field: "default".to_string(),
                    value: "(non-string)".to_string(),
                    reason: "must be a string".to_string(),
                });
            }
            None => AclAction::Deny, // Default to deny
        };

        let mut acl = QueryAclPlugin::new(default_action);

        // Parse rules parameter (optional)
        if let Some(Value::Sequence(rules)) = args.get("rules") {
            for rule_value in rules {
                if let Value::Mapping(rule_map) = rule_value {
                    // Parse network
                    let network_str = match rule_map.get(Value::String("network".to_string())) {
                        Some(Value::String(s)) => s.clone(),
                        Some(_) => {
                            return Err(crate::Error::InvalidConfigValue {
                                field: "network".to_string(),
                                value: "(non-string)".to_string(),
                                reason: "must be a string".to_string(),
                            });
                        }
                        None => {
                            return Err(crate::Error::MissingConfigField {
                                field: "network".to_string(),
                                context: "acl rule".to_string(),
                            });
                        }
                    };

                    let network: IpNet =
                        network_str
                            .parse()
                            .map_err(|e| crate::Error::InvalidConfigValue {
                                field: "network".to_string(),
                                value: network_str.clone(),
                                reason: format!("{}", e),
                            })?;

                    // Parse action
                    let action_str = match rule_map.get(Value::String("action".to_string())) {
                        Some(Value::String(s)) => s.clone(),
                        Some(_) => {
                            return Err(crate::Error::InvalidConfigValue {
                                field: "action".to_string(),
                                value: "(non-string)".to_string(),
                                reason: "must be a string".to_string(),
                            });
                        }
                        None => {
                            return Err(crate::Error::MissingConfigField {
                                field: "action".to_string(),
                                context: "acl rule".to_string(),
                            });
                        }
                    };

                    let action = match action_str.to_lowercase().as_str() {
                        "allow" => AclAction::Allow,
                        "deny" => AclAction::Deny,
                        _ => {
                            return Err(crate::Error::InvalidConfigValue {
                                field: "action".to_string(),
                                value: action_str.clone(),
                                reason: "expected 'allow' or 'deny'".to_string(),
                            });
                        }
                    };

                    acl.add_rule(network, action);
                } else {
                    return Err(crate::Error::Config(
                        "each rule must be a mapping".to_string(),
                    ));
                }
            }
        }

        Ok(std::sync::Arc::new(acl))
    }
}

// Auto-register using the register macro

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Message;

    #[test]
    fn test_acl_rule_matches() {
        let rule = AclRule::new("192.168.0.0/16".parse().unwrap(), AclAction::Allow);

        assert!(rule.matches(&"192.168.1.1".parse().unwrap()));
        assert!(rule.matches(&"192.168.255.255".parse().unwrap()));
        assert!(!rule.matches(&"10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_acl_default_action() {
        let acl = QueryAclPlugin::new(AclAction::Deny);
        let ip: IpAddr = "1.2.3.4".parse().unwrap();

        assert_eq!(acl.evaluate(&ip), AclAction::Deny);
    }

    #[test]
    fn test_acl_allow_list() {
        let acl = QueryAclPlugin::allow_list(vec![
            "192.168.0.0/16".parse().unwrap(),
            "10.0.0.0/8".parse().unwrap(),
        ]);

        assert_eq!(
            acl.evaluate(&"192.168.1.1".parse().unwrap()),
            AclAction::Allow
        );
        assert_eq!(acl.evaluate(&"10.0.0.1".parse().unwrap()), AclAction::Allow);
        assert_eq!(acl.evaluate(&"1.2.3.4".parse().unwrap()), AclAction::Deny);
    }

    #[test]
    fn test_acl_deny_list() {
        let acl = QueryAclPlugin::deny_list(vec!["192.168.100.0/24".parse().unwrap()]);

        assert_eq!(
            acl.evaluate(&"192.168.100.50".parse().unwrap()),
            AclAction::Deny
        );
        assert_eq!(
            acl.evaluate(&"192.168.1.1".parse().unwrap()),
            AclAction::Allow
        );
        assert_eq!(acl.evaluate(&"1.2.3.4".parse().unwrap()), AclAction::Allow);
    }

    #[test]
    fn test_acl_rule_order() {
        let mut acl = QueryAclPlugin::new(AclAction::Deny);
        // More specific rule first
        acl.add_rule("192.168.1.0/24".parse().unwrap(), AclAction::Allow);
        // Broader rule second
        acl.add_rule("192.168.0.0/16".parse().unwrap(), AclAction::Deny);

        // Should match first rule (more specific)
        assert_eq!(
            acl.evaluate(&"192.168.1.50".parse().unwrap()),
            AclAction::Allow
        );
        // Should match second rule
        assert_eq!(
            acl.evaluate(&"192.168.2.50".parse().unwrap()),
            AclAction::Deny
        );
    }

    #[tokio::test]
    async fn test_acl_plugin_allow() {
        let acl = QueryAclPlugin::allow_list(vec!["192.168.0.0/16".parse().unwrap()]);

        let mut ctx = Context::new(Message::new());
        ctx.set_metadata("client_ip", "192.168.1.1".parse::<IpAddr>().unwrap());

        acl.execute(&mut ctx).await.unwrap();

        // Should not set response (allowed to continue)
        assert!(ctx.response().is_none());
    }

    #[tokio::test]
    async fn test_acl_plugin_deny() {
        let acl = QueryAclPlugin::allow_list(vec!["192.168.0.0/16".parse().unwrap()]);

        let mut ctx = Context::new(Message::new());
        ctx.set_metadata("client_ip", "1.2.3.4".parse::<IpAddr>().unwrap());

        acl.execute(&mut ctx).await.unwrap();

        // Should set REFUSED response
        assert!(ctx.response().is_some());
        assert_eq!(
            ctx.response().unwrap().response_code(),
            ResponseCode::Refused
        );
    }

    #[test]
    fn test_acl_plugin_init_allow_list() {
        use crate::config::types::PluginConfig;
        use serde_yaml::{Mapping, Value};

        let mut args = Mapping::new();
        args.insert(
            Value::String("default".to_string()),
            Value::String("deny".to_string()),
        );

        let mut rules = Vec::new();
        let mut rule1 = Mapping::new();
        rule1.insert(
            Value::String("network".to_string()),
            Value::String("192.168.0.0/16".to_string()),
        );
        rule1.insert(
            Value::String("action".to_string()),
            Value::String("allow".to_string()),
        );
        rules.push(Value::Mapping(rule1));

        let mut rule2 = Mapping::new();
        rule2.insert(
            Value::String("network".to_string()),
            Value::String("10.0.0.0/8".to_string()),
        );
        rule2.insert(
            Value::String("action".to_string()),
            Value::String("allow".to_string()),
        );
        rules.push(Value::Mapping(rule2));

        args.insert(Value::String("rules".to_string()), Value::Sequence(rules));

        let config = PluginConfig {
            tag: Some("test_acl".to_string()),
            plugin_type: "query_acl".to_string(),
            args: Value::Mapping(args),
        };

        let plugin = QueryAclPlugin::init(&config).unwrap();
        let acl = plugin.as_any().downcast_ref::<QueryAclPlugin>().unwrap();

        // Test that rules were loaded correctly
        assert_eq!(
            acl.evaluate(&"192.168.1.1".parse().unwrap()),
            AclAction::Allow
        );
        assert_eq!(acl.evaluate(&"10.0.0.1".parse().unwrap()), AclAction::Allow);
        assert_eq!(acl.evaluate(&"1.2.3.4".parse().unwrap()), AclAction::Deny);
    }

    #[test]
    fn test_acl_plugin_init_deny_list() {
        use crate::config::types::PluginConfig;
        use serde_yaml::{Mapping, Value};

        let mut args = Mapping::new();
        args.insert(
            Value::String("default".to_string()),
            Value::String("allow".to_string()),
        );

        let mut rules = Vec::new();
        let mut rule = Mapping::new();
        rule.insert(
            Value::String("network".to_string()),
            Value::String("192.168.100.0/24".to_string()),
        );
        rule.insert(
            Value::String("action".to_string()),
            Value::String("deny".to_string()),
        );
        rules.push(Value::Mapping(rule));

        args.insert(Value::String("rules".to_string()), Value::Sequence(rules));

        let config = PluginConfig {
            tag: Some("test_acl".to_string()),
            plugin_type: "query_acl".to_string(),
            args: Value::Mapping(args),
        };

        let plugin = QueryAclPlugin::init(&config).unwrap();
        let acl = plugin.as_any().downcast_ref::<QueryAclPlugin>().unwrap();

        // Test that rules were loaded correctly
        assert_eq!(
            acl.evaluate(&"192.168.100.50".parse().unwrap()),
            AclAction::Deny
        );
        assert_eq!(
            acl.evaluate(&"192.168.1.1".parse().unwrap()),
            AclAction::Allow
        );
        assert_eq!(acl.evaluate(&"1.2.3.4".parse().unwrap()), AclAction::Allow);
    }
}
