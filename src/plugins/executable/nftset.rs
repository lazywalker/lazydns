//! NftSet executable plugin.
//!
//! This plugin inspects DNS response answers and emits nftables set
//! entries (CIDR prefixes) for A/AAAA records. On Linux it will try to
//! call the `nft` command to add elements to the configured set; on
//! non-Linux platforms it records additions in the request `Context`
//! metadata under `"nftset_added_v4"` and `"nftset_added_v6"`.
//!
//! Quick-setup accepts a compact shorthand used by some configurations:
//! `"<family>,<table>,<set>,<addr_type>,<mask> ..."` (max two fields).
//! Example: `"inet,my_table,my_set,ipv4_addr,24"`.
//!
//! Note: `SetArgs` contains optional table family and table names which
//! are used when attempting to run `nft` with the configured parameters.
use crate::dns::RData;
use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use tracing::info;

const PLUGIN_NFTSET_IDENTIFIER: &str = "nftset";

#[derive(Debug, Deserialize, Clone)]
pub struct NftSetArgs {
    /// Optional configuration for IPv4 elements (table, set and mask).
    pub ipv4: Option<SetArgs>,
    /// Optional configuration for IPv6 elements (table, set and mask).
    pub ipv6: Option<SetArgs>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SetArgs {
    /// Optional nftables table family (e.g. `inet`).
    pub table_family: Option<String>,
    /// Optional nftables table name.
    pub table: Option<String>,
    /// Optional nftables set name.
    pub set: Option<String>,
    /// Prefix length mask to apply when synthesizing CIDRs.
    pub mask: Option<u8>,
}
#[derive(RegisterExecPlugin)]
pub struct NftSetPlugin {
    args: NftSetArgs,
}

impl NftSetPlugin {
    pub fn new(args: NftSetArgs) -> Self {
        NftSetPlugin { args }
    }

    /// QuickSetup format: [{ip|ip6|inet},table_name,set_name,{ipv4_addr|ipv6_addr},mask] *2
    /// e.g. "inet,my_table,my_set,ipv4_addr,24 inet,my_table,my_set,ipv6_addr,48"
    pub fn quick_setup(s: &str) -> Result<Self> {
        let fs: Vec<&str> = s.split_whitespace().collect();
        if fs.len() > 2 {
            return Err(crate::Error::InvalidConfigValue {
                field: "nftset".to_string(),
                value: s.to_string(),
                reason: format!("expect no more than 2 fields, got {}", fs.len()),
            });
        }
        let mut args = NftSetArgs {
            ipv4: None,
            ipv6: None,
        };
        for args_str in fs {
            let ss: Vec<&str> = args_str.split(',').collect();
            if ss.len() != 5 {
                return Err(crate::Error::InvalidConfigValue {
                    field: "nftset".to_string(),
                    value: args_str.to_string(),
                    reason: format!("expect 5 comma-separated fields, got {}", ss.len()),
                });
            }
            let m: i32 = ss[4]
                .parse()
                .map_err(|e| crate::Error::InvalidConfigValue {
                    field: "mask".to_string(),
                    value: ss[4].to_string(),
                    reason: format!("{}", e),
                })?;
            let sa = SetArgs {
                table_family: Some(ss[0].to_string()),
                table: Some(ss[1].to_string()),
                set: Some(ss[2].to_string()),
                mask: Some(m as u8),
            };
            match ss[3] {
                "ipv4_addr" => args.ipv4 = Some(sa),
                "ipv6_addr" => args.ipv6 = Some(sa),
                other => {
                    return Err(crate::Error::InvalidAddress {
                        input: format!("unsupported ip type: {}", other),
                    });
                }
            }
        }
        Ok(NftSetPlugin::new(args))
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

impl fmt::Debug for NftSetPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NftSetPlugin").finish()
    }
}

#[async_trait]
impl Plugin for NftSetPlugin {
    fn name(&self) -> &str {
        PLUGIN_NFTSET_IDENTIFIER
    }

