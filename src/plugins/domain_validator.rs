//! Domain Validator Plugin
//!
//! Validates DNS query domain names for RFC 1035/1123 compliance and rejects
//! malformed queries early, reducing upstream load and improving robustness.

use crate::RegisterPlugin;
use crate::Result;
use crate::dns::types::RecordType;
use crate::plugin::{Context, Plugin};
use async_trait::async_trait;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Validation result
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationResult {
    Valid,
    Invalid(String),
}

/// Domain validator plugin
#[derive(Debug, RegisterPlugin)]
pub struct DomainValidatorPlugin {
    /// Enable strict RFC compliance mode
    strict_mode: bool,
    /// LRU cache for validation results
    cache: Arc<RwLock<LruCache<String, ValidationResult>>>,
}

impl DomainValidatorPlugin {
    /// Create a new domain validator
    pub fn new(strict_mode: bool, cache_size: usize) -> Self {
        let cache = if cache_size > 0 {
            LruCache::new(NonZeroUsize::new(cache_size).unwrap())
        } else {
            LruCache::new(NonZeroUsize::new(1).unwrap()) // Minimal cache
        };

        // Initialize metrics if metrics enabled: set current size
        #[cfg(feature = "metrics")]
        {
            crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_SIZE.set(cache.len() as i64);
        }

        Self {
            strict_mode,
            cache: Arc::new(RwLock::new(cache)),
        }
    }

    /// Validate a domain name (legacy API)
    ///
    /// This keeps backwards compatibility with unit tests and callers that don't
    /// know the question qtype. It calls into `validate_domain_with_qtype` with
    /// `None` which enforces the original strict rules.
    pub fn validate_domain(&self, domain: &str) -> ValidationResult {
        self.validate_domain_with_qtype(domain, None)
    }

    /// Validate a domain name with optional query type context
    ///
    /// When `qtype` is `Some(RecordType::SVCB)` or `Some(RecordType::HTTPS)`, a
    /// label starting with `_` (underscore) is accepted as the first character
    /// which is commonly used for service name labels (such as `_dns.resolver.arpa`).
    pub fn validate_domain_with_qtype(
        &self,
        domain: &str,
        qtype: Option<RecordType>,
    ) -> ValidationResult {
        // Basic checks
        if domain.is_empty() || domain.len() > 253 {
            return ValidationResult::Invalid("invalid length".into());
        }

        // Allow root domain
        if domain == "." {
            return ValidationResult::Valid;
        }

        let labels: Vec<&str> = domain.split('.').collect();

        for label in labels {
            if label.is_empty() || label.len() > 63 {
                return ValidationResult::Invalid("invalid length".into());
            }

            let bytes = label.as_bytes();
            if bytes.is_empty() {
                return ValidationResult::Invalid("invalid length".into());
            }

            // First character must be alphanumeric, except when qtype indicates
            // a service-binding record query (SVCB/HTTPS) and the label starts
            // with an underscore (such as `_dns`).
            let first = bytes[0];
            if !first.is_ascii_alphanumeric() {
                let allow_underscore = (first == b'_')
                    && qtype
                        .map(|t| matches!(t, RecordType::SVCB | RecordType::HTTPS))
                        .unwrap_or(false);
                if !allow_underscore {
                    return ValidationResult::Invalid("invalid characters".into());
                }
            }

            // Last character must be alphanumeric
            let last = bytes[bytes.len() - 1];
            if !last.is_ascii_alphanumeric() {
                return ValidationResult::Invalid("invalid characters".into());
            }

            // Middle characters: alphanumeric or hyphen
            if bytes.len() > 2 {
                for &b in &bytes[1..bytes.len() - 1] {
                    if !b.is_ascii_alphanumeric() && b != b'-' {
                        return ValidationResult::Invalid("invalid characters".into());
                    }
                }
            }

            // No consecutive hyphens in strict mode, except for Punycode A-labels
            // (RFC 5890): "xn--<punycode>" legitimately contains "--".
            let is_punycode_label =
                label.len() >= 5 && label.as_bytes()[..4].eq_ignore_ascii_case(b"xn--");
            if self.strict_mode && label.contains("--") && !is_punycode_label {
                return ValidationResult::Invalid("invalid format".into());
            }
        }

        ValidationResult::Valid
    }
}

