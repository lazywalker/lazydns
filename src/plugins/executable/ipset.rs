//! IpSet executable plugin.
//!
//! This plugin inspects DNS response answers and emits ipset entries
//! (CIDR prefixes) for A/AAAA records. On Linux it will attempt to run
//! the `ipset` command to add the computed prefixes; on other platforms
//! it simply records the additions in the request `Context` metadata
//! under the key `"ipset_added"` as a `Vec<(String, String)>` with
//! (set_name, cidr).
//!
//! Quick-setup accepts a compact shorthand used by some configurations:
//! `"<set_name>,inet,<mask> <set_name6>,inet6,<mask>"` (max two fields).
//!
//! Example metadata after execution: `ipset_added = vec![("myset".into(), "192.0.2.0/24".into())]`.
//!
//! Note: this file contains only the executable wrapper; logic is small
//! and intended to be fast and dependency-free.
use crate::dns::RData;
use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use tracing::info;

const PLUGIN_IPSET_IDENTIFIER: &str = "ipset";

#[derive(Debug, Deserialize, Clone)]
pub struct IpSetArgs {
    /// Optional ipset name for IPv4 entries (e.g. "my_ipset_v4").
    pub set_name4: Option<String>,
    /// Optional ipset name for IPv6 entries (e.g. "my_ipset_v6").
    pub set_name6: Option<String>,
    /// IPv4 prefix length to use when converting A records to CIDR.
    /// Defaults to `Some(24)` in `Default` if not specified.
    pub mask4: Option<u8>,
    /// IPv6 prefix length to use when converting AAAA records to CIDR.
    /// Defaults to `Some(32)` in `Default` if not specified.
    pub mask6: Option<u8>,
}

impl Default for IpSetArgs {
    fn default() -> Self {
        Self {
            set_name4: None,
            set_name6: None,
            mask4: Some(24),
            mask6: Some(32),
        }
    }
}

#[derive(RegisterExecPlugin)]
pub struct IpSetPlugin {
    args: IpSetArgs,
}

impl IpSetPlugin {
    pub fn new(args: IpSetArgs) -> Self {
        IpSetPlugin { args }
    }

    /// QuickSetup format: [set_name,{inet|inet6},mask] *2
    /// e.g. "my_set,inet,24 my_set6,inet6,48"
    pub fn quick_setup(s: &str) -> Result<Self> {
        let fs: Vec<&str> = s.split_whitespace().collect();
        if fs.len() > 2 {
            return Err(crate::Error::Other(format!(
                "expect no more than 2 fields, got {}",
                fs.len()
            )));
        }
        let mut args = IpSetArgs::default();
        for args_str in fs {
            let ss: Vec<&str> = args_str.split(',').collect();
            if ss.len() != 3 {
                return Err(crate::Error::Other(format!(
                    "invalid args, expect 3 fields, got {}",
                    ss.len()
                )));
            }
            let m: i32 = ss[2]
                .parse()
                .map_err(|e| crate::Error::Other(format!("invalid mask, {}", e)))?;
            match ss[1] {
                "inet" => {
                    args.mask4 = Some(m as u8);
                    args.set_name4 = Some(ss[0].to_string());
                }
                "inet6" => {
                    args.mask6 = Some(m as u8);
                    args.set_name6 = Some(ss[0].to_string());
                }
                other => {
                    return Err(crate::Error::Other(format!(
                        "invalid set family, {}",
                        other
                    )));
                }
            }
        }
        Ok(IpSetPlugin::new(args))
    }

    fn make_v4_prefix(ip: &Ipv4Addr, mask: u8) -> String {
        let mask = mask.min(32);
        // Guard against shift overflow: `!0u32 << 32` is UB/panic on a 32-bit
        // shift, and a /0 network is "0.0.0.0/0" regardless of the input IP.
        if mask == 0 {
            return "0.0.0.0/0".to_string();
        }
        let ip_u32 = u32::from_be_bytes(ip.octets());
        let net = ip_u32 & (!0u32 << (32 - mask as u32));
        let bytes = net.to_be_bytes();
        // Return canonical CIDR like "192.0.2.0/24" (no spaces).
        format!(
            "{}.{}.{}.{} /{}",
            bytes[0], bytes[1], bytes[2], bytes[3], mask
        )
        .replace(' ', "")
    }

    fn make_v6_prefix(ip: &Ipv6Addr, mask: u8) -> String {
        let mask = mask.min(128);
        let mut bytes = ip.octets();
        let mut bits = mask as usize;
        for b in bytes.iter_mut() {
            if bits >= 8 {
                bits -= 8;
                continue;
            }
            if bits == 0 {
                *b = 0;
            } else {
                // keep upper `bits` bits
                let keep = (!0u8) << (8 - bits);
                *b &= keep;
                bits = 0;
            }
        }
        use std::net::Ipv6Addr;
        let addr = Ipv6Addr::from(bytes);
        // Return canonical IPv6 CIDR like "2001:db8::/48".
        format!("{}/{}", addr, mask)
    }
}

impl fmt::Debug for IpSetPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IpSetPlugin").finish()
    }
}

