//! RouterOS address list plugin
//!
//! Adds matched IPs to RouterOS address lists (stub implementation)

use crate::config::PluginConfig;
use crate::plugin::{Context, Plugin};
use crate::{RegisterPlugin, Result};
use async_trait::async_trait;
use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::json;
use std::fmt;
use std::net::IpAddr;
use std::time::Duration;
use tracing::{debug, warn};

// Auto-register using the register macro

/// Plugin that manages RouterOS address lists
///
/// This is a stub implementation that logs address list operations.
/// Full implementation would require RouterOS API integration.
///
/// # Example
///
/// ```rust
/// use lazydns::plugins::executable::RosAddrlistPlugin;
///
/// let plugin = RosAddrlistPlugin::new("blocked_ips");
/// ```
#[derive(RegisterPlugin)]
pub struct RosAddrlistPlugin {
    /// Address list name in RouterOS
    list_name: String,
    /// Whether to add IPs from query responses
    track_responses: bool,
    /// Optional HTTP endpoint to call for adding IPs (such as a helper service)
    server: Option<String>,
    /// Optional basic auth user
    user: Option<String>,
    /// Optional basic auth password
    passwd: Option<String>,
    /// IPv4 mask to apply when adding (such as 24)
    mask4: Option<u8>,
    /// IPv6 mask to apply when adding (such as 32)
    mask6: Option<u8>,
    // client: Option<Client>,
}

impl RosAddrlistPlugin {
    /// Create a new RouterOS address list plugin
    pub fn new(list_name: impl Into<String>) -> Self {
        Self {
            list_name: list_name.into(),
            track_responses: true,
            server: None,
            user: None,
            passwd: None,
            mask4: None,
            mask6: None,
        }
    }

    /// Set whether to track response IPs
    pub fn track_responses(mut self, enabled: bool) -> Self {
        self.track_responses = enabled;
        self
    }

    /// Set an HTTP helper server to notify when adding addresses
    pub fn with_server(mut self, server: impl Into<String>) -> Self {
        self.server = Some(server.into());
        self
    }

    /// Set basic auth credentials for the HTTP helper
    pub fn with_auth(mut self, user: impl Into<String>, passwd: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self.passwd = Some(passwd.into());
        self
    }

    /// Configure masks for IPv4 and IPv6 when adding single IPs
    pub fn with_masks(mut self, mask4: Option<u8>, mask6: Option<u8>) -> Self {
        self.mask4 = mask4;
        self.mask6 = mask6;
        self
    }

    /// Set IPv4 mask
    pub fn with_mask4(mut self, mask4: u8) -> Self {
        self.mask4 = Some(mask4);
        self
    }

    /// Set IPv6 mask
    pub fn with_mask6(mut self, mask6: u8) -> Self {
        self.mask6 = Some(mask6);
        self
    }

    /// Extract IPs from response
    fn extract_ips(&self, ctx: &Context) -> Vec<IpAddr> {
        let mut ips = Vec::new();

        if let Some(response) = ctx.response() {
            for answer in response.answers() {
                // Extract IP addresses from A and AAAA records
                if let Some(ip) = self.extract_ip_from_rdata(answer.rdata()) {
                    ips.push(ip);
                }
            }
        }

        ips
    }

    /// Extract IP from RData (simplified)
    fn extract_ip_from_rdata(&self, rdata: &crate::dns::RData) -> Option<IpAddr> {
        use crate::dns::RData;

        match rdata {
            RData::A(addr) => Some(IpAddr::V4(*addr)),
            RData::AAAA(addr) => Some(IpAddr::V6(*addr)),
            _ => None,
        }
    }

    async fn notify_server(&self, ips: &[IpAddr], domain: &str) -> Result<()> {
        // Build a fresh client with TLS accept invalid certs and 2s timeout
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|e| crate::Error::Other(format!("failed build http client: {}", e)))?;

