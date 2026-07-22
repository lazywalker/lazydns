//! GeoSite plugin for geographic domain matching
//!
//! This plugin provides geographic location matching for domain names.
//! It's useful for routing decisions based on where domains are hosted or their service region.
//!
//! # Features
//!
//! - **Country/region matching**: Match domains by country/region
//! - **Category support**: Support for domain categories (e.g., "cn", "geolocation-!cn")
//! - **Multiple domains**: Single category can contain many domains
//! - **Wildcard support**: Suffix matching for domains
//!
//! # Example
//!
//! ```rust
//! use lazydns::plugins::GeoSitePlugin;
//! use lazydns::plugin::Plugin;
//! use std::sync::Arc;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut geosite = GeoSitePlugin::new("site_category");
//! // Add Chinese domains
//! geosite.add_domain("cn", "baidu.com");
//! geosite.add_domain("cn", "qq.com");
//! // Add US domains
//! geosite.add_domain("us", "google.com");
//! geosite.add_domain("us", "facebook.com");
//!
//! let plugin: Arc<dyn Plugin> = Arc::new(geosite);
//! # Ok(())
//! # }
//! ```

use crate::RegisterPlugin;
use crate::dns::RecordType;
use crate::error::Error;
use crate::plugin::{Context, Plugin};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use tracing::debug;

/// GeoSite plugin for geographic domain matching
///
/// Matches domains to categories (countries/regions) and sets metadata for routing.
#[derive(RegisterPlugin)]
pub struct GeoSitePlugin {
    /// Metadata key to store category
    metadata_key: String,
    /// Category to domains mapping
    /// Each category contains a set of exact domains and suffix patterns
    categories: HashMap<String, DomainSet>,
}

/// Set of domains for a category
#[derive(Debug, Clone)]
struct DomainSet {
    /// Exact domain matches
    exact: HashSet<String>,
    /// Suffix patterns (e.g., "example.com" matches "*.example.com")
    suffixes: HashSet<String>,
}

impl DomainSet {
    fn new() -> Self {
        Self {
            exact: HashSet::new(),
            suffixes: HashSet::new(),
        }
    }

    fn add_exact(&mut self, domain: String) {
        self.exact.insert(domain.to_lowercase());
    }

    fn add_suffix(&mut self, suffix: String) {
        let suffix = suffix.to_lowercase();
        let suffix = suffix.strip_prefix('.').unwrap_or(&suffix);
        self.suffixes.insert(suffix.to_string());
    }

    fn matches(&self, domain: &str) -> bool {
        let domain_lower = domain.to_lowercase();

        // Check exact match
        if self.exact.contains(&domain_lower) {
            return true;
        }

        // Check suffix match
        for suffix in &self.suffixes {
            if domain_lower == *suffix || domain_lower.ends_with(&format!(".{}", suffix)) {
                return true;
            }
        }

        false
    }

    fn len(&self) -> usize {
        self.exact.len() + self.suffixes.len()
    }
}

impl GeoSitePlugin {
    /// Create a new GeoSite plugin
    ///
    /// # Arguments
    ///
    /// * `metadata_key` - Key to use for storing category in metadata
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::GeoSitePlugin;
    ///
    /// let geosite = GeoSitePlugin::new("site_category");
    /// ```
    pub fn new(metadata_key: impl Into<String>) -> Self {
        Self {
            metadata_key: metadata_key.into(),
            categories: HashMap::new(),
        }
    }

    /// Add a domain to a category
    ///
    /// # Arguments
    ///
    /// * `category` - Category name (e.g., "cn", "us", "ads")
    /// * `domain` - Domain name
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::GeoSitePlugin;
    ///
    /// let mut geosite = GeoSitePlugin::new("category");
    /// geosite.add_domain("cn", "baidu.com");
    /// geosite.add_domain("us", "google.com");
    /// ```
    pub fn add_domain(&mut self, category: &str, domain: &str) {
        let category = category.to_lowercase();
        let domain_set = self
            .categories
            .entry(category)
            .or_insert_with(DomainSet::new);

        // Check if it's a wildcard pattern
        if let Some(suffix) = domain.strip_prefix("*.") {
            domain_set.add_suffix(suffix.to_string());
        } else if let Some(suffix) = domain.strip_prefix('.') {
            domain_set.add_suffix(suffix.to_string());
        } else {
            domain_set.add_exact(domain.to_string());
        }
    }

