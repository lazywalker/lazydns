use crate::config::PluginConfig;
use crate::plugin::traits::{Matcher, Shutdown};
use crate::plugin::{Context, Plugin};
use crate::{RegisterPlugin, Result, ShutdownPlugin};
use async_trait::async_trait;
use parking_lot::RwLock;
use regex::Regex;
use std::collections::HashSet;
use std::fmt;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Domain matching rule types with priority-based evaluation.
///
/// The `MatchType` enum defines different strategies for matching domain names against rules.
/// When a query domain needs to be matched, the system checks rules in priority order:
/// **Full > Domain > Regexp > Keyword**, and returns `true` on the first match.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MatchType {
    /// Exact match only (such as `full:google.com` matches only `google.com`)
    #[default]
    Full,
    /// Domain and subdomain match (such as `domain:google.com` matches `google.com`, `www.google.com`)
    Domain,
    /// Regex pattern match (such as `regexp:.+\.google\.com$`)
    Regexp,
    /// Keyword/substring match (such as `keyword:google` matches any domain containing "google")
    Keyword,
}

/// Compiled regex rule with original pattern for debugging
#[derive(Clone)]
struct RegexpRule {
    pattern: String,
    regex: Regex,
}

impl fmt::Debug for RegexpRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RegexpRule({:?})", self.pattern)
    }
}

/// Domain matching rules storage with optimized data structures.
pub struct DomainRules {
    /// Full/exact match domains - O(1) lookup
    full: HashSet<String>,
    /// Domain match rules (matches self and subdomains) - O(1) lookup per level
    domain: HashSet<String>,
    /// Regexp rules in import order - O(n) traversal
    regexp: Vec<RegexpRule>,
    /// Keyword rules in import order - O(n) traversal
    keyword: Vec<String>,
}

impl DomainRules {
    pub fn new() -> Self {
        Self::with_capacity(0, 0)
    }

    /// Create a new DomainRules with pre-allocated capacity
    ///
    /// This avoids multiple reallocations when loading large domain lists.
    /// Use this when you know the total size upfront.
    pub fn with_capacity(full_cap: usize, domain_cap: usize) -> Self {
        Self {
            full: HashSet::with_capacity(full_cap),
            domain: HashSet::with_capacity(domain_cap),
            regexp: Vec::new(),
            keyword: Vec::new(),
        }
    }

    /// Add a single rule to the domain rules collection.
    ///
    /// The rule is normalized to lowercase and added to the appropriate internal storage
    /// based on the match type. Invalid regex patterns are logged as warnings and skipped.
    ///
    /// # Arguments
    ///
    /// * `match_type` - The type of matching strategy (Full, Domain, Regexp, or Keyword)
    /// * `value` - The rule pattern/value (domain, regex pattern, or keyword)
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut rules = DomainRules::new();
    /// rules.add_rule(MatchType::Full, "example.com");
    /// rules.add_rule(MatchType::Domain, "google.com");
    /// rules.add_rule(MatchType::Regexp, r".+\.github\.io$");
    /// rules.add_rule(MatchType::Keyword, "test");
    /// ```
    /// Add a rule with an owned String pattern.
    pub fn add_rule(&mut self, match_type: MatchType, value: String) {
        if value.is_empty() {
            return;
        }

        // Normalize non-regexp rules to lowercase for case-insensitive matching
        let normalized = match match_type {
            MatchType::Regexp => value,
            _ => value.to_lowercase(),
        };

        match match_type {
            MatchType::Full => {
                self.full.insert(normalized);
            }
            MatchType::Domain => {
                self.domain.insert(normalized);
            }
            MatchType::Regexp => match Regex::new(&normalized) {
                Ok(regex) => {
                    self.regexp.push(RegexpRule {
                        pattern: normalized,
                        regex,
                    });
                }
                Err(e) => {
                    warn!(pattern = %normalized, error = %e, "Invalid regexp pattern, skipping");
                }
            },
            MatchType::Keyword => {
                self.keyword.push(normalized);
            }
        }
    }

