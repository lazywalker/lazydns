//! Domain Validator Plugin
//!
//! Validates DNS query domain names for RFC 1035/1123 compliance and rejects
//! malformed queries early, reducing upstream load and improving robustness.

use crate::RegisterPlugin;
use crate::Result;
use crate::dns::ResponseCode;
use crate::dns::types::RecordType;
use crate::plugin::{Context, Plugin};
use async_trait::async_trait;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Validation result
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValidationResult {
    Valid,
    InvalidChars,
    InvalidLength,
    InvalidFormat,
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
            return ValidationResult::InvalidLength;
        }

        // Allow root domain
        if domain == "." {
            return ValidationResult::Valid;
        }

        let labels: Vec<&str> = domain.split('.').collect();

        for label in labels {
            if label.is_empty() || label.len() > 63 {
                return ValidationResult::InvalidLength;
            }

            // Check characters
            let bytes = label.as_bytes();
            if bytes.is_empty() {
                return ValidationResult::InvalidLength;
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
                    return ValidationResult::InvalidChars;
                }
            }

            // Last character must be alphanumeric
            let last = bytes[bytes.len() - 1];
            if !last.is_ascii_alphanumeric() {
                return ValidationResult::InvalidChars;
            }

            // Middle characters: alphanumeric or hyphen (only if there are middle characters)
            if bytes.len() > 2 {
                for &b in &bytes[1..bytes.len() - 1] {
                    if !b.is_ascii_alphanumeric() && b != b'-' {
                        return ValidationResult::InvalidChars;
                    }
                }
            }

            // No consecutive hyphens in strict mode, except for valid Punycode
            // A-labels (RFC 5890): "xn--<punycode>". These legitimately
            // contain "--" right after the prefix and represent internationalized
            // domain names; rejecting them would refuse all IDN queries.
            let is_punycode_label =
                label.len() >= 5 && label.as_bytes()[..4].eq_ignore_ascii_case(b"xn--");
            if self.strict_mode && label.contains("--") && !is_punycode_label {
                return ValidationResult::InvalidFormat;
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
                return handle_result(*result, &qname, ctx).await;
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
                ValidationResult::InvalidChars => "invalid_chars",
                ValidationResult::InvalidLength => "invalid_length",
                ValidationResult::InvalidFormat => "invalid_format",
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
                // Track cache size before put to detect evictions
                let size_before = cache.len();
                let evicted = cache.put(qname.clone(), result);
                let size_after = cache.len();

                // Increment eviction counter if:
                // 1. put() explicitly returned Some (key override case), OR
                // 2. cache was at capacity before and size didn't increase (new key evicted old)
                if evicted.is_some() {
                    crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_EVICTIONS_TOTAL.inc();
                } else if size_before >= 100 && size_after == size_before {
                    // Cache was full, and size didn't increase = an eviction must have occurred
                    crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_EVICTIONS_TOTAL.inc();
                }

                crate::metrics::DNS_DOMAIN_VALIDATION_CACHE_SIZE.set(size_after as i64);
            }

            #[cfg(not(feature = "metrics"))]
            {
                // No metrics enabled: just insert into cache
                cache.put(qname.clone(), result);
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
        ValidationResult::InvalidChars => {
            debug!("Rejected domain with invalid characters: {}", qname);

            #[cfg(feature = "web")]
            crate::plugins::AUDIT_LOGGER
                .log_security_event(
                    crate::plugins::SecurityEventType::MalformedQuery,
                    format!("Domain with invalid characters rejected: {}", qname),
                    ctx.get_metadata::<std::net::IpAddr>("client_ip").copied(),
                    Some(qname.to_string()),
                )
                .await;

            set_refused_response(ctx);
            Ok(())
        }
        ValidationResult::InvalidLength => {
            debug!("Rejected domain with invalid length: {}", qname);

            #[cfg(feature = "web")]
            crate::plugins::AUDIT_LOGGER
                .log_security_event(
                    crate::plugins::SecurityEventType::MalformedQuery,
                    format!("Domain with invalid length rejected: {}", qname),
                    ctx.get_metadata::<std::net::IpAddr>("client_ip").copied(),
                    Some(qname.to_string()),
                )
                .await;

            set_refused_response(ctx);
            Ok(())
        }
        ValidationResult::InvalidFormat => {
            debug!("Rejected domain with invalid format: {}", qname);

            #[cfg(feature = "web")]
            crate::plugins::AUDIT_LOGGER
                .log_security_event(
                    crate::plugins::SecurityEventType::MalformedQuery,
                    format!("Domain with invalid format rejected: {}", qname),
                    ctx.get_metadata::<std::net::IpAddr>("client_ip").copied(),
                    Some(qname.to_string()),
                )
                .await;

            set_refused_response(ctx);
            Ok(())
        }
    }
}

