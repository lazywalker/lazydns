//! Audit event types

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use time::OffsetDateTime;

/// Get current timestamp as ISO 8601 string in local timezone
fn now_timestamp() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let format = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3][offset_hour sign:mandatory]:[offset_minute]"
    );
    now.format(&format).unwrap_or_else(|_| {
        // Fallback to a simpler format if formatting fails
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        )
    })
}

/// Security event types for audit logging
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityEventType {
    /// Client exceeded rate limit
    RateLimitExceeded,
    /// Query for a blocked domain
    BlockedDomainQuery,
    /// Upstream DNS server failure
    UpstreamFailure,
    /// Query denied by ACL
    AclDenied,
    /// Malformed DNS query received
    MalformedQuery,
    /// Query timeout
    QueryTimeout,
}

impl SecurityEventType {
    /// Get the string name of this event type
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RateLimitExceeded => "rate_limit_exceeded",
            Self::BlockedDomainQuery => "blocked_domain_query",
            Self::UpstreamFailure => "upstream_failure",
            Self::AclDenied => "acl_denied",
            Self::MalformedQuery => "malformed_query",
            Self::QueryTimeout => "query_timeout",
        }
    }

    /// Parse from string
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rate_limit_exceeded" => Some(Self::RateLimitExceeded),
            "blocked_domain_query" => Some(Self::BlockedDomainQuery),
            "upstream_failure" => Some(Self::UpstreamFailure),
            "acl_denied" => Some(Self::AclDenied),
            "malformed_query" => Some(Self::MalformedQuery),
            "query_timeout" => Some(Self::QueryTimeout),
            _ => None,
        }
    }
}

/// Query log entry for audit logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryLogEntry {
    /// Timestamp of the query (ISO 8601 format)
    pub timestamp: String,

    /// Query ID from DNS header
    pub query_id: u16,

    /// Client IP address (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<IpAddr>,

    /// Protocol used (udp, tcp, tls, doh, doq)
    pub protocol: String,

    /// Query name (domain being queried)
    pub qname: String,

    /// Query type (A, AAAA, MX)
    pub qtype: String,

    /// Query class (usually IN)
    pub qclass: String,

    /// Response code (if response available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rcode: Option<String>,

    /// Number of answers in response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_count: Option<usize>,

    /// Response time in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_time_ms: Option<u64>,

    /// Response time in microseconds (for higher precision latency tracking)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_time_us: Option<u64>,

    /// Whether response was served from cache
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,

    /// Upstream server used (if forwarded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,

    /// Answer IPs (for A/AAAA queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<Vec<String>>,
}

impl QueryLogEntry {
    /// Create a new query log entry with minimal information
    pub fn new(
        query_id: u16,
        protocol: &str,
        qname: String,
        qtype: String,
        qclass: String,
    ) -> Self {
        Self {
            timestamp: now_timestamp(),
            query_id,
            client_ip: None,
            protocol: protocol.to_string(),
            qname,
            qtype,
            qclass,
            rcode: None,
            answer_count: None,
            response_time_ms: None,
            response_time_us: None,
            cached: None,
            upstream: None,
            answers: None,
        }
    }

    /// Set client IP
    pub fn with_client_ip(mut self, ip: IpAddr) -> Self {
        self.client_ip = Some(ip);
        self
    }

    /// Set response details
    pub fn with_response(
        mut self,
        rcode: &str,
        answer_count: usize,
        response_time_ms: u64,
    ) -> Self {
        self.rcode = Some(rcode.to_string());
        self.answer_count = Some(answer_count);
        self.response_time_ms = Some(response_time_ms);
        self
    }

    /// Set response time in microseconds for high precision latency tracking
    pub fn with_response_time_us(mut self, response_time_us: u64) -> Self {
        self.response_time_us = Some(response_time_us);
        self
    }

    /// Set cache status
    pub fn with_cached(mut self, cached: bool) -> Self {
        self.cached = Some(cached);
        self
    }

    /// Set upstream server
    pub fn with_upstream(mut self, upstream: &str) -> Self {
        self.upstream = Some(upstream.to_string());
        self
    }

    /// Set answer IPs
    pub fn with_answers(mut self, answers: Vec<String>) -> Self {
        self.answers = Some(answers);
        self
    }

    /// Format as JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Format as text log line
    pub fn to_text(&self) -> String {
        let mut parts = vec![self.timestamp.clone(), format!("id={}", self.query_id)];

        if let Some(ip) = self.client_ip {
            parts.push(format!("client={}", ip));
        }

        parts.push(format!("proto={}", self.protocol));
        parts.push(format!("qname={}", self.qname));
        parts.push(format!("qtype={}", self.qtype));

        if let Some(ref rcode) = self.rcode {
            parts.push(format!("rcode={}", rcode));
        }

        if let Some(count) = self.answer_count {
            parts.push(format!("answers={}", count));
        }

        // Display response time or dash if not available
        if let Some(ms) = self.response_time_ms {
            parts.push(format!("time={}ms", ms));
        } else {
            parts.push("time=-".to_string());
        }

        if let Some(cached) = self.cached {
            parts.push(format!("cached={}", cached));
        }

        parts.join(" ")
    }
}