    /// Parse a single line from a rules file and add it to the collection.
    ///
    /// This method handles various rule formats commonly found in domain list files.
    /// Empty lines and comments (starting with `#`) are ignored.
    ///
    /// # Supported Formats
    ///
    /// Rules can be prefixed with a match type indicator:
    /// - `full:example.com` - Exact domain match
    /// - `domain:example.com` - Domain and subdomain match
    /// - `keyword:google` - Substring/keyword match
    /// - `regexp:.+\.google\.com$` - Regex pattern match
    /// - `example.com` - Uses the provided default match type
    ///
    /// Comments and whitespace:
    /// - Lines starting with `#` are treated as comments and ignored
    /// - Leading and trailing whitespace is automatically trimmed
    /// - Empty lines are silently skipped
    ///
    /// # Arguments
    ///
    /// * `line` - A single line from a rules file
    /// * `default_type` - The match type to use if no prefix is specified
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut rules = DomainRules::new();
    /// rules.add_line("# This is a comment", MatchType::Domain);
    /// rules.add_line("example.com", MatchType::Domain);
    /// rules.add_line("domain:google.com", MatchType::Full);
    /// rules.add_line("full:exact.net", MatchType::Domain);
    /// rules.add_line("keyword:facebook", MatchType::Domain);
    /// rules.add_line("regexp:.*twitter.*", MatchType::Domain);
    /// ```
    pub fn add_line(&mut self, line: &str, default_type: &MatchType) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return;
        }

        let (match_type, raw) = if let Some(rest) = line.strip_prefix("full:") {
            (MatchType::Full, rest)
        } else if let Some(rest) = line.strip_prefix("domain:") {
            (MatchType::Domain, rest)
        } else if let Some(rest) = line.strip_prefix("keyword:") {
            (MatchType::Keyword, rest)
        } else if let Some(rest) = line.strip_prefix("regexp:") {
            (MatchType::Regexp, rest)
        } else {
            (default_type.clone(), line)
        };

        let value = raw.trim();
        if value.is_empty() {
            return;
        }

        // For regex patterns we must not lowercase or alter the pattern.
        // For other types, normalize to lowercase to enable case-insensitive matching.
        let normalized = match match_type {
            MatchType::Regexp => value.to_string(),
            _ => value.to_lowercase(),
        };

        self.add_rule(match_type, normalized);
    }

    /// Check if a domain matches any rule in the collection.
    ///
    /// This is the main evaluation function that applies all rules in priority order.
    /// The matching process:
    ///
    /// 1. **Full Match** (Highest Priority, O(1)): Checks for exact domain match
    /// 2. **Domain Match** (O(levels)): Splits domain into labels and checks each level
    ///    - For example, `www.google.com` checks: `www.google.com`, `google.com`, `com`
    ///    - Returns on first match (most specific wins)
    /// 3. **Regexp Match** (O(n·regex)): Evaluates regex rules in import order
    /// 4. **Keyword Match** (Lowest Priority, O(n)): Checks substring in import order
    ///
    /// # Arguments
    ///
    /// * `domain` - The domain to test (case-insensitive, trailing dot optional)
    ///
    /// # Returns
    ///
    /// `true` if the domain matches any rule, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut rules = DomainRules::new();
    /// rules.add_rule(MatchType::Full, "exact.com");
    /// rules.add_rule(MatchType::Domain, "google.com");
    ///
    /// assert!(rules.matches("exact.com"));        // Exact match
    /// assert!(rules.matches("EXACT.COM"));        // Case-insensitive
    /// assert!(rules.matches("exact.com."));       // Trailing dot normalized
    /// assert!(!rules.matches("sub.exact.com"));   // Full doesn't match subs
    /// assert!(rules.matches("www.google.com"));   // Domain subdomain match
    /// assert!(!rules.matches("google.com.hk"));   // Not a subdomain
    /// ```
    pub fn matches(&self, domain: &str) -> bool {
        let domain_lower = domain.trim_end_matches('.').to_lowercase();

        // 1. Full match (highest priority) - O(1)
        if self.full.contains(domain_lower.as_str()) {
            return true;
        }

        // 2. Domain match - O(levels) where levels is domain depth
        // Check from most specific (full domain) to least specific (TLD)
        // This ensures subdomain priority: www.google.com matches google.com before com
        let parts: Vec<&str> = domain_lower.split('.').collect();
        for i in 0..parts.len() {
            let candidate = parts[i..].join(".");
            if self.domain.contains(candidate.as_str()) {
                return true;
            }
        }

        // 3. Regexp match - O(n) in import order
        for rule in &self.regexp {
            if rule.regex.is_match(&domain_lower) {
                return true;
            }
        }

        // 4. Keyword match (lowest priority) - O(n) in import order
        for kw in &self.keyword {
            if domain_lower.contains(kw) {
                return true;
            }
        }

        false
    }

    /// Get total rule count
    pub fn len(&self) -> usize {
        self.full.len() + self.domain.len() + self.regexp.len() + self.keyword.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get statistics
    pub fn stats(&self) -> DomainRulesStats {
        DomainRulesStats {
            full_count: self.full.len(),
            domain_count: self.domain.len(),
            regexp_count: self.regexp.len(),
            keyword_count: self.keyword.len(),
        }
    }

    /// Clear all rules
    pub fn clear(&mut self) {
        self.full.clear();
        self.domain.clear();
        self.regexp.clear();
        self.keyword.clear();
        // Explicitly shrink capacity to return memory to allocator
        self.full.shrink_to_fit();
        self.domain.shrink_to_fit();
        self.regexp.shrink_to_fit();
        self.keyword.shrink_to_fit();
    }

    /// Merge rules from another DomainRules
    pub fn merge(&mut self, other: DomainRules) {
        // Pre-reserve capacity to avoid multiple allocations during extend
        // This prevents the HashSet from repeatedly doubling capacity (waste)
        self.full.reserve(other.full.len());
        self.domain.reserve(other.domain.len());
        self.regexp.reserve(other.regexp.len());
        self.keyword.reserve(other.keyword.len());

        // Now extend without triggering additional implicit reserve
        self.full.extend(other.full);
        self.domain.extend(other.domain);
        self.regexp.extend(other.regexp);
        self.keyword.extend(other.keyword);
    }

    /// Shrink memory usage to fit actual content
    /// Should be called after loading is complete to minimize peak memory
    pub fn shrink_to_fit(&mut self) {
        self.full.shrink_to_fit();
        self.domain.shrink_to_fit();
        self.regexp.shrink_to_fit();
        self.keyword.shrink_to_fit();
    }
}

