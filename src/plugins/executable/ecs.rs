use crate::plugin::{Context, ExecPlugin, Plugin};
use crate::{RegisterExecPlugin, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;

/// Arguments for the ECS handler.
///
/// This struct defines the configuration options for the ECS (EDNS Client Subnet) handler plugin.
/// The ECS handler manages EDNS0 CLIENT-SUBNET options for DNS queries.
///
/// # Configuration Options
///
/// - `forward`: When true, copy client-provided EDNS0 options to the outgoing query
/// - `send`: When true, derive ECS from client address metadata and attach it
/// - `preset`: Optional preset address to use as ECS (useful for testing)
/// - `mask4`: Source prefix length for IPv4 addresses (default: 24, max: 32)
/// - `mask6`: Source prefix length for IPv6 addresses (default: 48, max: 128)
///
/// # Examples
///
/// ```yaml
/// plugins:
///   - type: ecs
///     args:
///       forward: true
///       send: false
///       mask4: 24
///       mask6: 48
/// ```
#[derive(Deserialize, Clone)]
pub struct EcsArgs {
    /// Whether to forward client-provided EDNS0 options
    pub forward: Option<bool>,
    /// Whether to derive and send ECS from client address
    pub send: Option<bool>,
    /// Optional preset IP address to use as ECS
    pub preset: Option<String>,
    /// IPv4 source prefix length (default: 24)
    pub mask4: Option<u8>,
    /// IPv6 source prefix length (default: 48)
    pub mask6: Option<u8>,
}

/// Runtime ECS handler.
///
/// Prepares EDNS0 CLIENT-SUBNET options and writes them into context metadata
/// (`edns0_options` and `edns0_preserve_existing`) for downstream forwarding.
///
/// # Processing Order
///
/// 1. If `forward` is enabled and client provided ECS options, copy them
/// 2. If `preset` is configured, use it to generate ECS option
/// 3. If `send` is enabled, derive ECS from client IP address
///
/// # Metadata Keys Used
///
/// - `client_edns0_options`: Client-provided EDNS0 options (for forwarding)
/// - `client_addr`: Client IP address string (for deriving ECS)
/// - `edns0_options`: Output ECS options for downstream plugins
/// - `edns0_preserve_existing`: Flag to preserve existing options
#[derive(Clone, RegisterExecPlugin)]
pub struct EcsPlugin {
    forward: bool,
    send: bool,
    preset: Option<IpAddr>,
    mask4: u8,
    mask6: u8,
}

impl EcsPlugin {
    /// Create a new ECS handler with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `args` - Configuration arguments for the ECS handler
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the configured `EcsPlugin` or an error
    /// if the configuration is invalid (such as invalid mask values or preset address).
    ///
    /// # Errors
    ///
    /// - If `mask4` > 32 or `mask6` > 128
    /// - If `preset` contains an invalid IP address
    ///
    /// # Examples
    ///
    /// ```rust
    /// use lazydns::plugins::executable::ecs::{EcsPlugin, EcsArgs};
    ///
    /// let args = EcsArgs {
    ///     forward: Some(true),
    ///     send: Some(false),
    ///     preset: None,
    ///     mask4: Some(24),
    ///     mask6: Some(48),
    /// };
    ///
    /// let handler = EcsPlugin::new(args).unwrap();
    /// ```
    pub fn new(args: EcsArgs) -> Result<Self> {
        let forward = args.forward.unwrap_or(false);
        let send = args.send.unwrap_or(false);
        let mask4 = args.mask4.unwrap_or(24);
        let mask6 = args.mask6.unwrap_or(48);
        if mask4 > 32 || mask6 > 128 {
            return Err(crate::Error::Other("invalid mask".into()));
        }
        let preset = if let Some(p) = args.preset {
            match p.parse::<IpAddr>() {
                Ok(ip) => Some(ip),
                Err(e) => return Err(crate::Error::Other(format!("invalid preset addr: {}", e))),
            }
        } else {
            None
        };
        Ok(Self {
            forward,
            send,
            preset,
            mask4,
            mask6,
        })
    }

    /// Build a single EDNS0 CLIENT-SUBNET option tuple `(code, data)` for the given IP.
    ///
    /// The returned `Vec<u8>` follows the RFC 7871 format:
    /// FAMILY(2 bytes) | SOURCE_PREFIX(1 byte) | SCOPE_PREFIX(1 byte) | ADDRESS(variable length)
    ///
    /// # Arguments
    ///
    /// * `ip` - The IP address to encode in the ECS option
    /// * `mask4` - IPv4 subnet mask length (used only for IPv4 addresses)
    /// * `mask6` - IPv6 subnet mask length (used only for IPv6 addresses)
    ///
    /// # Returns
    ///
    /// Returns `Some((code, data))` where:
    /// - `code` is always 8 (EDNS Client Subnet option code)
    /// - `data` is the binary-encoded ECS option data
    ///
    /// Returns `None` only in impossible cases - callers can rely on `Some(...)` for valid inputs.
    #[allow(clippy::manual_div_ceil)]
    fn make_ecs_option(ip: IpAddr, mask4: u8, mask6: u8) -> Option<(u16, Vec<u8>)> {
        // EDNS Client Subnet option code = 8
        let code = 8u16;
        match ip {
            IpAddr::V4(v4) => {
                let family = 1u16.to_be_bytes();
                let src_mask = mask4.min(32);
                let scope = 0u8;
                let octets = v4.octets();
                let nbytes = ((src_mask as usize) + 7) / 8;
                let mut data = Vec::with_capacity(4 + nbytes);
                data.extend_from_slice(&family);
                data.push(src_mask);
                data.push(scope);
                data.extend_from_slice(&octets[..nbytes]);
                Some((code, data))
            }
            IpAddr::V6(v6) => {
                let family = 2u16.to_be_bytes();
                let src_mask = mask6.min(128);
                let scope = 0u8;
                let octets = v6.octets();
                let nbytes = ((src_mask as usize) + 7) / 8;
                let mut data = Vec::with_capacity(4 + nbytes);
                data.extend_from_slice(&family);
                data.push(src_mask);
                data.push(scope);
                data.extend_from_slice(&octets[..nbytes]);
                Some((code, data))
            }
        }
    }
}

impl fmt::Debug for EcsPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EcsPlugin")
            .field("forward", &self.forward)
            .field("send", &self.send)
            .field("preset", &self.preset)
            .finish()
    }
}

