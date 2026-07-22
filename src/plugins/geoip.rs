//! GeoIP plugin for geographic IP address matching
//!
//! This plugin provides geographic location matching for IP addresses.
//! It supports a simple text-based database format for easy testing and
//! can be extended to support MaxMind GeoIP2 databases.
//!
//! # Features
//!
//! - **Country code matching**: Match IPs by country code
//! - **Text database**: Simple text-based IP to country mapping
//! - **CIDR support**: Works with CIDR ranges
//! - **Metadata tagging**: Sets geographic metadata in context
//!
//! # Example
//!
//! ```rust
//! use lazydns::plugins::GeoIpPlugin;
//! use lazydns::plugin::Plugin;
//! use std::sync::Arc;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut geoip = GeoIpPlugin::new("country_code");
//! // Add IP range for China
//! geoip.add_country_cidr("CN", "1.0.1.0/24".parse()?)?;
//! geoip.add_country_cidr("CN", "1.0.2.0/24".parse()?)?;
//!
//! let plugin: Arc<dyn Plugin> = Arc::new(geoip);
//! # Ok(())
//! # }
//! ```

use crate::RegisterPlugin;
use crate::config::types::PluginConfig;
use crate::dns::RData;
use crate::error::Error;
use crate::plugin::{Context, Plugin};
use async_trait::async_trait;
use ipnet::IpNet;
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

/// GeoIP plugin for geographic IP matching
///
/// Matches IP addresses to countries and sets metadata for routing decisions.
#[derive(Clone, RegisterPlugin)]
pub struct GeoIpPlugin {
    /// Metadata key to store country code
    metadata_key: String,
    /// Country code to CIDR ranges mapping
    country_ranges: HashMap<String, Vec<IpNet>>,
}

impl GeoIpPlugin {
    /// Create a new GeoIP plugin
    ///
    /// # Arguments
    ///
    /// * `metadata_key` - Key to use for storing country code in metadata
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::GeoIpPlugin;
    ///
    /// let geoip = GeoIpPlugin::new("country");
    /// ```
    pub fn new(metadata_key: impl Into<String>) -> Self {
        Self {
            metadata_key: metadata_key.into(),
            country_ranges: HashMap::new(),
        }
    }

    /// Add a CIDR range for a country
    ///
    /// # Arguments
    ///
    /// * `country_code` - ISO 3166-1 alpha-2 country code (e.g., "US", "CN")
    /// * `cidr` - CIDR range for the country
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::GeoIpPlugin;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut geoip = GeoIpPlugin::new("country");
    /// geoip.add_country_cidr("US", "8.8.8.0/24".parse()?)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_country_cidr(&mut self, country_code: &str, cidr: IpNet) -> Result<(), Error> {
        let country = country_code.to_uppercase();
        self.country_ranges.entry(country).or_default().push(cidr);
        Ok(())
    }

    /// Load GeoIP data from a text file
    ///
    /// Format: `<cidr> <country_code>`
    ///
    /// Lines starting with `#` are comments.
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::GeoIpPlugin;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut geoip = GeoIpPlugin::new("country");
    /// let data = r#"
    /// # US ranges
    /// 8.8.8.0/24 US
    /// 8.8.4.0/24 US
    /// # China ranges
    /// 1.0.1.0/24 CN
    /// "#;
    /// geoip.load_from_string(data)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn load_from_string(&mut self, content: &str) -> Result<(), Error> {
        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse line: <cidr> <country_code>
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue; // Skip malformed lines
            }

            let cidr: IpNet = parts[0]
                .parse()
                .map_err(|e| Error::Config(format!("Invalid CIDR '{}': {}", parts[0], e)))?;

            let country_code = parts[1];
            self.add_country_cidr(country_code, cidr)?;
        }

        Ok(())
    }

    /// Lookup country code for an IP address
    ///
    /// # Returns
    ///
    /// Country code if found, None otherwise
    pub fn lookup(&self, ip: &IpAddr) -> Option<String> {
        for (country, ranges) in &self.country_ranges {
            for range in ranges {
                if range.contains(ip) {
                    return Some(country.clone());
                }
            }
        }
        None
    }

    /// Get the number of countries in the database
    pub fn country_count(&self) -> usize {
        self.country_ranges.len()
    }

    /// Get the total number of CIDR ranges
    pub fn range_count(&self) -> usize {
        self.country_ranges.values().map(|v| v.len()).sum()
    }

    /// Check if the database is empty
    pub fn is_empty(&self) -> bool {
        self.country_ranges.is_empty()
    }

    /// Clear all GeoIP data
    pub fn clear(&mut self) {
        self.country_ranges.clear();
    }

    /// Extract IP address from DNS RData
    fn extract_ip(rdata: &RData) -> Option<IpAddr> {
        match rdata {
            RData::A(ipv4) => Some(IpAddr::V4(*ipv4)),
            RData::AAAA(ipv6) => Some(IpAddr::V6(*ipv6)),
            _ => None,
        }
    }
}

impl fmt::Debug for GeoIpPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeoIpPlugin")
            .field("metadata_key", &self.metadata_key)
            .field("countries", &self.country_count())
            .field("ranges", &self.range_count())
            .finish()
    }
}