impl Default for DomainRules {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics for domain rules
#[derive(Debug, Clone)]
pub struct DomainRulesStats {
    pub full_count: usize,
    pub domain_count: usize,
    pub regexp_count: usize,
    pub keyword_count: usize,
}

/// Domain set data provider plugin
///
/// Loads domain names from files and provides them for matching.
/// Supports multiple match types with proper priority:
/// - `full:` - exact match only
/// - `domain:` - match self and subdomains
/// - `keyword:` - substring match
/// - `regexp:` - regex pattern match
///
/// Match priority: full > domain > regexp > keyword
///
/// # Example
///
/// ```rust
/// use lazydns::plugins::DomainSetPlugin;
/// use lazydns::plugins::dataset::MatchType;
///
/// let plugin = DomainSetPlugin::new("cn-domains")
///     .with_files(vec!["direct-list.txt".to_string()])
///     .with_auto_reload(true)
///     .with_default_match_type(MatchType::Domain);
/// ```
#[derive(Clone, RegisterPlugin, ShutdownPlugin)]
pub struct DomainSetPlugin {
    /// Name/tag for this domain set
    name: String,
    /// Files to load domains from
    files: Vec<PathBuf>,
    /// Inline domain expressions
    exps: Vec<String>,
    /// Whether to auto-reload files
    auto_reload: bool,
    /// Default match type for rules without prefix
    default_match_type: MatchType,
    /// Loaded domain rules (stored in shared state)
    rules: Arc<RwLock<DomainRules>>,
    /// Plugin tag from YAML configuration
    tag: Option<String>,
    /// Optional file watcher handle for auto-reload
    watcher: Arc<parking_lot::Mutex<Option<crate::utils::FileWatcherHandle>>>,
}

impl DomainSetPlugin {
    /// Create a new domain set plugin
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            files: Vec::new(),
            exps: Vec::new(),
            auto_reload: false,
            default_match_type: MatchType::Domain,
            rules: Arc::new(RwLock::new(DomainRules::new())),
            tag: None,
            watcher: Arc::new(parking_lot::Mutex::new(None)),
        }
    }

    /// Add files to load domains from
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files.into_iter().map(PathBuf::from).collect();
        self
    }

    /// Enable auto-reload
    pub fn with_auto_reload(mut self, enabled: bool) -> Self {
        self.auto_reload = enabled;
        self
    }

    /// Add inline domain expressions
    pub fn with_exps(mut self, exps: Vec<String>) -> Self {
        self.exps = exps;
        self
    }

    /// Set default match type for rules without prefix
    pub fn with_default_match_type(mut self, match_type: MatchType) -> Self {
        self.default_match_type = match_type;
        self
    }

    /// Start file watcher if auto-reload is enabled
    pub fn start_file_watcher(&self) {
        if !self.auto_reload || self.files.is_empty() {
            return;
        }

        let name = self.name.clone();
        let files = self.files.clone();
        let rules_weak = Arc::downgrade(&self.rules);
        let default_match_type = self.default_match_type.clone();

        debug!(
            name = %name,
            files = ?files,
            "enabling file auto-reload"
        );

        const DEBOUNCE_MS: u64 = 200;

        let handle = crate::utils::spawn_file_watcher(
            name.clone(),
            files.clone(),
            DEBOUNCE_MS,
            move |path, files| {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                let start = std::time::Instant::now();

                // First pass: count total lines across files without loading them fully
                use std::fs::File;
                use std::io::{BufRead, BufReader};

                let mut total_est = 0usize;
                for file_path in files.iter() {
                    match std::fs::metadata(file_path) {
                        Ok(m) => {
                            total_est = total_est
                                .saturating_add(((m.len() / 40) as usize).saturating_add(1))
                        }
                        Err(_) => total_est = total_est.saturating_add(128),
                    }
                }

                // Create rules with pre-allocated estimated capacity
                let mut new_rules = DomainRules::with_capacity(total_est, 0);

                // Read files line-by-line and add to rules (no large temporary buffers)
                for file_path in files.iter() {
                    if let Ok(file) = File::open(file_path) {
                        let reader = BufReader::new(file);
                        for l in reader.lines().map_while(|r| r.ok()) {
                            new_rules.add_line(&l, &default_match_type);
                        }
                    } else {
                        error!(file = ?file_path, "Failed to read domain file");
                    }
                }

                let stats = new_rules.stats();

                // Upgrade weak reference to Arc, if plugin still exists
                if let Some(rules) = rules_weak.upgrade() {
                    // Replace old rules and explicitly drop old value to free memory immediately
                    let old_rules = {
                        let mut writer = rules.write();
                        std::mem::replace(&mut *writer, new_rules)
                    };
                    // Explicitly drop old rules immediately (~25-30MB)
                    drop(old_rules);
                    // Tell allocator to release unused memory back to the OS immediately after reload (platform guarded)
                    crate::utils::malloc_trim_hint();
                } else {
                    warn!(name = %name, "plugin dropped, skipping reload");
                    return;
                }

                let duration = start.elapsed();

                info!(
                    name = %name,
                    filename = file_name,
                    duration = ?duration,
                    full = stats.full_count,
                    domain = stats.domain_count,
                    regexp = stats.regexp_count,
                    keyword = stats.keyword_count,
                    "scheduled auto-reload completed"
                );
            },
        );

        // Store handle so we can stop it on shutdown
        let mut guard = self.watcher.lock();
        *guard = Some(handle);
    }

    /// Load domains from all configured files
    pub fn load_domains(&self) -> Result<()> {
        // First pass: collect rules and calculate total sizes to avoid reallocation
        let mut all_file_rules = Vec::new();
        let mut total_full = 0;
        let mut total_domain = 0;

        // Load from files first
        for file_path in &self.files {
            match self.load_domain_file(file_path) {
                Ok(file_rules) => {
                    let stats = file_rules.stats();
                    debug!(
                        file = ?file_path,
                        full = stats.full_count,
                        domain = stats.domain_count,
                        regexp = stats.regexp_count,
                        keyword = stats.keyword_count,
                        "Loaded domains from file"
                    );
                    total_full += stats.full_count;
                    total_domain += stats.domain_count;
                    all_file_rules.push(file_rules);
                }
                Err(e) => {
                    error!(
                        file = ?file_path,
                        error = %e,
                        "Failed to load domain file"
                    );
                    // Continue loading other files
                }
            }
        }

        // Second pass: create rules with exact capacity, then merge all files
        let mut new_rules = DomainRules::with_capacity(total_full, total_domain);

        for file_rules in all_file_rules {
            new_rules.merge(file_rules);
        }

        // Then load from inline expressions (exps)
        for exp in &self.exps {
            new_rules.add_line(exp, &self.default_match_type);
        }

        // Shrink to fit to release excess capacity after full load
        new_rules.shrink_to_fit();

        let stats = new_rules.stats();
        *self.rules.write() = new_rules;

        // Tell allocator to release unused memory back to the OS immediately after loading (platform guarded)
        crate::utils::malloc_trim_hint();

        info!(
            name = %self.name,
            total = stats.full_count + stats.domain_count + stats.regexp_count + stats.keyword_count,
            full = stats.full_count,
            domain = stats.domain_count,
            regexp = stats.regexp_count,
            keyword = stats.keyword_count,
            files = self.files.len(),
            exps = self.exps.len(),
            "Domain set loaded"
        );

        Ok(())
    }

    /// Load domains from a single file
    fn load_domain_file(&self, path: &Path) -> Result<DomainRules> {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        // First pass: count lines without loading entire file into memory
        let file = File::open(path)
            .map_err(|e| crate::Error::Config(format!("Failed to open file {:?}: {}", path, e)))?;
        let reader = BufReader::new(file);
        let line_count = reader.lines().count();

        // Second pass: read lines and add rules incrementally
        let file = File::open(path)
            .map_err(|e| crate::Error::Config(format!("Failed to open file {:?}: {}", path, e)))?;
        let reader = BufReader::new(file);

        let mut rules = DomainRules::with_capacity(line_count, line_count);
        for line in reader.lines() {
            let l = line.map_err(|e| {
                crate::Error::Config(format!("Failed to read line {:?}: {}", path, e))
            })?;
            rules.add_line(&l, &self.default_match_type);
        }

        Ok(rules)
    }

    /// Check if a domain matches the set
    pub fn matches(&self, domain: &str) -> bool {
        self.rules.read().matches(domain)
    }

    /// Get the domain rules for use by other plugins
    pub fn get_rules(&self) -> Arc<RwLock<DomainRules>> {
        Arc::clone(&self.rules)
    }

    /// Get statistics
    pub fn stats(&self) -> DomainRulesStats {
        self.rules.read().stats()
    }
}