    fn aliases() -> &'static [&'static str] {
        // allow "nftset" as the canonical name
        &["nftset"]
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        if let Some(resp) = ctx.response() {
            let mut added_v4 = Vec::new();
            let mut added_v6 = Vec::new();
            for rr in resp.answers() {
                match rr.rdata() {
                    RData::A(ipv4) => {
                        if let Some(sa) = &self.args.ipv4
                            && let Some(set_name) = &sa.set
                        {
                            let mask = sa.mask.unwrap_or(24);
                            let prefix = Self::make_v4_prefix(ipv4, mask);
                            info!(set = %set_name, cidr = %prefix, "nftset add");
                            added_v4.push((set_name.clone(), prefix));
                        }
                    }
                    RData::AAAA(ipv6) => {
                        if let Some(sa) = &self.args.ipv6
                            && let Some(set_name) = &sa.set
                        {
                            let mask = sa.mask.unwrap_or(48);
                            let prefix = Self::make_v6_prefix(ipv6, mask);
                            info!(set = %set_name, cidr = %prefix, "nftset add");
                            added_v6.push((set_name.clone(), prefix));
                        }
                    }
                    _ => {}
                }
            }
            // On Linux, try to apply to system nftables; otherwise keep metadata.
            #[cfg(target_os = "linux")]
            {
                use std::process::Command;
                for (set_name, prefix) in &added_v4 {
                    if let Some(sa) = &self.args.ipv4
                        && let (Some(table_family), Some(table)) = (&sa.table_family, &sa.table)
                    {
                        let status = Command::new("nft")
                            .args([
                                "add",
                                "element",
                                table_family.as_str(),
                                table.as_str(),
                                set_name.as_str(),
                                "{",
                                prefix.as_str(),
                                "}",
                            ])
                            .status();
                        match status {
                            Ok(s) if s.success() => {}
                            Ok(s) => {
                                tracing::warn!(table = %table, set = %set_name, prefix = %prefix, status = ?s, "nft command returned non-zero exit status, continuing");
                            }
                            Err(e) => {
                                tracing::warn!(table = %table, set = %set_name, prefix = %prefix, error = %e, "failed to spawn nft command, continuing");
                            }
                        }
                    }
                }
                for (set_name, prefix) in &added_v6 {
                    if let Some(sa) = &self.args.ipv6
                        && let (Some(table_family), Some(table)) = (&sa.table_family, &sa.table)
                    {
                        let status = Command::new("nft")
                            .args([
                                "add",
                                "element",
                                table_family.as_str(),
                                table.as_str(),
                                set_name.as_str(),
                                "{",
                                prefix.as_str(),
                                "}",
                            ])
                            .status();
                        match status {
                            Ok(s) if s.success() => {}
                            Ok(s) => {
                                tracing::warn!(table = %table, set = %set_name, prefix = %prefix, status = ?s, "nft command returned non-zero exit status, continuing");
                            }
                            Err(e) => {
                                tracing::warn!(table = %table, set = %set_name, prefix = %prefix, error = %e, "failed to spawn nft command, continuing");
                            }
                        }
                    }
                }
            }

            if !added_v4.is_empty() {
                ctx.set_metadata("nftset_added_v4", added_v4);
            }
            if !added_v6.is_empty() {
                ctx.set_metadata("nftset_added_v6", added_v6);
            }
        }
        Ok(())
    }
}