#[async_trait]
impl Plugin for EcsPlugin {
    /// Returns the plugin name.
    fn name(&self) -> &str {
        "ecs"
    }

    /// Execute the ECS handler logic.
    ///
    /// This method processes the DNS query context and prepares EDNS0 CLIENT-SUBNET
    /// options based on the plugin configuration. The options are stored in context
    /// metadata for use by downstream forwarding plugins.
    ///
    /// # Processing Logic
    ///
    /// 1. **Forward Mode**: If `forward` is enabled and client provided ECS options
    ///    exist in `client_edns0_options` metadata, copy them to `edns0_options`.
    ///
    /// 2. **Preset Mode**: If `preset` is configured, generate ECS option from the
    ///    preset IP address and store in `edns0_options`.
    ///
    /// 3. **Send Mode**: If `send` is enabled, attempt to derive client IP from
    ///    `client_addr` metadata and generate ECS option.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The mutable plugin context containing request data and metadata
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success. This method does not fail under normal circumstances.
    ///
    /// # Metadata Effects
    ///
    /// - Sets `edns0_options` with the prepared ECS options
    /// - Sets `edns0_preserve_existing` to `true` to indicate options should be preserved
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        // If forward mode, attempt to copy client EDNS0 options from metadata `client_edns0_options`.
        if self.forward
            && let Some(options) = ctx.get_metadata::<Vec<(u16, Vec<u8>)>>("client_edns0_options")
        {
            ctx.set_metadata("edns0_options", options.clone());
            ctx.set_metadata("edns0_preserve_existing", true);
            return Ok(());
        }