#[async_trait]
impl Plugin for GeoIpPlugin {
    async fn execute(&self, context: &mut Context) -> Result<(), Error> {
        // Only process if we have a response
        let response = match context.response() {
            Some(r) => r,
            None => return Ok(()),
        };

        // Check answer section for IP addresses
        for record in response.answers() {
            if let Some(ip) = Self::extract_ip(record.rdata())
                && let Some(country) = self.lookup(&ip)
            {
                // Set country code in metadata
                context.set_metadata(self.metadata_key.clone(), country.clone());

                debug!("GeoIP: IP {} belongs to country {}", ip, country);

                // Return after first match
                return Ok(());
            }
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "geo_ip"
    }

    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>, Error> {
        use serde_yaml::Value;

        let args = config.effective_args();

        let metadata_key = match args.get("metadata_key") {
            Some(Value::String(s)) => s.clone(),
            _ => "country".to_string(),
        };

        let mut geoip = GeoIpPlugin::new(metadata_key);

        // Load from files
        if let Some(Value::Sequence(seq)) = args.get("files") {
            for file_val in seq {
                if let Some(file_str) = file_val.as_str() {
                    let file = PathBuf::from(file_str);
                    let content = std::fs::read_to_string(&file).map_err(|e| {
                        Error::Config(format!(
                            "Failed to read GeoIP file '{}': {}",
                            file.display(),
                            e
                        ))
                    })?;
                    geoip.load_from_string(&content)?;
                }
            }
        }

        // Load inline data
        if let Some(Value::Sequence(seq)) = args.get("data") {
            for entry_val in seq {
                if let Some(entry) = entry_val.as_str() {
                    geoip.load_from_string(entry)?;
                }
            }
        }

        Ok(Arc::new(geoip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, RecordClass, RecordType, ResourceRecord};
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_geoip_plugin_creation() {
        let geoip = GeoIpPlugin::new("country");
        assert!(geoip.is_empty());
        assert_eq!(geoip.country_count(), 0);
        assert_eq!(geoip.range_count(), 0);
    }

    #[test]
    fn test_add_country_cidr() {
        let mut geoip = GeoIpPlugin::new("country");
        geoip
            .add_country_cidr("US", "8.8.8.0/24".parse().unwrap())
            .unwrap();

        assert_eq!(geoip.country_count(), 1);
        assert_eq!(geoip.range_count(), 1);
    }

    #[test]
    fn test_lookup() {
        let mut geoip = GeoIpPlugin::new("country");
        geoip
            .add_country_cidr("US", "8.8.8.0/24".parse().unwrap())
            .unwrap();
        geoip
            .add_country_cidr("CN", "1.0.1.0/24".parse().unwrap())
            .unwrap();

        // Test US IP
        let ip_us = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
        assert_eq!(geoip.lookup(&ip_us), Some("US".to_string()));

        // Test China IP
        let ip_cn = IpAddr::V4(Ipv4Addr::new(1, 0, 1, 1));
        assert_eq!(geoip.lookup(&ip_cn), Some("CN".to_string()));

        // Test unknown IP
        let ip_unknown = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(geoip.lookup(&ip_unknown), None);
    }

    #[test]
    fn test_load_from_string() {
        let mut geoip = GeoIpPlugin::new("country");
        let data = r#"
# US ranges
8.8.8.0/24 US
8.8.4.0/24 US
# China ranges
1.0.1.0/24 CN
1.0.2.0/24 CN
        "#;

        geoip.load_from_string(data).unwrap();

        assert_eq!(geoip.country_count(), 2);
        assert_eq!(geoip.range_count(), 4);

        // Verify lookups
        assert_eq!(
            geoip.lookup(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
            Some("US".to_string())
        );
        assert_eq!(
            geoip.lookup(&IpAddr::V4(Ipv4Addr::new(1, 0, 1, 1))),
            Some("CN".to_string())
        );
    }

    #[test]
    fn test_ipv6_support() {
        let mut geoip = GeoIpPlugin::new("country");
        geoip
            .add_country_cidr("US", "2001:4860::/32".parse().unwrap())
            .unwrap();

        let ip = IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0, 0, 0, 0, 0, 1));
        assert_eq!(geoip.lookup(&ip), Some("US".to_string()));
    }

    #[test]
    fn test_clear() {
        let mut geoip = GeoIpPlugin::new("country");
        geoip
            .add_country_cidr("US", "8.8.8.0/24".parse().unwrap())
            .unwrap();

        assert!(!geoip.is_empty());

        geoip.clear();

        assert!(geoip.is_empty());
        assert_eq!(geoip.country_count(), 0);
    }

    #[tokio::test]
    async fn test_geoip_plugin_execution() {
        let mut geoip = GeoIpPlugin::new("country");
        geoip
            .add_country_cidr("US", "8.8.8.0/24".parse().unwrap())
            .unwrap();

        // Create response with US IP
        let mut response = Message::new();
        response.set_response(true);
        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A(Ipv4Addr::new(8, 8, 8, 8)),
        ));

        let mut context = Context::new(Message::new());
        context.set_response(Some(response));

        geoip.execute(&mut context).await.unwrap();

        // Check that country code was set
        let country: Option<&String> = context.get_metadata("country");
        assert_eq!(country, Some(&"US".to_string()));
    }

    #[tokio::test]
    async fn test_geoip_plugin_no_response() {
        let mut geoip = GeoIpPlugin::new("country");
        geoip
            .add_country_cidr("US", "8.8.8.0/24".parse().unwrap())
            .unwrap();

        let mut context = Context::new(Message::new());
        // No response set

        geoip.execute(&mut context).await.unwrap();

        // Should not set metadata
        let country: Option<&String> = context.get_metadata("country");
        assert_eq!(country, None);
    }
}