fn set_refused_response(ctx: &mut Context) {
    let mut response = crate::dns::Message::new();
    response.set_id(ctx.request().id());
    response.set_response(true);
    response.set_response_code(ResponseCode::Refused);
    ctx.set_response(Some(response));
}

impl Default for DomainValidatorPlugin {
    fn default() -> Self {
        Self::new(true, 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            plugin.validate_domain("test space.com"),
            ValidationResult::InvalidChars
        );
        assert_eq!(
            plugin.validate_domain("test@domain.com"),
            ValidationResult::InvalidChars
        );
        assert_eq!(
            plugin.validate_domain("-test.com"),
            ValidationResult::InvalidChars
        );
        assert_eq!(
            plugin.validate_domain("test-.com"),
            ValidationResult::InvalidChars
        );
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
        assert_eq!(
            plugin.validate_domain(&long_label),
            ValidationResult::InvalidLength
        );
        let long_domain = "a.".repeat(126) + "com";
        assert_eq!(
            plugin.validate_domain(&long_domain),
            ValidationResult::InvalidLength
        );
    }

    #[tokio::test]
    async fn test_strict_mode() {
        let strict_plugin = DomainValidatorPlugin::new(true, 1000);
        assert_eq!(
            strict_plugin.validate_domain("te--st.com"),
            ValidationResult::InvalidFormat
        );

        let lenient_plugin = DomainValidatorPlugin::new(false, 1000);
        assert_eq!(
            lenient_plugin.validate_domain("te--st.com"),
            ValidationResult::Valid
        );
    }

    /// Punycode "A-labels" (RFC 5890) encode internationalized domain names
    /// and legitimately contain "--" after the "xn-" prefix. Strict mode must
    /// not reject them; otherwise all IDN queries (such as Chinese/Arabic domains)
    /// would be refused under the default configuration.
    #[tokio::test]
    async fn test_strict_mode_allows_punycode_labels() {
        let strict_plugin = DomainValidatorPlugin::new(true, 1000);
        assert_eq!(
            strict_plugin.validate_domain("xn--fsq.com"),
            ValidationResult::Valid
        );
        // Mixed case prefix is also valid.
        assert_eq!(
            strict_plugin.validate_domain("XN--FSQ.com"),
            ValidationResult::Valid
        );
        // Non-punycode labels with "--" are still rejected.
        assert_eq!(
            strict_plugin.validate_domain("te--st.com"),
            ValidationResult::InvalidFormat
        );
    }

    #[tokio::test]
    async fn test_cache() {
        use crate::dns::{Message, Question, RecordClass, RecordType};

        let plugin = DomainValidatorPlugin::new(true, 10);

        // Create a test request
        let mut request = Message::new();
        request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        let mut ctx = Context::new(request);

        // First execution
        let result = plugin.execute(&mut ctx).await;
        assert!(result.is_ok());
        assert!(ctx.response().is_none()); // Valid domain, no response set

        // Check cache
        {
            let cache = plugin.cache.write().await;
            assert!(cache.contains("example.com"));
        }
    }

    #[tokio::test]
    async fn test_consecutive_dots() {
        let plugin = DomainValidatorPlugin::default();
        // Consecutive dots result in empty labels
        assert_eq!(
            plugin.validate_domain("example..com"),
            ValidationResult::InvalidLength
        );
        assert_eq!(
            plugin.validate_domain("sub..domain.example.com"),
            ValidationResult::InvalidLength
        );
        assert_eq!(
            plugin.validate_domain("..."),
            ValidationResult::InvalidLength
        );
    }

    #[tokio::test]
    async fn test_domains_starting_with_dot() {
        let plugin = DomainValidatorPlugin::default();
        // Domains starting with dot have empty first label (except root ".")
        assert_eq!(
            plugin.validate_domain(".example.com"),
            ValidationResult::InvalidLength
        );
        assert_eq!(
            plugin.validate_domain(".com"),
            ValidationResult::InvalidLength
        );
    }

    #[tokio::test]
    async fn test_domains_ending_with_dot() {
        let plugin = DomainValidatorPlugin::default();
        // Domains ending with dot have empty last label
        assert_eq!(
            plugin.validate_domain("example.com."),
            ValidationResult::InvalidLength
        );
        assert_eq!(
            plugin.validate_domain("localhost."),
            ValidationResult::InvalidLength
        );
    }

    #[tokio::test]
    async fn test_empty_string() {
        let plugin = DomainValidatorPlugin::default();
        // Empty string should be invalid
        assert_eq!(plugin.validate_domain(""), ValidationResult::InvalidLength);
    }
}
