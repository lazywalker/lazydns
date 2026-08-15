//! Common utilities shared across DNS server implementations
//!
//! This module provides common helper functions used by multiple server
//! implementations (TCP, DoT, etc.) to avoid code duplication.

use crate::Result;
use crate::dns::Message;

/// Resolve when the shutdown bus is set to true. A bus that was already
/// signalled resolves immediately; a bus whose sender was dropped (server
/// built standalone with a default channel) never resolves.
pub async fn await_shutdown(shutdown: &tokio::sync::watch::Receiver<bool>) {
    let mut shutdown = shutdown.clone();
    if *shutdown.borrow_and_update() {
        return;
    }
    loop {
        match shutdown.changed().await {
            Ok(()) => {
                if *shutdown.borrow() {
                    return;
                }
            }
            // sender dropped without a signal: serve forever
            Err(_) => std::future::pending::<()>().await,
        }
    }
}

/// Wait until every permit taken from `limit` has been returned, bounded by
/// `timeout`. Used to drain in-flight requests before a server task exits.
pub async fn drain_permits(
    limit: &tokio::sync::Semaphore,
    total: usize,
    timeout: std::time::Duration,
) {
    let _ = tokio::time::timeout(timeout, async {
        while limit.available_permits() < total {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await;
}

/// Parse DNS request from wire format
///
/// Thin wrapper around `dns::wire::parse_message` that converts a byte
/// slice into a `Message` structure. Returns an error on invalid input.
///
/// # Arguments
///
/// * `data` - Raw DNS wire format bytes
///
/// # Returns
///
/// Parsed DNS `Message` or error if the data is malformed
pub fn parse_dns_request(data: &[u8]) -> Result<Message> {
    crate::dns::wire::parse_message(data)
}

/// Serialize DNS response to wire format
///
/// Converts a `Message` into DNS wire-format byte vector suitable for
/// sending over TCP/TLS connections.
///
/// # Arguments
///
/// * `message` - DNS message to serialize
///
/// # Returns
///
/// Serialized bytes or error if serialization fails
pub fn serialize_dns_response(message: &Message) -> Result<Vec<u8>> {
    crate::dns::wire::serialize_message(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dns_request_minimal_header() {
        // Minimal valid DNS header (12 bytes of zeros)
        let data = vec![0u8; 12];
        let result = parse_dns_request(&data);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_dns_request_empty_fails() {
        let data: Vec<u8> = vec![];
        let result = parse_dns_request(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_dns_response() {
        let message = Message::new();
        let result = serialize_dns_response(&message);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 12); // Minimal DNS header
    }
}