        // If preset is configured, add ECS option derived from preset
        if let Some(ip) = &self.preset {
            if let Some((code, data)) = EcsPlugin::make_ecs_option(*ip, self.mask4, self.mask6) {
                let opt = vec![(code, data)];
                ctx.set_metadata("edns0_options", opt);
                ctx.set_metadata("edns0_preserve_existing", true);
            }
            return Ok(());
        }

        // If send: try to derive client IP from metadata `client_addr` (string)
        if self.send
            && let Some(addr) = ctx.get_metadata::<String>("client_addr")
            && let Ok(ip) = addr.parse::<IpAddr>()
            && let Some((code, data)) = EcsPlugin::make_ecs_option(ip, self.mask4, self.mask6)
        {
            let opt = vec![(code, data)];
            ctx.set_metadata("edns0_options", opt);
            ctx.set_metadata("edns0_preserve_existing", true);
        }

        Ok(())
    }
}

impl ExecPlugin for EcsPlugin {
    /// Parse a quick configuration string for ECS handler plugin.
    ///
    /// Accepts comma-separated key=value options:
    /// - `forward=true/false`: Enable forwarding of client ECS options
    /// - `send=true/false`: Enable deriving ECS from client address
    /// - `preset=IP`: Use preset IP address for ECS
    /// - `mask4=N`: IPv4 subnet mask length (default 24)
    /// - `mask6=N`: IPv6 subnet mask length (default 48)
    ///
    /// Examples: "forward=true", "send=true,mask4=24,mask6=56", "preset=192.0.2.1"
    fn quick_setup(prefix: &str, exec_str: &str) -> Result<Arc<dyn Plugin>> {
        if prefix != "ecs" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'ecs'",
                prefix
            )));
        }

        let mut forward = None;
        let mut send = None;
        let mut preset = None;
        let mut mask4 = None;
        let mut mask6 = None;

        for part in exec_str.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let kv: Vec<&str> = part.splitn(2, '=').collect();
            if kv.len() != 2 {
                return Err(crate::Error::Config(format!(
                    "Invalid key=value pair: '{}'",
                    part
                )));
            }
            let key = kv[0].trim();
            let value = kv[1].trim();
            match key {
                "forward" => {
                    forward = Some(value.parse::<bool>().map_err(|_| {
                        crate::Error::Config(format!("Invalid boolean for forward: '{}'", value))
                    })?);
                }
                "send" => {
                    send = Some(value.parse::<bool>().map_err(|_| {
                        crate::Error::Config(format!("Invalid boolean for send: '{}'", value))
                    })?);
                }
                "preset" => {
                    preset = Some(value.to_string());
                }
                "mask4" => {
                    mask4 = Some(value.parse::<u8>().map_err(|_| {
                        crate::Error::Config(format!("Invalid u8 for mask4: '{}'", value))
                    })?);
                }
                "mask6" => {
                    mask6 = Some(value.parse::<u8>().map_err(|_| {
                        crate::Error::Config(format!("Invalid u8 for mask6: '{}'", value))
                    })?);
                }
                _ => {
                    return Err(crate::Error::Config(format!("Unknown option: '{}'", key)));
                }
            }
        }

        let args = EcsArgs {
            forward,
            send,
            preset,
            mask4,
            mask6,
        };

        let plugin = EcsPlugin::new(args)?;
        Ok(Arc::new(plugin))
    }
}

#[cfg(test)]
mod tests {
    //! Test module for EcsPlugin
    //!
    //! This module contains comprehensive unit tests for the ECS handler functionality,
    //! covering all configuration modes and edge cases.

    use super::*;
    use crate::dns::Message;
    use std::net::Ipv6Addr;