#[async_trait]
impl Plugin for DomainValidatorPlugin {
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        #[cfg(feature = "metrics")]
        let start = std::time::Instant::now();
        let qname = ctx
            .request()
            .questions()
            .first()
            .map(|q| q.qname().to_string())
            .unwrap_or_default();

        // Check cache first. Use write lock + get() so cache hits update LRU recency
        // and hot items are kept in the cache. This trades some write contention for
        // correct LRU behavior under heavy hit workloads.
        {
            let mut cache = self.cache.write().await;
            if let Some(result) = cache.get(&qname) {
                #[cfg(feature = "metrics")]
                {
                    crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_HITS_TOTAL.inc();
                    let duration = start.elapsed().as_secs_f64();
                    crate::metrics::DNS_DOMAIN_VALIDATION_DURATION_SECONDS.observe(duration);
                }
                return handle_result(result.clone(), &qname, ctx).await;
            } else {
                // Cache miss - record it
                #[cfg(feature = "metrics")]
                {
                    crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_MISSES_TOTAL.inc();
                }
            }
        }

        // Validate (pass qtype so we can allow service name labels like `_dns` for SVCB/HTTPS)
        let qtype = ctx.request().questions().first().map(|q| q.qtype());
        let result = self.validate_domain_with_qtype(&qname, qtype);

        // Record metrics
        #[cfg(feature = "metrics")]
        {
            let result_label = match &result {
                ValidationResult::Valid => "valid",
                ValidationResult::Invalid(r) => match r.as_str() {
                    "invalid characters" => "invalid_chars",
                    "invalid length" => "invalid_length",
                    "invalid format" => "invalid_format",
                    _ => "invalid",
                },
            };
            crate::metrics::DNS_DOMAIN_VALIDATION_TOTAL
                .with_label_values(&[result_label])
                .inc();
        }

        // Cache result (update cache size metric after mutation, count evictions)
        {
            let mut cache = self.cache.write().await;

            #[cfg(feature = "metrics")]
            {
                let size_before = cache.len();
                let evicted = cache.put(qname.clone(), result.clone());
                let size_after = cache.len();

                if evicted.is_some() || (size_before >= 100 && size_after == size_before) {
                    crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_EVICTIONS_TOTAL.inc();
                }

                crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_SIZE.set(size_after as i64);
            }

            #[cfg(not(feature = "metrics"))]
            {
                cache.put(qname.clone(), result.clone());
            }
        }

        #[cfg(feature = "metrics")]
        {
            let duration = start.elapsed().as_secs_f64();
            crate::metrics::DNS_DOMAIN_VALIDATION_DURATION_SECONDS.observe(duration);
        }

        handle_result(result, &qname, ctx).await
    }

    fn name(&self) -> &str {
        "domain_validator"
    }

    fn init(config: &crate::config::PluginConfig) -> Result<Arc<dyn Plugin>> {
        let args = config.effective_args();
        let strict_mode = args
            .get("strict_mode")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let cache_size = args
            .get("cache_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000) as usize;

        Ok(Arc::new(Self::new(strict_mode, cache_size)))
    }
}

async fn handle_result(result: ValidationResult, qname: &str, ctx: &mut Context) -> Result<()> {
    match result {
        ValidationResult::Valid => Ok(()),
        ValidationResult::Invalid(reason) => {
            debug!("Rejected domain ({}): {}", reason, qname);

            #[cfg(feature = "web")]
            crate::plugins::AUDIT_LOGGER
                .log_security_event(
                    crate::plugins::SecurityEventType::MalformedQuery,
                    format!("Domain rejected ({}): {}", reason, qname),
                    ctx.get_metadata::<std::net::IpAddr>("client_ip").copied(),
                    Some(qname.to_string()),
                )
                .await;

            ctx.set_refused();
            Ok(())
        }
    }
}