    /// Load GeoSite data from a text file
    ///
    /// Format: `<category> <domain>`
    ///
    /// Lines starting with `#` are comments.
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::GeoSitePlugin;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut geosite = GeoSitePlugin::new("category");
    /// let data = r#"
    /// # Chinese sites
    /// cn baidu.com
    /// cn *.qq.com
    /// # US sites
    /// us google.com
    /// us *.facebook.com
    /// "#;
    /// geosite.load_from_string(data)?;
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

            // Parse line: <category> <domain>
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue; // Skip malformed lines
            }

            let category = parts[0];
            let domain = parts[1];
            self.add_domain(category, domain);
        }

        Ok(())
    }

    /// Lookup category for a domain
    ///
    /// # Returns
    ///
    /// Category name if found, None otherwise
    pub fn lookup(&self, domain: &str) -> Option<String> {
        for (category, domain_set) in &self.categories {
            if domain_set.matches(domain) {
                return Some(category.clone());
            }
        }
        None
    }

    /// Get the number of categories
    pub fn category_count(&self) -> usize {
        self.categories.len()
    }

    /// Get the total number of domains across all categories
    pub fn domain_count(&self) -> usize {
        self.categories.values().map(|ds| ds.len()).sum()
    }

    /// Check if the database is empty
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
    }

    /// Clear all GeoSite data
    pub fn clear(&mut self) {
        self.categories.clear();
    }
}

impl fmt::Debug for GeoSitePlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeoSitePlugin")
            .field("metadata_key", &self.metadata_key)
            .field("categories", &self.category_count())
            .field("domains", &self.domain_count())
            .finish()
    }
}

