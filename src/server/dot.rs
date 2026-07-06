//! DNS over TLS (DoT) server implementation
//!
//! Implements RFC 7858 - DNS over TLS.
//!
//! This module provides a straightforward in-process DoT server that:
//! - Performs TLS handshakes using `tokio-rustls` (`TlsAcceptor`).
//! - Reads DNS-over-TCP framed messages (2-byte length prefix) from the
//!   negotiated TLS stream, parses them using `hickory-proto`, and dispatches
//!   them to a `RequestHandler` implementation.
//! - Serializes handler responses back to wire format and writes them to the
//!   TLS stream using the TCP framing (2-byte length prefix).
//!
//! The implementation favors clarity and testability for use in the test-suite
//! and simple deployments. For high-throughput production usage, consider
//! additional connection and buffer management, timeouts, and connection
//! limits.

use crate::error::{Error, Result};
use crate::server::common::{parse_dns_request, serialize_dns_response};
use crate::server::{RequestHandler, Server, ServerConfig, TlsConfig};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, trace, warn};

/// DNS over TLS server
///
/// Listens for encrypted DNS queries over TLS (port 853 by default).
pub struct DotServer {
    /// Server listening address
    addr: String,
    /// TLS configuration
    tls_config: TlsConfig,
    /// Request handler
    handler: Arc<dyn RequestHandler>,
    /// Maximum concurrent connections
    max_connections: usize,
}