impl Default for DomainValidatorPlugin {
    fn default() -> Self {
        Self::new(true, 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid_chars() -> ValidationResult {
        ValidationResult::Invalid("invalid characters".into())
    }
    fn invalid_length() -> ValidationResult {
        ValidationResult::Invalid("invalid length".into())
    }
    fn invalid_format() -> ValidationResult {
        ValidationResult::Invalid("invalid format".into())
    }

    #[tokio::test]
    async fn test_valid_domains() {
        let plugin = DomainValidatorPlugin::default();
        assert_eq!(
            plugin.validate_domain("example.com"),
            ValidationResult::Valid
        );
        assert_eq!(
            plugin.validate_domain("sub.example.co.uk"),
            ValidationResult::Valid
        );
        assert_eq!(plugin.validate_domain("localhost"), ValidationResult::Valid);
        assert_eq!(plugin.validate_domain("."), ValidationResult::Valid);
    }

    #[tokio::test]
    async fn test_invalid_chars() {
        let plugin = DomainValidatorPlugin::default();
        assert_eq!(plugin.validate_domain("test space.com"), invalid_chars());
        assert_eq!(plugin.validate_domain("test@domain.com"), invalid_chars());
        assert_eq!(plugin.validate_domain("-test.com"), invalid_chars());
        assert_eq!(plugin.validate_domain("test-.com"), invalid_chars());
    }

    #[tokio::test]
    async fn test_single_char_labels() {
        let plugin = DomainValidatorPlugin::default();
        assert_eq!(plugin.validate_domain("a.com"), ValidationResult::Valid);
        assert_eq!(plugin.validate_domain("a.b.com"), ValidationResult::Valid);
        assert_eq!(plugin.validate_domain("x.y.z"), ValidationResult::Valid);
    }

    #[tokio::test]
    async fn test_invalid_length() {
        let plugin = DomainValidatorPlugin::default();
        let long_label = "a".repeat(64) + ".com";
        assert_eq!(plugin.validate_domain(&long_label), invalid_length());
        let long_domain = "a.".repeat(126) + "com";
        assert_eq!(plugin.validate_domain(&long_domain), invalid_length());
    }

    #[tokio::test]
    async fn test_strict_mode() {
        let strict_plugin = DomainValidatorPlugin::new(true, 1000);
        assert_eq!(
            strict_plugin.validate_domain("te--st.com"),
            invalid_format()
        );

        let lenient_plugin = DomainValidatorPlugin::new(false, 1000);
        assert_eq!(
            lenient_plugin.validate_domain("te--st.com"),
            ValidationResult::Valid
        );
    }

    // Punycode A-labels (RFC 5890) legitimately contain "--" after "xn-".
    #[tokio::test]
    async fn test_strict_mode_allows_punycode_labels() {
        let strict_plugin = DomainValidatorPlugin::new(true, 1000);
        assert_eq!(
            strict_plugin.validate_domain("xn--fsq.com"),
            ValidationResult::Valid
        );
        assert_eq!(
            strict_plugin.validate_domain("XN--FSQ.com"),
            ValidationResult::Valid
        );
        assert_eq!(
            strict_plugin.validate_domain("te--st.com"),
            invalid_format()
        );
    }

    #[tokio::test]
    async fn test_cache() {
        use crate::dns::{Message, Question, RecordClass, RecordType};

        let plugin = DomainValidatorPlugin::new(true, 10);

        let mut request = Message::new();
        request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);

        let result = plugin.execute(&mut ctx).await;
        assert!(result.is_ok());
        assert!(ctx.response().is_none());

        {
            let cache = plugin.cache.write().await;
            assert!(cache.contains("example.com"));
        }
    }

    #[tokio::test]
    async fn test_consecutive_dots() {
        let plugin = DomainValidatorPlugin::default();
        assert_eq!(plugin.validate_domain("example..com"), invalid_length());
        assert_eq!(
            plugin.validate_domain("sub..domain.example.com"),
            invalid_length()
        );
        assert_eq!(plugin.validate_domain("..."), invalid_length());
    }

    #[tokio::test]
    async fn test_domains_starting_with_dot() {
        let plugin = DomainValidatorPlugin::default();
        assert_eq!(plugin.validate_domain(".example.com"), invalid_length());
        assert_eq!(plugin.validate_domain(".com"), invalid_length());
    }

    #[tokio::test]
    async fn test_domains_ending_with_dot() {
        let plugin = DomainValidatorPlugin::default();
        assert_eq!(plugin.validate_domain("example.com."), invalid_length());
        assert_eq!(plugin.validate_domain("localhost."), invalid_length());
    }

    #[tokio::test]
    async fn test_empty_string() {
        let plugin = DomainValidatorPlugin::default();
        assert_eq!(plugin.validate_domain(""), invalid_length());
    }
}