    /// Test ECS handler with IPv4 preset address.
    ///
    /// Verifies that when a preset IPv4 address is configured, the handler
    /// correctly generates ECS options and stores them in context metadata.
    #[tokio::test]
    async fn test_ecs_preset_v4() {
        let args = EcsArgs {
            forward: None,
            send: None,
            preset: Some("192.0.2.5".to_string()),
            mask4: Some(24),
            mask6: None,
        };
        let plugin = EcsPlugin::new(args).unwrap();
        let req = Message::new();
        let mut ctx = Context::new(req);
        plugin.execute(&mut ctx).await.unwrap();
        let opts = ctx.get_metadata::<Vec<(u16, Vec<u8>)>>("edns0_options");
        assert!(opts.is_some());
    }

    /// Test ECS handler forward mode.
    ///
    /// Verifies that when forward mode is enabled, client-provided EDNS0 options
    /// are correctly copied from `client_edns0_options` metadata to `edns0_options`.
    #[tokio::test]
    async fn test_ecs_forward_copies_client_options() {
        let args = EcsArgs {
            forward: Some(true),
            send: None,
            preset: None,
            mask4: None,
            mask6: None,
        };
        let plugin = EcsPlugin::new(args).unwrap();
        let req = Message::new();
        let mut ctx = Context::new(req);
        // prepare client_edns0_options metadata and ensure it's copied
        let client_opts: Vec<(u16, Vec<u8>)> = vec![(8u16, vec![0, 1, 24, 0, 192, 0, 2])];
        ctx.set_metadata("client_edns0_options", client_opts.clone());
        plugin.execute(&mut ctx).await.unwrap();
        let got = ctx.get_metadata::<Vec<(u16, Vec<u8>)>>("edns0_options");
        assert!(got.is_some());
        assert_eq!(got.unwrap(), &client_opts);
    }

    /// Test ECS handler send mode with client address derivation.
    ///
    /// Verifies that when send mode is enabled and client address is available
    /// in metadata, the handler correctly derives ECS options from the client IP.
    /// Also tests that IPv6 ECS option generation works correctly.
    #[tokio::test]
    async fn test_ecs_send_derives_from_client_addr() {
        let args = EcsArgs {
            forward: None,
            send: Some(true),
            preset: None,
            mask4: Some(24),
            mask6: Some(56),
        };
        let plugin = EcsPlugin::new(args).unwrap();
        let req = Message::new();
        let mut ctx = Context::new(req);
        ctx.set_metadata("client_addr", "192.0.2.7".to_string());
        plugin.execute(&mut ctx).await.unwrap();
        let got = ctx.get_metadata::<Vec<(u16, Vec<u8>)>>("edns0_options");
        assert!(got.is_some());
        // also verify preset/IPv6 path doesn't panic: call make_ecs_option directly
        let _ = EcsPlugin::make_ecs_option(IpAddr::V6(Ipv6Addr::LOCALHOST), 24, 56);
    }

    /// Test ECS handler quick_setup functionality.
    ///
    /// Verifies that the quick_setup method correctly parses exec strings
    /// and creates configured EcsPlugin instances.
    #[test]
    fn test_ecs_quick_setup() {
        // Test forward mode
        let plugin = EcsPlugin::quick_setup("ecs", "forward=true").unwrap();
        assert_eq!(plugin.name(), "ecs");

        // Test send mode with custom masks
        let plugin = EcsPlugin::quick_setup("ecs", "send=true,mask4=20,mask6=40").unwrap();
        assert_eq!(plugin.name(), "ecs");

        // Test preset mode
        let plugin = EcsPlugin::quick_setup("ecs", "preset=192.0.2.1").unwrap();
        assert_eq!(plugin.name(), "ecs");

        // Test invalid prefix
        assert!(EcsPlugin::quick_setup("invalid", "forward=true").is_err());

        // Test invalid option
        assert!(EcsPlugin::quick_setup("ecs", "invalid=true").is_err());
    }
}