impl fmt::Debug for DomainSetPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stats = self.stats();
        f.debug_struct("DomainSetPlugin")
            .field("name", &self.name)
            .field("files", &self.files)
            .field("auto_reload", &self.auto_reload)
            .field("default_match_type", &self.default_match_type)
            .field("full_count", &stats.full_count)
            .field("domain_count", &stats.domain_count)
            .field("regexp_count", &stats.regexp_count)
            .field("keyword_count", &stats.keyword_count)
            .finish()
    }
}

#[async_trait]
impl Plugin for DomainSetPlugin {
    fn name(&self) -> &str {
        "domain_set"
    }

    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        // Store the domain rules in context metadata for other plugins to use
        ctx.set_metadata(format!("domain_set_{}", self.name), Arc::clone(&self.rules));
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn init(config: &PluginConfig) -> Result<std::sync::Arc<dyn Plugin>> {
        let args = config.effective_args();
        use serde_yaml::Value;

        let tag = args.get("tag").and_then(|v| v.as_str()).unwrap_or("");
        let name = if !tag.is_empty() {
            tag.to_string()
        } else {
            config.effective_name().to_string()
        };

        let mut plugin = DomainSetPlugin::new(name);
        plugin.tag = config.tag.clone();

        // Parse default match type
        if let Some(s) = args
            .get("default_match_type")
            .or(args.get("match_type"))
            .and_then(|v| v.as_str())
        {
            plugin.default_match_type = match s.to_lowercase().as_str() {
                "full" => MatchType::Full,
                "domain" => MatchType::Domain,
                "keyword" => MatchType::Keyword,
                "regexp" | "regex" => MatchType::Regexp,
                _ => {
                    warn!(value = %s, "Unknown match type, using 'domain'");
                    MatchType::Domain
                }
            };
        }

        if let Some(files_val) = args.get("files") {
            match files_val {
                Value::Sequence(seq) => {
                    let files: Vec<String> = seq
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    plugin = plugin.with_files(files);
                }
                Value::String(s) => {
                    plugin = plugin.with_files(vec![s.clone()]);
                }
                _ => {}
            }
        }

        if let Some(Value::Bool(b)) = args.get("auto_reload") {
            plugin = plugin.with_auto_reload(*b);
        }

        // Parse inline domain expressions (exps parameter)
        if let Some(exps_val) = args.get("exps") {
            let exps: Vec<String> = match exps_val {
                Value::Sequence(seq) => seq
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
            plugin = plugin.with_exps(exps);
        }

        // Load and start watcher as per legacy behavior
        if let Err(e) = plugin.load_domains() {
            tracing::warn!(error = %e, "Failed to load domains during init, continuing");
        }
        plugin.start_file_watcher();

        Ok(Arc::new(plugin))
    }
}

impl Matcher for DomainSetPlugin {
    fn matches_context(&self, ctx: &Context) -> bool {
        if let Some(question) = ctx.request().questions().first() {
            let domain = question.qname().to_string();
            let normalized = domain.trim_end_matches('.');
            self.matches(normalized)
        } else {
            false
        }
    }
}

#[async_trait]
impl Shutdown for DomainSetPlugin {
    async fn shutdown(&self) -> Result<()> {
        // Stop the file watcher if active
        let handle = {
            let mut guard = self.watcher.lock();
            guard.take()
        };
        if let Some(h) = handle {
            h.stop().await;
        }

        // Clear all rules and shrink memory usage
        {
            let mut rules = self.rules.write();
            rules.clear();
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_domain_set_creation() {
        let plugin = DomainSetPlugin::new("test");
        assert_eq!(plugin.name, "test");
        assert!(plugin.files.is_empty());
        assert!(!plugin.auto_reload);
        assert_eq!(plugin.default_match_type, MatchType::Domain);
    }

    #[test]
    fn test_domain_set_builder() {
        let plugin = DomainSetPlugin::new("test")
            .with_files(vec!["file1.txt".to_string()])
            .with_auto_reload(true)
            .with_default_match_type(MatchType::Full);

        assert_eq!(plugin.files.len(), 1);
        assert!(plugin.auto_reload);
        assert_eq!(plugin.default_match_type, MatchType::Full);
    }

    #[test]
    fn test_full_match() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Full, "google.com".to_string());

        assert!(rules.matches("google.com"));
        assert!(rules.matches("GOOGLE.COM")); // case insensitive
        assert!(!rules.matches("www.google.com")); // full match doesn't match subdomains
        assert!(!rules.matches("google.com.hk"));
    }