impl ExecPlugin for NftSetPlugin {
    /// Parse a quick configuration string for nftset plugin.
    ///
    /// The exec_str should be in the format: "`<family>,<table>,<set>,<addr_type>,<mask> ...`"
    /// Examples: "inet,my_table,my_set,ipv4_addr,24,inet,my_table,my_set6,ipv6_addr,48"
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != PLUGIN_NFTSET_IDENTIFIER {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'nftset'",
                prefix
            )));
        }

        // Convert comma-separated format to space-separated format expected by quick_setup
        // "a,b,c,d,e,f,g,h,i,j" -> "a,b,c,d,e f,g,h,i,j"
        let parts: Vec<&str> = exec_str.split(',').collect();
        if !parts.len().is_multiple_of(5) {
            return Err(crate::Error::Config(format!(
                "Invalid nftset arguments: expected multiples of 5 comma-separated values, got {}",
                parts.len()
            )));
        }

        let mut space_separated = Vec::new();
        for chunk in parts.chunks(5) {
            space_separated.push(chunk.join(","));
        }
        let space_separated = space_separated.join(" ");

        let plugin = NftSetPlugin::quick_setup(&space_separated)?;
        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::types::{RecordClass, RecordType};
    use crate::dns::{Message, ResourceRecord};

    #[test]
    fn test_nftset_new() {
        let args = NftSetArgs {
            ipv4: None,
            ipv6: None,
        };
        let plugin = NftSetPlugin::new(args);
        assert_eq!(plugin.name(), "nftset");
    }

    #[test]
    fn test_nftset_debug() {
        let args = NftSetArgs {
            ipv4: Some(SetArgs {
                table_family: Some("inet".into()),
                table: Some("t".into()),
                set: Some("s".into()),
                mask: Some(24),
            }),
            ipv6: None,
        };
        let plugin = NftSetPlugin::new(args);
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("NftSetPlugin"));
    }

    #[test]
    fn test_nftset_quick_setup_ipv4() {
        let plugin = NftSetPlugin::quick_setup("inet,my_table,my_set,ipv4_addr,24").unwrap();
        assert_eq!(plugin.name(), "nftset");
        assert!(plugin.args.ipv4.is_some());
        assert!(plugin.args.ipv6.is_none());
    }

    #[test]
    fn test_nftset_quick_setup_ipv6() {
        let plugin = NftSetPlugin::quick_setup("inet,my_table,my_set,ipv6_addr,64").unwrap();
        assert_eq!(plugin.name(), "nftset");
        assert!(plugin.args.ipv4.is_none());
        assert!(plugin.args.ipv6.is_some());
    }

    #[test]
    fn test_nftset_quick_setup_both() {
        let plugin =
            NftSetPlugin::quick_setup("inet,t1,s1,ipv4_addr,24 inet,t2,s2,ipv6_addr,64").unwrap();
        assert!(plugin.args.ipv4.is_some());
        assert!(plugin.args.ipv6.is_some());
    }

    #[test]
    fn test_nftset_quick_setup_too_many_fields() {
        let result = NftSetPlugin::quick_setup("a,b,c,d,24 e,f,g,h,32 i,j,k,l,48");
        assert!(result.is_err());
    }

    #[test]
    fn test_nftset_quick_setup_wrong_parts() {
        let result = NftSetPlugin::quick_setup("inet,my_table,my_set");
        assert!(result.is_err());
    }

    #[test]
    fn test_nftset_quick_setup_invalid_mask() {
        let result = NftSetPlugin::quick_setup("inet,t,s,ipv4_addr,notanumber");
        assert!(result.is_err());
    }

    #[test]
    fn test_nftset_quick_setup_invalid_addr_type() {
        let result = NftSetPlugin::quick_setup("inet,t,s,invalid_type,24");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_nftset_v4_add() {
        let sa = SetArgs {
            table_family: Some("inet".into()),
            table: Some("t".into()),
            set: Some("s".into()),
            mask: Some(24),
        };
        let args = NftSetArgs {
            ipv4: Some(sa),
            ipv6: None,
        };
        let plugin = NftSetPlugin::new(args);
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
            .get_metadata::<Vec<(String, String)>>("nftset_added_v4")
            .unwrap();
        assert_eq!(added.len(), 1);
    }

    /// Regression: mask=0 previously triggered `!0u32 << 32`, a shift-overflow
    /// panic in debug / wrong mask in release. A /0 prefix must be "0.0.0.0/0".
    #[test]
    fn test_make_v4_prefix_mask_zero() {
        let ip: Ipv4Addr = "192.0.2.5".parse().unwrap();
        assert_eq!(NftSetPlugin::make_v4_prefix(&ip, 0), "0.0.0.0/0");
        assert_eq!(NftSetPlugin::make_v4_prefix(&ip, 24), "192.0.2.0/24");
        assert_eq!(NftSetPlugin::make_v4_prefix(&ip, 32), "192.0.2.5/32");
    }

    #[tokio::test]
    async fn test_nftset_v6_add() {
        let sa = SetArgs {
            table_family: Some("inet".into()),
            table: Some("t".into()),
            set: Some("s".into()),
            mask: Some(64),
        };
        let args = NftSetArgs {
            ipv4: None,
            ipv6: Some(sa),
        };
        let plugin = NftSetPlugin::new(args);
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
            .get_metadata::<Vec<(String, String)>>("nftset_added_v6")
            .unwrap();
        assert_eq!(added.len(), 1);
    }

    #[tokio::test]
    async fn test_nftset_no_response() {
        let args = NftSetArgs {
            ipv4: Some(SetArgs {
                table_family: None,
                table: None,
                set: None,
                mask: Some(24),
            }),
            ipv6: None,
        };
        let plugin = NftSetPlugin::new(args);
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        // No response set
        plugin.execute(&mut ctx).await.unwrap();
        // Should not add any entries
        let added = ctx.get_metadata::<Vec<(String, String)>>("nftset_added_v4");
        assert!(added.is_none());
    }

    #[tokio::test]
    async fn test_nftset_no_matching_records() {
        let args = NftSetArgs {
            ipv4: Some(SetArgs {
                table_family: None,
                table: None,
                set: None,
                mask: Some(24),
            }),
            ipv6: None,
        };
        let plugin = NftSetPlugin::new(args);
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
        // No matching A/AAAA records
        let added = ctx.get_metadata::<Vec<(String, String)>>("nftset_added_v4");
        assert!(added.is_none());
    }

    #[tokio::test]
    async fn test_nftset_disabled_config() {
        let args = NftSetArgs {
            ipv4: None,
            ipv6: None,
        };
        let plugin = NftSetPlugin::new(args);
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
        // No config for ipv4, should not add
        let added = ctx.get_metadata::<Vec<(String, String)>>("nftset_added_v4");
        assert!(added.is_none());
    }

    #[test]
    fn test_nftset_exec_plugin() {
        // Test ExecPlugin quick_setup
        let plugin = NftSetPlugin::quick_setup("inet,my_table,my_set,ipv4_addr,24").unwrap();
        assert_eq!(plugin.name(), "nftset");
    }

    #[test]
    fn test_exec_plugin_quick_setup_wrong_prefix() {
        let result = <NftSetPlugin as ExecPlugin>::quick_setup("wrong", "inet,t,s,ipv4_addr,24");
        assert!(result.is_err());
    }

    #[test]
    fn test_exec_plugin_quick_setup_both_families() {
        let result = <NftSetPlugin as ExecPlugin>::quick_setup(
            "nftset",
            "inet,t1,s1,ipv4_addr,24,inet,t2,s2,ipv6_addr,64",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_exec_plugin_quick_setup_invalid_count() {
        let result = <NftSetPlugin as ExecPlugin>::quick_setup(
            "nftset",
            "inet,t,s,ipv4_addr", // Only 4 parts
        );
        assert!(result.is_err());
    }
}