impl std::fmt::Debug for DotServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DotServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl DotServer {
    /// Create a new DoT server
    ///
    /// # Arguments
    ///
    /// * `addr` - Address to listen on (e.g., "0.0.0.0:853")
    /// * `tls_config` - TLS configuration with certificates
    /// * `handler` - Request handler for processing DNS queries
    ///
    /// # Example
    ///
    /// ```no_run
    /// use lazydns::server::{DotServer, TlsConfig, DefaultHandler};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let tls = TlsConfig::from_files("cert.pem", "key.pem")?;
    /// let handler = Arc::new(DefaultHandler);
    /// let server = DotServer::new("0.0.0.0:853", tls, handler);
    /// // server.run().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        addr: impl Into<String>,
        tls_config: TlsConfig,
        handler: Arc<dyn RequestHandler>,
    ) -> Self {
        Self {
            addr: addr.into(),
            tls_config,
            handler,
            max_connections: 1000, // Default value
        }
    }

    /// Create a new DoT server with custom max connections
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Start the DoT server
    ///
    /// Listens for TLS connections and processes DNS queries.
    pub async fn run(self) -> Result<()> {
        let listener = TcpListener::bind(&self.addr).await.map_err(Error::Io)?;

        // Create semaphore for concurrency control
        let concurrent_limit = Arc::new(Semaphore::new(self.max_connections));

        info!(
            "DoT server listening on {} (max_concurrent: {})",
            self.addr, self.max_connections
        );

        let tls_config = self.tls_config.build_server_config()?;
        let acceptor = TlsAcceptor::from(tls_config);

        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                    continue;
                }
            };

            // Try to acquire a permit without waiting
            let permit = match concurrent_limit.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    warn!(
                        "DoT concurrent connection limit reached, rejecting connection from {}",
                        peer_addr
                    );
                    continue;
                }
            };

            debug!("DoT connection from {}", peer_addr);

            let acceptor = acceptor.clone();
            let handler = Arc::clone(&self.handler);

            tokio::spawn(async move {
                let _permit = permit; // Hold permit until connection is done
                if let Err(e) = Self::handle_connection(stream, acceptor, handler).await {
                    warn!("Error handling DoT connection from {}: {}", peer_addr, e);
                }
            });
        }
    }

    /// Handle a single TLS connection
    async fn handle_connection(
        stream: TcpStream,
        acceptor: TlsAcceptor,
        handler: Arc<dyn RequestHandler>,
    ) -> Result<()> {
        // Capture peer address if available for logging
        let peer_addr = stream.peer_addr().ok();

        // Perform TLS handshake
        let mut tls_stream = acceptor
            .accept(stream)
            .await
            .map_err(|e| Error::Other(format!("TLS handshake failed: {}", e)))?;

        debug!(peer = ?peer_addr, "TLS handshake succeeded for DoT connection");

        // Process DNS queries over this TLS connection
        loop {
            // Read message length (2 bytes, big-endian) - same as TCP DNS
            let mut len_buf = [0u8; 2];
            match tls_stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // Client closed connection
                    debug!("DoT client closed connection");
                    break;
                }
                Err(e) => {
                    return Err(Error::Io(e));
                }
            }

            let msg_len = u16::from_be_bytes(len_buf) as usize;

            // Validate message length
            if msg_len == 0 || msg_len > 65535 {
                warn!("Invalid DoT message length: {}", msg_len);
                break;
            }

            // Read message data
            let mut buf = vec![0u8; msg_len];
            trace!(peer = ?peer_addr, len = msg_len, "Reading DoT message");
            tls_stream.read_exact(&mut buf).await.map_err(Error::Io)?;

            // Parse request from wire-format bytes
            let request = parse_dns_request(&buf)?;

            // Log parsed query details
            debug!(
                peer = ?peer_addr,
                question = ?request.questions(),
                "Processing DoT query ID {} with {} questions",
                request.id(),
                request.question_count()
            );

            // Create request context with client address
            let ctx = crate::server::RequestContext::with_client(
                request,
                peer_addr,
                crate::server::Protocol::DoT,
            );

            // Handle request
            let response = handler.handle(ctx).await?;

            // Serialize response to wire-format bytes
            let response_data = serialize_dns_response(&response)?;

            // Log response details
            trace!(peer = ?peer_addr, id = response.id(), answers = response.answer_count(), "Sending DoT response");

            // Write response length
            let response_len = u16::try_from(response_data.len()).map_err(|_| {
                Error::Other(format!(
                    "response too large for DoT DNS framing: {} bytes (max 65535)",
                    response_data.len()
                ))
            })?;
            tls_stream
                .write_all(&response_len.to_be_bytes())
                .await
                .map_err(Error::Io)?;

            // Write response data
            tls_stream
                .write_all(&response_data)
                .await
                .map_err(Error::Io)?;

            tls_stream.flush().await.map_err(Error::Io)?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl Server for DotServer {
    async fn from_config(config: ServerConfig) -> Result<Self> {
        let addr = config
            .tcp_addr
            .ok_or_else(|| Error::Config("TCP address not configured for DoT".to_string()))?
            .to_string();

        let tls_config = config
            .tls_config
            .ok_or_else(|| Error::Config("TLS config not configured for DoT".to_string()))?;

        let handler = config
            .handler
            .ok_or_else(|| Error::Config("Handler not configured".to_string()))?;

        Ok(Self::new(addr, tls_config, handler))
    }

    async fn run(self) -> Result<()> {
        DotServer::run(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::common::{parse_dns_request, serialize_dns_response};

    #[test]
    fn test_parse_request() {
        let data = vec![0u8; 12]; // Minimal DNS header
        let result = parse_dns_request(&data);
        assert!(result.is_ok());
    }

    #[test]
    fn test_serialize_response() {
        let message = crate::dns::Message::new();
        let result = serialize_dns_response(&message);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 12);
    }

    #[tokio::test]
    async fn test_parse_request_invalid() {
        // empty data should fail parsing
        let data: Vec<u8> = vec![];
        let result = parse_dns_request(&data);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_invalid_bind_address() {
        use rcgen::generate_simple_self_signed;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // generate self-signed cert and key files
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let key_pem = cert.signing_key.serialize_pem();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_path_buf();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_path_buf();

        let tls = crate::server::TlsConfig::from_files(cert_path, key_path).unwrap();

        struct DummyHandler;
        #[async_trait::async_trait]
        impl crate::server::RequestHandler for DummyHandler {
            async fn handle(
                &self,
                ctx: crate::server::RequestContext,
            ) -> crate::Result<crate::dns::Message> {
                let req = ctx.into_message();
                Ok(req)
            }
        }

        // Invalid bind address should return an Io error when attempting to bind
        let server = DotServer::new("not-a-valid-addr", tls, Arc::new(DummyHandler));
        let res = server.run().await;
        assert!(res.is_err());
        match res.unwrap_err() {
            Error::Io(_) => {}
            other => panic!("expected Io error, got: {:?}", other),
        }
    }

    #[test]
    fn test_dot_server_new() {
        use rcgen::generate_simple_self_signed;
        use std::io::Write;
        use tempfile::NamedTempFile;

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let key_pem = cert.signing_key.serialize_pem();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_path_buf();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_path_buf();

        let tls = crate::server::TlsConfig::from_files(cert_path, key_path).unwrap();

        struct DummyHandler;
        #[async_trait::async_trait]
        impl crate::server::RequestHandler for DummyHandler {
            async fn handle(
                &self,
                ctx: crate::server::RequestContext,
            ) -> crate::Result<crate::dns::Message> {
                let req = ctx.into_message();
                Ok(req)
            }
        }

        let server = DotServer::new("127.0.0.1:8853", tls, Arc::new(DummyHandler));
        assert_eq!(server.addr, "127.0.0.1:8853");
    }

    #[tokio::test]
    async fn test_dot_server_from_config_missing_addr() {
        let config = crate::server::ServerConfig {
            tcp_addr: None, // Explicitly clear the default address
            ..Default::default()
        };
        let result = DotServer::from_config(config).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Config(msg) => assert!(msg.contains("TCP address not configured")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_dot_server_from_config_missing_tls() {
        let config = crate::server::ServerConfig {
            tcp_addr: Some("127.0.0.1:853".parse().unwrap()),
            ..Default::default()
        };
        let result = DotServer::from_config(config).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Config(msg) => assert!(msg.contains("TLS config not configured")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_dot_server_from_config_missing_handler() {
        use rcgen::generate_simple_self_signed;
        use std::io::Write;
        use tempfile::NamedTempFile;

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let key_pem = cert.signing_key.serialize_pem();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_path_buf();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_path_buf();

        let tls = crate::server::TlsConfig::from_files(cert_path, key_path).unwrap();

        let config = crate::server::ServerConfig {
            tcp_addr: Some("127.0.0.1:853".parse().unwrap()),
            tls_config: Some(tls),
            ..Default::default()
        };

        let result = DotServer::from_config(config).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::Config(msg) => assert!(msg.contains("Handler not configured")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_dot_server_from_config_complete() {
        use rcgen::generate_simple_self_signed;
        use std::io::Write;
        use tempfile::NamedTempFile;

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let key_pem = cert.signing_key.serialize_pem();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_path_buf();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_path_buf();

        let tls = crate::server::TlsConfig::from_files(cert_path, key_path).unwrap();

        struct DummyHandler;
        #[async_trait::async_trait]
        impl crate::server::RequestHandler for DummyHandler {
            async fn handle(
                &self,
                ctx: crate::server::RequestContext,
            ) -> crate::Result<crate::dns::Message> {
                let req = ctx.into_message();
                Ok(req)
            }
        }

        let config = crate::server::ServerConfig {
            tcp_addr: Some("127.0.0.1:853".parse().unwrap()),
            tls_config: Some(tls),
            handler: Some(Arc::new(DummyHandler)),
            ..Default::default()
        };

        let result = DotServer::from_config(config).await;
        assert!(result.is_ok());
        let server = result.unwrap();
        assert_eq!(server.addr, "127.0.0.1:853");
    }

    #[test]
    fn test_parse_request_with_query() {
        // Minimal valid DNS query: 12-byte header + question
        let mut data = vec![
            0x00, 0x01, // ID
            0x01, 0x00, // Flags: standard query, recursion desired
            0x00, 0x01, // QDCOUNT: 1
            0x00, 0x00, // ANCOUNT: 0
            0x00, 0x00, // NSCOUNT: 0
            0x00, 0x00, // ARCOUNT: 0
        ];
        // Question: example.com A IN
        data.extend_from_slice(&[
            0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', // "example"
            0x03, b'c', b'o', b'm', // "com"
            0x00, // Root label
            0x00, 0x01, // QTYPE: A
            0x00, 0x01, // QCLASS: IN
        ]);

        let result = parse_dns_request(&data);
        assert!(result.is_ok());
        let message = result.unwrap();
        assert_eq!(message.id(), 1);
        assert_eq!(message.question_count(), 1);
        assert!(message.recursion_desired());
    }

    #[test]
    fn test_serialize_response_with_answer() {
        use std::net::Ipv4Addr;

        let mut message = crate::dns::Message::new();
        message.set_id(1234);
        message.set_response(true);
        message.add_question(crate::dns::Question::new(
            "example.com",
            crate::dns::RecordType::A,
            crate::dns::RecordClass::IN,
        ));
        message.add_answer(crate::dns::ResourceRecord::new(
            "example.com",
            crate::dns::RecordType::A,
            crate::dns::RecordClass::IN,
            300,
            crate::dns::RData::A(Ipv4Addr::new(93, 184, 216, 34)),
        ));

        let result = serialize_dns_response(&message);
        assert!(result.is_ok());
        let data = result.unwrap();
        // Verify we get more than just header (has question + answer)
        assert!(data.len() > 12);

        // Re-parse to verify roundtrip
        let parsed = parse_dns_request(&data).unwrap();
        assert_eq!(parsed.id(), 1234);
        assert!(parsed.is_response());
        assert_eq!(parsed.answer_count(), 1);
    }

    #[test]
    fn test_parse_request_truncated_header() {
        // Less than 12 bytes
        let data = vec![0x00, 0x01, 0x02];
        let result = parse_dns_request(&data);
        assert!(result.is_err());
    }

    // Integration tests moved to tests/integration_tls_doh_dot.rs
}