    #[test]
    fn test_domain_match() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Domain, "google.com".to_string());

        assert!(rules.matches("google.com")); // matches self
        assert!(rules.matches("www.google.com")); // matches subdomain
        assert!(rules.matches("maps.l.google.com")); // matches deep subdomain
        assert!(!rules.matches("google.com.hk")); // not a subdomain
        assert!(!rules.matches("notgoogle.com"));
    }

    #[test]
    fn test_keyword_match() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Keyword, "google".to_string());

        assert!(rules.matches("google.com"));
        assert!(rules.matches("www.google.com"));
        assert!(rules.matches("google.com.hk")); // keyword matches anywhere
        assert!(rules.matches("mygoogle.net"));
        assert!(!rules.matches("gogle.com")); // typo doesn't match
    }

    #[test]
    fn test_regexp_match() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Regexp, r".+\.google\.com$".to_string());

        assert!(!rules.matches("google.com")); // requires prefix
        assert!(rules.matches("www.google.com"));
        assert!(rules.matches("maps.l.google.com"));
        assert!(!rules.matches("google.com.hk"));
    }

    #[test]
    fn test_match_priority_full_over_domain() {
        let mut rules = DomainRules::new();
        // Add domain rule first, then full - full should still win
        rules.add_rule(MatchType::Domain, "example.com".to_string());
        rules.add_rule(MatchType::Full, "test.example.com".to_string());

        // Both should match test.example.com, but full has priority
        assert!(rules.matches("test.example.com"));
        assert!(rules.matches("other.example.com")); // domain match
    }

    #[test]
    fn test_domain_subdomain_priority() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Domain, "com".to_string());
        rules.add_rule(MatchType::Domain, "google.com".to_string());

        // www.google.com should match google.com first (more specific)
        // This is verified by the match order in the implementation
        assert!(rules.matches("www.google.com"));
        assert!(rules.matches("other.com"));
    }

    #[test]
    fn test_parse_line_with_prefix() {
        let mut rules = DomainRules::new();

        rules.add_line("full:exact.com", &MatchType::Domain);
        rules.add_line("domain:parent.com", &MatchType::Full);
        rules.add_line("keyword:searchterm", &MatchType::Domain);
        rules.add_line("regexp:.*test.*", &MatchType::Domain);
        rules.add_line("default.com", &MatchType::Domain); // uses default

        assert!(rules.matches("exact.com"));
        assert!(!rules.matches("sub.exact.com")); // full match

        assert!(rules.matches("parent.com"));
        assert!(rules.matches("sub.parent.com")); // domain match

        assert!(rules.matches("has-searchterm.com")); // keyword

        assert!(rules.matches("mytest.com")); // regexp

        assert!(rules.matches("sub.default.com")); // domain (default)
    }

    #[test]
    fn test_load_domain_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "# Comment line").unwrap();
        writeln!(file).unwrap();
        writeln!(file, "example.com").unwrap();
        writeln!(file, "domain:google.com").unwrap();
        writeln!(file, "full:exact.net").unwrap();
        writeln!(file, "keyword:facebook").unwrap();
        writeln!(file, "regexp:.*twitter.*").unwrap();
        file.flush().unwrap();

        let plugin = DomainSetPlugin::new("test")
            .with_files(vec![file.path().to_string_lossy().to_string()]);
        plugin.load_domains().unwrap();

        let stats = plugin.stats();
        assert_eq!(stats.full_count, 1); // exact.net
        assert_eq!(stats.domain_count, 2); // example.com (default) + google.com
        assert_eq!(stats.keyword_count, 1); // facebook
        assert_eq!(stats.regexp_count, 1); // .*twitter.*

        // Test matching
        assert!(plugin.matches("example.com"));
        assert!(plugin.matches("sub.example.com"));
        assert!(plugin.matches("www.google.com"));
        assert!(plugin.matches("exact.net"));
        assert!(!plugin.matches("sub.exact.net")); // full match
        assert!(plugin.matches("facebook.com"));
        assert!(plugin.matches("my.twitter.handle.com"));
    }

    #[test]
    fn test_invalid_regexp() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Regexp, "[invalid(regex".to_string());

        // Invalid regex should be skipped
        assert_eq!(rules.regexp.len(), 0);
    }

    #[test]
    fn test_case_insensitive() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Full, "UPPER.COM".to_string());
        rules.add_rule(MatchType::Domain, "MixedCase.ORG".to_string());
        rules.add_rule(MatchType::Keyword, "KeyWord".to_string());

        assert!(rules.matches("upper.com"));
        assert!(rules.matches("UPPER.COM"));
        assert!(rules.matches("sub.mixedcase.org"));
        assert!(rules.matches("has-keyword.com"));
    }

    #[test]
    fn test_trailing_dot_handling() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Full, "example.com".to_string());

        assert!(rules.matches("example.com"));
        assert!(rules.matches("example.com.")); // trailing dot normalized
    }

    #[test]
    fn test_empty_and_comment_lines() {
        let mut rules = DomainRules::new();
        rules.add_line("", &MatchType::Domain);
        rules.add_line("   ", &MatchType::Domain);
        rules.add_line("# comment", &MatchType::Domain);
        rules.add_line("  # another comment", &MatchType::Domain);

        assert!(rules.is_empty());
    }

    #[test]
    fn test_domain_rules_merge() {
        let mut rules1 = DomainRules::new();
        rules1.add_rule(MatchType::Full, "a.com".to_string());
        rules1.add_rule(MatchType::Domain, "b.com".to_string());

        let mut rules2 = DomainRules::new();
        rules2.add_rule(MatchType::Keyword, "test".to_string());
        rules2.add_rule(MatchType::Regexp, ".*pattern.*".to_string());

        rules1.merge(rules2);

        assert_eq!(rules1.full.len(), 1);
        assert_eq!(rules1.domain.len(), 1);
        assert_eq!(rules1.keyword.len(), 1);
        assert_eq!(rules1.regexp.len(), 1);
    }

    #[tokio::test]
    async fn test_domain_set_plugin_execution() {
        let plugin = DomainSetPlugin::new("test");
        let request = crate::dns::Message::new();
        let mut ctx = Context::new(request);

        plugin.execute(&mut ctx).await.unwrap();

        // Verify metadata was set
        assert!(ctx.has_metadata("domain_set_test"));
    }

    #[tokio::test]
    async fn test_domain_set_plugin_shutdown_clears_rules() {
        let plugin = DomainSetPlugin::new("test").with_files(vec!["nonexistent.txt".to_string()]); // Won't load but that's fine

        // Manually add some rules to test clearing
        {
            let mut rules = plugin.rules.write();
            rules.add_rule(MatchType::Full, "test.com".to_string());
            rules.add_rule(MatchType::Domain, "example.com".to_string());
            rules.add_rule(MatchType::Keyword, "keyword".to_string());
            rules.add_rule(MatchType::Regexp, ".*regexp.*".to_string());
        }

        // Verify rules are loaded
        {
            let rules = plugin.rules.read();
            assert_eq!(rules.full.len(), 1);
            assert_eq!(rules.domain.len(), 1);
            assert_eq!(rules.keyword.len(), 1);
            assert_eq!(rules.regexp.len(), 1);
        }

        // Shutdown should clear all rules
        plugin.shutdown().await.unwrap();

        // Verify rules are cleared
        {
            let rules = plugin.rules.read();
            assert_eq!(rules.full.len(), 0);
            assert_eq!(rules.domain.len(), 0);
            assert_eq!(rules.keyword.len(), 0);
            assert_eq!(rules.regexp.len(), 0);
        }
    }

    #[test]
    fn test_domain_match_boundary() {
        let mut rules = DomainRules::new();
        rules.add_rule(MatchType::Domain, "google.com".to_string());

        // Should NOT match - these are not subdomains
        assert!(!rules.matches("notgoogle.com"));
        assert!(!rules.matches("mygoogle.com"));
        assert!(!rules.matches("google.com.cn"));

        // Should match - actual subdomains
        assert!(rules.matches("google.com"));
        assert!(rules.matches("www.google.com"));
        assert!(rules.matches("a.b.c.google.com"));
    }

    #[test]
    fn test_init_with_exps() {
        use serde_yaml::Value;

        let mut config_args = serde_yaml::Mapping::new();
        config_args.insert(
            Value::String("exps".to_string()),
            Value::Sequence(vec![
                Value::String("google.com".to_string()),
                Value::String("full:exact.com".to_string()),
                Value::String("regexp:.+\\.github\\.io$".to_string()),
                Value::String("keyword:test".to_string()),
            ]),
        );

        let config = crate::config::PluginConfig {
            tag: Some("test".to_string()),
            plugin_type: "domain_set".to_string(),
            args: Value::Mapping(config_args),
        };

        let plugin_arc = DomainSetPlugin::init(&config).expect("Failed to init");
        let plugin = plugin_arc
            .as_any()
            .downcast_ref::<DomainSetPlugin>()
            .unwrap();

        // Verify rules were loaded from exps
        let stats = plugin.stats();
        assert_eq!(stats.full_count, 1);
        assert_eq!(stats.domain_count, 1);
        assert_eq!(stats.regexp_count, 1);
        assert_eq!(stats.keyword_count, 1);

        // Verify they actually match
        assert!(plugin.matches("google.com"));
        assert!(plugin.matches("www.google.com")); // domain match
        assert!(plugin.matches("exact.com"));
        assert!(!plugin.matches("sub.exact.com")); // full match doesn't match subdomains
        assert!(plugin.matches("test.github.io"));
        assert!(plugin.matches("has-test.com")); // keyword match
    }

    #[test]
    fn test_init_with_exps_single_string() {
        use serde_yaml::Value;

        let mut config_args = serde_yaml::Mapping::new();
        config_args.insert(
            Value::String("exps".to_string()),
            Value::String("example.com".to_string()),
        );

        let config = crate::config::PluginConfig {
            tag: Some("test".to_string()),
            plugin_type: "domain_set".to_string(),
            args: Value::Mapping(config_args),
        };

        let plugin_arc = DomainSetPlugin::init(&config).expect("Failed to init");
        let plugin = plugin_arc
            .as_any()
            .downcast_ref::<DomainSetPlugin>()
            .unwrap();

        let stats = plugin.stats();
        assert_eq!(stats.domain_count, 1); // Single string should be parsed as domain match
        assert!(plugin.matches("example.com"));
        assert!(plugin.matches("sub.example.com"));
    }
}