/// Audit event (generic wrapper for all audit log types)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuditEvent {
    /// DNS query/response log
    Query(QueryLogEntry),

    /// Security event
    Security {
        /// Timestamp (ISO 8601 format)
        timestamp: String,
        /// Event type
        event_type: SecurityEventType,
        /// Event message
        message: String,
        /// Client IP (if available)
        #[serde(skip_serializing_if = "Option::is_none")]
        client_ip: Option<IpAddr>,
        /// Query name (if available)
        #[serde(skip_serializing_if = "Option::is_none")]
        qname: Option<String>,
        /// Additional details
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<serde_json::Value>,
    },
}

impl AuditEvent {
    /// Create a security event
    pub fn security(event_type: SecurityEventType, message: impl Into<String>) -> Self {
        Self::Security {
            timestamp: now_timestamp(),
            event_type,
            message: message.into(),
            client_ip: None,
            qname: None,
            details: None,
        }
    }

    /// Create a security event with client info
    pub fn security_with_client(
        event_type: SecurityEventType,
        message: impl Into<String>,
        client_ip: Option<IpAddr>,
        qname: Option<String>,
    ) -> Self {
        Self::Security {
            timestamp: now_timestamp(),
            event_type,
            message: message.into(),
            client_ip,
            qname,
            details: None,
        }
    }

    /// Format as JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        match self {
            AuditEvent::Security {
                timestamp,
                event_type,
                message,
                client_ip,
                qname,
                details: _,
            } => {
                // Manually construct JSON to control field order: timestamp first, type at end
                let mut result = String::from("{");
                result.push_str(&format!(
                    "\"timestamp\":{},",
                    serde_json::to_string(timestamp)?
                ));
                result.push_str(&format!(
                    "\"event_type\":{},",
                    serde_json::to_string(event_type.as_str())?
                ));
                result.push_str(&format!("\"message\":{},", serde_json::to_string(message)?));

                if let Some(ip) = client_ip {
                    result.push_str(&format!(
                        "\"client_ip\":{},",
                        serde_json::to_string(&ip.to_string())?
                    ));
                }

                if let Some(name) = qname {
                    result.push_str(&format!("\"qname\":{},", serde_json::to_string(name)?));
                }

                // Add type field before closing brace
                result.push_str("\"type\":\"security\"}");

                Ok(result)
            }
            _ => serde_json::to_string(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_security_event_type_as_str() {
        assert_eq!(
            SecurityEventType::RateLimitExceeded.as_str(),
            "rate_limit_exceeded"
        );
        assert_eq!(
            SecurityEventType::BlockedDomainQuery.as_str(),
            "blocked_domain_query"
        );
    }

    #[test]
    fn test_security_event_type_from_str() {
        assert_eq!(
            SecurityEventType::parse("rate_limit_exceeded"),
            Some(SecurityEventType::RateLimitExceeded)
        );
        assert_eq!(SecurityEventType::parse("unknown"), None);
    }

    #[test]
    fn test_query_log_entry_to_json() {
        let entry = QueryLogEntry::new(1234, "udp", "example.com".into(), "A".into(), "IN".into())
            .with_client_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)))
            .with_response("NOERROR", 2, 15);

        let json = entry.to_json().unwrap();
        assert!(json.contains("example.com"));
        assert!(json.contains("192.168.1.100"));
        assert!(json.contains("NOERROR"));
    }

    #[test]
    fn test_query_log_entry_to_text() {
        let entry = QueryLogEntry::new(1234, "udp", "example.com".into(), "A".into(), "IN".into())
            .with_response("NOERROR", 2, 15);

        let text = entry.to_text();
        assert!(text.contains("id=1234"));
        assert!(text.contains("qname=example.com"));
        assert!(text.contains("rcode=NOERROR"));
        assert!(text.contains("time=15ms"));
    }

    #[test]
    fn test_audit_event_security() {
        let event = AuditEvent::security(
            SecurityEventType::RateLimitExceeded,
            "Client exceeded 100 queries/minute",
        );

        let json = event.to_json().unwrap();
        assert!(json.contains("rate_limit_exceeded"));
        assert!(json.contains("exceeded 100"));
    }

    #[test]
    fn test_audit_event_security_with_client() {
        let event = AuditEvent::security_with_client(
            SecurityEventType::BlockedDomainQuery,
            "Query blocked by domain filter",
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            Some("malware.example.com".into()),
        );

        let json = event.to_json().unwrap();
        assert!(json.contains("blocked_domain_query"));
        assert!(json.contains("10.0.0.1"));
        assert!(json.contains("malware.example.com"));
    }
}