#[async_trait]
impl Plugin for GeoSitePlugin {
    async fn execute(&self, context: &mut Context) -> Result<(), Error> {
        // Get the first question
        let question = match context.request().questions().first() {
            Some(q) => q,
            None => return Ok(()),
        };

        // Only process A and AAAA queries
        let qtype = question.qtype();
        if qtype != RecordType::A && qtype != RecordType::AAAA {
            return Ok(());
        }

        // Lookup domain category (store domain string to avoid borrow issues)
        let domain = question.qname().to_string();
        if let Some(category) = self.lookup(&domain) {
            // Set category in metadata
            context.set_metadata(self.metadata_key.clone(), category.clone());

            debug!(
                "GeoSite: Domain {} belongs to category {}",
                domain, category
            );
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "geo_site"
    }

    fn init(
        config: &crate::config::types::PluginConfig,
    ) -> Result<std::sync::Arc<dyn Plugin>, Error> {
        use serde_yaml::Value;

        let args = config.effective_args();

        let metadata_key = match args.get("metadata_key") {
            Some(Value::String(s)) => s.clone(),
            _ => "category".to_string(),
        };

        let mut geosite = GeoSitePlugin::new(metadata_key);

        // Load from files
        if let Some(Value::Sequence(seq)) = args.get("files") {
            for file_val in seq {
                if let Some(file_str) = file_val.as_str() {
                    let file = PathBuf::from(file_str);
                    let content = std::fs::read_to_string(&file).map_err(|e| {
                        Error::Config(format!(
                            "Failed to read GeoSite file '{}': {}",
                            file.display(),
                            e
                        ))
                    })?;
                    geosite.load_from_string(&content)?;
                }
            }
        }

        // Load inline data
        if let Some(Value::Sequence(seq)) = args.get("data") {
            for entry_val in seq {
                if let Some(entry) = entry_val.as_str() {
                    geosite.load_from_string(entry)?;
                }
            }
        }

        Ok(std::sync::Arc::new(geosite))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, Question, RecordClass};

    #[test]
    fn test_geosite_plugin_creation() {
        let geosite = GeoSitePlugin::new("category");
        assert!(geosite.is_empty());
        assert_eq!(geosite.category_count(), 0);
        assert_eq!(geosite.domain_count(), 0);
    }

    #[test]
    fn test_add_domain() {
        let mut geosite = GeoSitePlugin::new("category");
        geosite.add_domain("cn", "baidu.com");
        geosite.add_domain("us", "google.com");

        assert_eq!(geosite.category_count(), 2);
        assert_eq!(geosite.domain_count(), 2);
    }

    #[test]
    fn test_lookup_exact() {
        let mut geosite = GeoSitePlugin::new("category");
        geosite.add_domain("cn", "baidu.com");
        geosite.add_domain("us", "google.com");

        assert_eq!(geosite.lookup("baidu.com"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("google.com"), Some("us".to_string()));
        assert_eq!(geosite.lookup("yahoo.com"), None);
    }

    #[test]
    fn test_lookup_wildcard() {
        let mut geosite = GeoSitePlugin::new("category");
        geosite.add_domain("cn", "*.qq.com");

        assert_eq!(geosite.lookup("qq.com"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("mail.qq.com"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("deep.mail.qq.com"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("notqq.com"), None);
    }

    #[test]
    fn test_load_from_string() {
        let mut geosite = GeoSitePlugin::new("category");
        let data = r#"
# Chinese sites
cn baidu.com
cn *.qq.com
cn weibo.com
# US sites
us google.com
us *.facebook.com
        "#;

        geosite.load_from_string(data).unwrap();

        assert_eq!(geosite.category_count(), 2);
        assert_eq!(geosite.domain_count(), 5);

        // Verify lookups
        assert_eq!(geosite.lookup("baidu.com"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("mail.qq.com"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("google.com"), Some("us".to_string()));
        assert_eq!(geosite.lookup("www.facebook.com"), Some("us".to_string()));
    }

    #[test]
    fn test_case_insensitive() {
        let mut geosite = GeoSitePlugin::new("category");
        geosite.add_domain("cn", "Baidu.COM");

        assert_eq!(geosite.lookup("baidu.com"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("BAIDU.COM"), Some("cn".to_string()));
        assert_eq!(geosite.lookup("BaiDu.CoM"), Some("cn".to_string()));
    }

    #[test]
    fn test_clear() {
        let mut geosite = GeoSitePlugin::new("category");
        geosite.add_domain("cn", "baidu.com");

        assert!(!geosite.is_empty());

        geosite.clear();

        assert!(geosite.is_empty());
        assert_eq!(geosite.category_count(), 0);
    }

    #[tokio::test]
    async fn test_geosite_plugin_execution() {
        let mut geosite = GeoSitePlugin::new("site_category");
        geosite.add_domain("cn", "baidu.com");

        let mut request = Message::new();
        request.add_question(Question::new("baidu.com", RecordType::A, RecordClass::IN));

        let mut context = Context::new(request);
        geosite.execute(&mut context).await.unwrap();

        // Check that category was set
        let category: Option<&String> = context.get_metadata("site_category");
        assert_eq!(category, Some(&"cn".to_string()));
    }

    #[tokio::test]
    async fn test_geosite_plugin_no_match() {
        let mut geosite = GeoSitePlugin::new("site_category");
        geosite.add_domain("cn", "baidu.com");

        let mut request = Message::new();
        request.add_question(Question::new("google.com", RecordType::A, RecordClass::IN));

        let mut context = Context::new(request);
        geosite.execute(&mut context).await.unwrap();

        // Should not set metadata
        let category: Option<&String> = context.get_metadata("site_category");
        assert_eq!(category, None);
    }
}