        if let Some(srv) = &self.server {
            for ip in ips {
                let v6 = matches!(ip, IpAddr::V6(_));
                let kind = if v6 { "ipv6" } else { "ip" };
                let router_url = format!(
                    "{}/rest/{}/firewall/address-list/add",
                    srv.trim_end_matches('/'),
                    kind
                );

                let payload = json!({
                    "address": ip.to_string(),
                    "list": self.list_name,
                    "comment": format!("[lazydns] domain: {}", domain),
                });

                let mut req = client.post(&router_url).json(&payload);
                if let (Some(user), Some(pass)) = (&self.user, &self.passwd) {
                    req = req.basic_auth(user, Some(pass));
                }

                let resp = req
                    .send()
                    .await
                    .map_err(|e| crate::Error::Other(format!("http request failed: {}", e)))?;
                match resp.status() {
                    StatusCode::OK => {
                        debug!(ip = %ip, list = %self.list_name, domain = %domain, "added ip to ros addrlist")
                    }
                    StatusCode::BAD_REQUEST => {
                        debug!(ip = %ip, list = %self.list_name, domain = %domain, "likely ip already exists")
                    }
                    StatusCode::UNAUTHORIZED => {
                        return Err(crate::Error::Other(format!(
                            "unauthorized when adding {}",
                            ip
                        )));
                    }
                    StatusCode::INTERNAL_SERVER_ERROR => {
                        return Err(crate::Error::Other(format!(
                            "internal server error when adding {}",
                            ip
                        )));
                    }
                    s => {
                        return Err(crate::Error::Other(format!(
                            "unexpected status code {} when adding {}",
                            s, ip
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

impl fmt::Debug for RosAddrlistPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RosAddrListPlugin")
            .field("list_name", &self.list_name)
            .field("track_responses", &self.track_responses)
            .field("server", &self.server)
            .field("user", &self.user)
            .field("mask4", &self.mask4)
            .field("mask6", &self.mask6)
            .finish()
    }
}

#[async_trait]
impl Plugin for RosAddrlistPlugin {
    fn name(&self) -> &str {
        "ros_addrlist"
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        if !self.track_responses {
            return Ok(());
        }

        let ips = self.extract_ips(ctx);

        if !ips.is_empty() {
            // If server is configured, notify helper endpoint (include query domain in comment)
            let domain = if let Some(question) = ctx.request().questions().first() {
                question.qname().trim_end_matches('.').to_string()
            } else {
                "".to_string()
            };

            debug!(
                list_name = %self.list_name,
                domain = %domain,
                ip_count = ips.len(),
                ips = ?ips,
                "RouterOS address list: add IPs"
            );

            if let Err(e) = self.notify_server(&ips, &domain).await {
                warn!(error = %e, domain = %domain, "Failed to notify RouterOS helper server");
            }
        }

        Ok(())
    }

    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>> {
        let args = config.effective_args();

        let addrlist = args
            .get("addrlist")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let mut plugin = RosAddrlistPlugin::new(addrlist);

        if let Some(server) = args.get("server").and_then(|v| v.as_str()) {
            plugin = plugin.with_server(server.to_string());
        }

        if let Some(user) = args.get("user").and_then(|v| v.as_str())
            && let Some(pass) = args.get("passwd").and_then(|v| v.as_str())
        {
            plugin = plugin.with_auth(user.to_string(), pass.to_string());
        }

        if let Some(mask4) = args.get("mask4").and_then(|v| v.as_i64()) {
            plugin = plugin.with_mask4(mask4 as u8);
        }

        if let Some(mask6) = args.get("mask6").and_then(|v| v.as_i64()) {
            plugin = plugin.with_mask6(mask6 as u8);
        }

        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::types::{RecordClass, RecordType};
    use crate::dns::{Message, RData, ResourceRecord};

    #[test]
    fn test_ros_addrlist_new() {
        let plugin = RosAddrlistPlugin::new("test_list");
        assert_eq!(plugin.list_name, "test_list");
        assert!(plugin.track_responses);
        assert!(plugin.server.is_none());
        assert!(plugin.user.is_none());
        assert!(plugin.passwd.is_none());
        assert!(plugin.mask4.is_none());
        assert!(plugin.mask6.is_none());
    }

    #[test]
    fn test_ros_addrlist_builder_pattern() {
        let plugin = RosAddrlistPlugin::new("blocked")
            .track_responses(false)
            .with_server("http://localhost:8080")
            .with_auth("admin", "password")
            .with_masks(Some(24), Some(64))
            .with_mask4(16)
            .with_mask6(48);

        assert_eq!(plugin.list_name, "blocked");
        assert!(!plugin.track_responses);
        assert_eq!(plugin.server.as_deref(), Some("http://localhost:8080"));
        assert_eq!(plugin.user.as_deref(), Some("admin"));
        assert_eq!(plugin.passwd.as_deref(), Some("password"));
        assert_eq!(plugin.mask4, Some(16));
        assert_eq!(plugin.mask6, Some(48));
    }

    #[test]
    fn test_ros_addrlist_debug() {
        let plugin = RosAddrlistPlugin::new("test");
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("RosAddrListPlugin"));
        assert!(debug_str.contains("test"));
    }

    #[tokio::test]
    async fn test_ros_addrlist_extract_ips() {
        let plugin = RosAddrlistPlugin::new("test_list");

        let mut ctx = Context::new(Message::new());
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

        // Should extract IPs and log them
        plugin.execute(&mut ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_ros_addrlist_disabled() {
        let plugin = RosAddrlistPlugin::new("test_list").track_responses(false);

        let mut ctx = Context::new(Message::new());
        let mut response = Message::new();

        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A("192.0.2.1".parse().unwrap()),
        ));

        ctx.set_response(Some(response));

        // Should not process IPs
        plugin.execute(&mut ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_ros_addrlist_no_ips() {
        let plugin = RosAddrlistPlugin::new("test_list");

        let mut ctx = Context::new(Message::new());
        let mut response = Message::new();

        // Add non-IP record
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::CNAME,
            RecordClass::IN,
            300,
            RData::CNAME("target.example.com".to_string()),
        ));

        ctx.set_response(Some(response));

        // Should not extract any IPs
        plugin.execute(&mut ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_ros_addrlist_no_response() {
        let plugin = RosAddrlistPlugin::new("test_list");

        let mut ctx = Context::new(Message::new());
        // No response set

        // Should handle gracefully
        plugin.execute(&mut ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_ros_addrlist_empty_response() {
        let plugin = RosAddrlistPlugin::new("test_list");

        let mut ctx = Context::new(Message::new());
        let response = Message::new(); // Empty response with no answers
        ctx.set_response(Some(response));

        // Should handle gracefully
        plugin.execute(&mut ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_ros_addrlist_with_masks() {
        let plugin = RosAddrlistPlugin::new("test_list")
            .with_mask4(24)
            .with_mask6(64);

        let mut ctx = Context::new(Message::new());
        let mut response = Message::new();

        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A("192.0.2.100".parse().unwrap()),
        ));

        ctx.set_response(Some(response));

        // Should apply mask when extracting
        plugin.execute(&mut ctx).await.unwrap();
    }
}
