//! Audit query logging.
//!
//! This module provides the query-logging hook used by `PluginHandler` to
//! automatically publish every DNS query to the event bus (for WebUI
//! real-time streaming and the alert engine). It is compiled with the `web`
//! feature and requires no user configuration — logging is always active when
//! `web` is enabled.

use super::event::QueryLogEntry;
use super::logger::AUDIT_LOGGER;
use crate::plugin::Context;
use std::net::IpAddr;
use std::time::Instant;

/// Check if a domain is a DNS-SD discovery query (filtered from query logs to
/// reduce noise, since these are internal mDNS discovery probes).
fn is_dns_sd_query(qname: &str) -> bool {
    // Examples:
    //   b._dns-sd._udp.0.8.100.10.in-addr.arpa
    //   db._dns-sd._udp.0.8.100.10.in-addr.arpa
    //   _services._dns-sd._udp.local
    qname.contains("._dns-sd._udp")
}

/// Log a DNS query for the given request context.
///
/// Called automatically by `PluginHandler::handle()` after the plugin sequence
/// completes (when the `web` feature is enabled). Builds a [`QueryLogEntry`]
/// from the context's request/response/metadata and publishes it to the event
/// bus via [`AUDIT_LOGGER`]. DNS-SD discovery queries are skipped to reduce
/// noise.
pub fn log_query_for_context(ctx: &Context) {
    let start_time = ctx.get_metadata::<Instant>("request_start_time").copied();

    let request = ctx.request();
    let question = match request.questions().first() {
        Some(q) => q,
        None => return, // No question to log
    };

    let qname = question.qname().to_string();

    // Skip DNS-SD discovery queries (internal mDNS probes)
    if is_dns_sd_query(&qname) {
        return;
    }

    let protocol = ctx
        .get_metadata::<String>("protocol")
        .map(|s| s.as_str())
        .unwrap_or("unknown");

    let mut entry = QueryLogEntry::new(
        request.id(),
        protocol,
        qname,
        format!("{:?}", question.qtype()),
        format!("{:?}", question.qclass()),
    );

    // Attach client IP if available
    if let Some(ip) = ctx.get_metadata::<IpAddr>("client_ip") {
        entry = entry.with_client_ip(*ip);
    }

    // Attach response details if available
    if let Some(response) = ctx.response() {
        let response_time_us = start_time
            .map(|t| t.elapsed().as_micros() as u64)
            .unwrap_or(0);
        let response_time_ms = response_time_us / 1000;

        entry = entry.with_response(
            &format!("{:?}", response.response_code()),
            response.answers().len(),
            response_time_ms,
        );
        entry = entry.with_response_time_us(response_time_us);

        // Check if cached (cache plugin sets "response_from_cache" metadata)
        let cached = ctx
            .get_metadata::<bool>("response_from_cache")
            .copied()
            .unwrap_or(false);
        entry = entry.with_cached(cached);

        // Add answer IPs for A/AAAA queries
        let answers: Vec<String> = response
            .answers()
            .iter()
            .filter_map(|a| match a.rdata() {
                crate::dns::RData::A(ip) => Some(ip.to_string()),
                crate::dns::RData::AAAA(ip) => Some(ip.to_string()),
                _ => None,
            })
            .collect();

        if !answers.is_empty() {
            entry = entry.with_answers(answers);
        }
    }

    // Publish to event bus (WebUI SSE + alert engine consume from here)
    AUDIT_LOGGER.log_query(entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, Question, RecordClass, RecordType};

    #[test]
    fn test_dns_sd_query_detection() {
        assert!(is_dns_sd_query("b._dns-sd._udp.0.8.100.10.in-addr.arpa"));
        assert!(is_dns_sd_query("db._dns-sd._udp.0.8.100.10.in-addr.arpa"));
        assert!(is_dns_sd_query("_services._dns-sd._udp.local"));
        assert!(is_dns_sd_query("r._dns-sd._udp.example.com"));

        assert!(!is_dns_sd_query("example.com"));
        assert!(!is_dns_sd_query("www.example.com"));
        assert!(!is_dns_sd_query("_tcp.example.com"));
    }

    #[tokio::test]
    async fn test_log_query_for_context_does_not_panic() {
        let mut request = Message::new();
        request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let mut ctx = Context::new(request);
        ctx.set_metadata("protocol".to_string(), "udp".to_string());

        // Should not panic even without event bus initialized
        log_query_for_context(&ctx);
    }

    #[tokio::test]
    async fn test_log_query_for_context_with_response() {
        let mut request = Message::new();
        request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let mut ctx = Context::new(request);
        ctx.set_metadata("protocol".to_string(), "tcp".to_string());

        let mut response = Message::new();
        response.set_response(true);
        ctx.set_response(Some(response));

        // Should not panic with response present
        log_query_for_context(&ctx);
    }
}