#[async_trait]
impl Plugin for IpSetPlugin {
    fn name(&self) -> &str {
        PLUGIN_IPSET_IDENTIFIER
    }

    fn aliases() -> &'static [&'static str] {
        // allow "ipset" as the canonical name
        &["ipset"]
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        if let Some(resp) = ctx.response() {
            let mut to_add: Vec<(String, String)> = ctx
                .get_metadata::<Vec<(String, String)>>("ipset_added")
                .cloned()
                .unwrap_or_default();
            for rr in resp.answers() {
                match rr.rdata() {
                    RData::A(ipv4) => {
                        if let Some(name) = &self.args.set_name4 {
                            let mask = self.args.mask4.unwrap_or(24);
                            let prefix = Self::make_v4_prefix(ipv4, mask);
                            info!(set = %name, cidr = %prefix, "ipset add");
                            to_add.push((name.clone(), prefix));
                        }
                    }
                    RData::AAAA(ipv6) => {
                        if let Some(name) = &self.args.set_name6 {
                            let mask = self.args.mask6.unwrap_or(32);
                            let prefix = Self::make_v6_prefix(ipv6, mask);
                            info!(set = %name, cidr = %prefix, "ipset add");
                            to_add.push((name.clone(), prefix));
                        }
                    }
                    _ => {}
                }
            }

            // On Linux try to apply to system ipset; on other platforms just record metadata.
            #[cfg(target_os = "linux")]
            {
                use std::process::Command;
                for (set_name, cidr) in &to_add {
                    // ipset add <set> <cidr> -exist
                    let status = Command::new("ipset")
                        .args(["add", set_name.as_str(), cidr.as_str(), "-exist"])
                        .status();
                    match status {
                        Ok(s) if s.success() => {}
                        Ok(s) => {
                            tracing::warn!(set = %set_name, cidr = %cidr, status = ?s, "ipset command returned non-zero exit status, continuing");
                        }
                        Err(e) => {
                            tracing::warn!(set = %set_name, cidr = %cidr, error = %e, "failed to spawn ipset command, continuing");
                        }
                    }
                }
            }

            // Always store metadata for visibility/tests
            if !to_add.is_empty() {
                ctx.set_metadata("ipset_added", to_add);
            }
        }

        Ok(())
    }
}

impl ExecPlugin for IpSetPlugin {
    /// Parse a quick configuration string for ipset plugin.
    ///
    /// The exec_str should be in the format: "`<set_name>,inet,<mask>,<set_name6>,inet6,<mask>`"
    /// Examples: "my_set,inet,24,my_set6,inet6,48"
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != PLUGIN_IPSET_IDENTIFIER {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'ipset'",
                prefix
            )));
        }

        // Convert comma-separated format to space-separated format expected by quick_setup
        // "a,b,c,d,e,f" -> "a,b,c d,e,f"
        let parts: Vec<&str> = exec_str.split(',').collect();
        if !parts.len().is_multiple_of(3) {
            return Err(crate::Error::Config(format!(
                "Invalid ipset arguments: expected multiples of 3 comma-separated values, got {}",
                parts.len()
            )));
        }

        let mut space_separated = Vec::new();
        for chunk in parts.chunks(3) {
            space_separated.push(chunk.join(","));
        }
        let space_separated = space_separated.join(" ");

        let plugin = IpSetPlugin::quick_setup(&space_separated)?;
        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::types::{RecordClass, RecordType};
    use crate::dns::{Message, ResourceRecord};

    #[test]
    fn test_ipset_args_default() {
        let args = IpSetArgs::default();
        assert!(args.set_name4.is_none());
        assert!(args.set_name6.is_none());
        assert_eq!(args.mask4, Some(24));
        assert_eq!(args.mask6, Some(32));
    }

    #[test]
    fn test_ipset_new() {
        let args = IpSetArgs {
            set_name4: Some("test".into()),
            set_name6: None,
            mask4: Some(24),
            mask6: None,
        };
        let plugin = IpSetPlugin::new(args);
        assert_eq!(plugin.name(), "ipset");
    }

    #[test]
    fn test_ipset_debug() {
        let args = IpSetArgs::default();
        let plugin = IpSetPlugin::new(args);
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("IpSetPlugin"));
    }

    #[test]
    fn test_ipset_quick_setup_ipv4() {
        let plugin = IpSetPlugin::quick_setup("myset,inet,24").unwrap();
        assert!(plugin.args.set_name4.is_some());
        assert!(plugin.args.set_name6.is_none());
    }

    #[test]
    fn test_ipset_quick_setup_ipv6() {
        let plugin = IpSetPlugin::quick_setup("myset,inet6,64").unwrap();
        assert!(plugin.args.set_name4.is_none());
        assert!(plugin.args.set_name6.is_some());
    }

    #[test]
    fn test_ipset_quick_setup_both() {
        let plugin = IpSetPlugin::quick_setup("set4,inet,24 set6,inet6,64").unwrap();
        assert!(plugin.args.set_name4.is_some());
        assert!(plugin.args.set_name6.is_some());
    }

    #[test]
    fn test_ipset_quick_setup_invalid_count() {
        let result = IpSetPlugin::quick_setup("set4,inet");
        assert!(result.is_err());
    }

    #[test]
    fn test_ipset_quick_setup_too_many() {
        let result = IpSetPlugin::quick_setup("a,b,c d,e,f g,h,i");
        assert!(result.is_err());
    }

    #[test]
    fn test_ipset_quick_setup_invalid_mask() {
        let result = IpSetPlugin::quick_setup("set,inet,notanumber");
        assert!(result.is_err());
    }

    #[test]
    fn test_ipset_quick_setup_invalid_family() {
        let result = IpSetPlugin::quick_setup("set,invalid,24");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ipset_v4_prefix() {
        let args = IpSetArgs {
            set_name4: Some("myset".into()),
            set_name6: None,
            mask4: Some(24),
            mask6: None,
        };
        let plugin = IpSetPlugin::new(args);
        let mut msg = Message::new();
        msg.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            crate::dns::RData::A("192.0.2.5".parse().unwrap()),
        ));
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        let added = ctx
            .get_metadata::<Vec<(String, String)>>("ipset_added")
            .unwrap();
        assert_eq!(added.len(), 1);
    }

    /// Regression: mask=0 previously triggered `!0u32 << 32`, a shift-overflow
    /// panic in debug / wrong mask in release. A /0 prefix must be "0.0.0.0/0".
    #[test]
    fn test_make_v4_prefix_mask_zero() {
        let ip: Ipv4Addr = "192.0.2.5".parse().unwrap();
        assert_eq!(IpSetPlugin::make_v4_prefix(&ip, 0), "0.0.0.0/0");
        assert_eq!(IpSetPlugin::make_v4_prefix(&ip, 24), "192.0.2.0/24");
        assert_eq!(IpSetPlugin::make_v4_prefix(&ip, 32), "192.0.2.5/32");
    }

    #[tokio::test]
    async fn test_ipset_v6_prefix() {
        let args = IpSetArgs {
            set_name4: None,
            set_name6: Some("myset6".into()),
            mask4: None,
            mask6: Some(64),
        };
        let plugin = IpSetPlugin::new(args);
        let mut msg = Message::new();
        msg.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            crate::dns::RData::AAAA("2001:db8::1".parse().unwrap()),
        ));
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        let added = ctx
            .get_metadata::<Vec<(String, String)>>("ipset_added")
            .unwrap();
        assert_eq!(added.len(), 1);
    }

    #[tokio::test]
    async fn test_ipset_no_response() {
        let args = IpSetArgs {
            set_name4: Some("myset".into()),
            set_name6: None,
            mask4: Some(24),
            mask6: None,
        };
        let plugin = IpSetPlugin::new(args);
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        // No response set
        plugin.execute(&mut ctx).await.unwrap();
        let added = ctx.get_metadata::<Vec<(String, String)>>("ipset_added");
        assert!(added.is_none());
    }

    #[tokio::test]
    async fn test_ipset_no_matching_records() {
        let args = IpSetArgs {
            set_name4: Some("myset".into()),
            set_name6: None,
            mask4: Some(24),
            mask6: None,
        };
        let plugin = IpSetPlugin::new(args);
        let mut msg = Message::new();
        msg.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::CNAME,
            RecordClass::IN,
            300,
            crate::dns::RData::CNAME("target.com".into()),
        ));
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        let added = ctx.get_metadata::<Vec<(String, String)>>("ipset_added");
        assert!(added.is_none());
    }

    #[tokio::test]
    async fn test_ipset_disabled_config() {
        let args = IpSetArgs {
            set_name4: None,
            set_name6: None,
            mask4: Some(24),
            mask6: Some(64),
        };
        let plugin = IpSetPlugin::new(args);
        let mut msg = Message::new();
        msg.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            crate::dns::RData::A("192.0.2.1".parse().unwrap()),
        ));
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        ctx.set_response(Some(msg));
        plugin.execute(&mut ctx).await.unwrap();
        // No set names configured
        let added = ctx.get_metadata::<Vec<(String, String)>>("ipset_added");
        assert!(added.is_none());
    }

    #[test]
    fn test_ipset_exec_plugin() {
        // Test ExecPlugin quick_setup
        let plugin = IpSetPlugin::quick_setup("test_set,inet,24").unwrap();
        assert_eq!(plugin.name(), "ipset");
    }

    #[test]
    fn test_exec_plugin_wrong_prefix() {
        let result = <IpSetPlugin as ExecPlugin>::quick_setup("wrong", "set,inet,24");
        assert!(result.is_err());
    }

    #[test]
    fn test_exec_plugin_both_families() {
        let result =
            <IpSetPlugin as ExecPlugin>::quick_setup("ipset", "set4,inet,24,set6,inet6,64");
        assert!(result.is_ok());
    }

    #[test]
    fn test_exec_plugin_invalid_count() {
        let result = <IpSetPlugin as ExecPlugin>::quick_setup(
            "ipset", "set,inet", // Only 2 parts
        );
        assert!(result.is_err());
    }
}
