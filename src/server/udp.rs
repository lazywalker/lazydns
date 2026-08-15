//! UDP DNS server implementation.
//!
//! Binds a single UDP socket and dispatches each query to a handler task.
//! Buffer sizes and max concurrency are configurable via [`ServerConfig`].

use crate::dns::Message;
use crate::dns::types::{RecordClass, RecordType};
use crate::server::{RequestHandler, Server, ServerConfig};
use crate::{Error, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, trace, warn};

/// Floor for the read buffer: queries can legally exceed the 512-byte
/// classic limit once EDNS is in play.
const RECV_BUF_MIN: usize = 4096;

/// UDP DNS server.
///
/// Each instance binds one UDP socket and dispatches incoming queries to
/// handler tasks, bounded by a concurrency semaphore. The socket is shared
/// via `Arc` so handler tasks can write responses, but the server itself is
/// meant to be driven by a single owner.
pub struct UdpServer {
    socket: Arc<UdpSocket>,
    handler: Arc<dyn RequestHandler>,
    config: ServerConfig,
    concurrent_limit: Arc<Semaphore>,
}

impl UdpServer {
    /// Create a new UDP server
    ///
    /// Initializes a UDP socket bound to the address specified in the configuration
    /// and prepares the server for handling DNS queries. The server will bind to
    /// the UDP address specified in `config.udp_addr`.
    ///
    /// # Arguments
    ///
    /// * `config` - Server configuration containing the UDP bind address and other settings.
    ///   The `udp_addr` field must be set, otherwise an error will be returned.
    /// * `handler` - Request handler that will process incoming DNS queries. The handler
    ///   is wrapped in an `Arc` to allow sharing across concurrent request processing tasks.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if no UDP address is configured in the server config.
    ///
    /// Returns [`Error::Io`] if the socket cannot be bound to the specified address.
    /// This can happen if:
    /// - The port is already in use by another process
    /// - The address is invalid or unreachable
    /// - Insufficient permissions to bind to the port (such as ports < 1024 on Unix)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lazydns::server::{UdpServer, ServerConfig, DefaultHandler};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Configure server for standard DNS port
    /// let config = ServerConfig::default()
    ///     .with_udp_addr("127.0.0.1:53".parse()?);
    ///
    /// let handler = Arc::new(DefaultHandler::default());
    /// let server = UdpServer::new(config, handler).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// This method does not panic under normal circumstances.
    pub async fn new(config: ServerConfig, handler: Arc<dyn RequestHandler>) -> Result<Self> {
        let addr = config
            .udp_addr
            .ok_or_else(|| Error::Config("UDP address not configured".to_string()))?;

        let socket = UdpSocket::bind(addr).await.map_err(Error::Io)?;

        // Create semaphore to limit concurrent request handling
        // Use max_connections from config (default: 1000)
        let max_concurrent = config.max_connections;
        let concurrent_limit = Arc::new(Semaphore::new(max_concurrent));

        info!(
            "UDP server listening on {} (max_concurrent: {})",
            addr, max_concurrent
        );

        Ok(Self {
            socket: Arc::new(socket),
            handler,
            config,
            concurrent_limit,
        })
    }

    /// Get the local address the server is bound to
    ///
    /// Returns the socket address that the UDP server is currently bound to.
    /// This is useful for logging, testing, or when the server was configured
    /// with port 0 (which gets assigned a random available port by the OS).
    ///
    /// # Returns
    ///
    /// The local [`std::net::SocketAddr`] that the server is listening on.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the local address cannot be retrieved from the socket.
    /// This is extremely rare and usually indicates a serious system issue.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lazydns::server::{UdpServer, ServerConfig, DefaultHandler};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = ServerConfig::default()
    ///     .with_udp_addr("127.0.0.1:0".parse()?); // Port 0 = auto-assign
    ///
    /// let handler = Arc::new(DefaultHandler::default());
    /// let server = UdpServer::new(config, handler).await?;
    ///
    /// // Get the actual port assigned by the OS
    /// let addr = server.local_addr()?;
    /// println!("Server listening on {}", addr);
    /// # Ok(())
    /// # }
    /// ```
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.socket.local_addr().map_err(Error::Io)
    }

    /// Run the UDP server
    ///
    /// Starts the main server loop that listens for incoming DNS queries and processes
    /// them asynchronously. This method will run indefinitely until an error occurs
    /// or the async task is cancelled.
    ///
    /// ## Processing Flow
    ///
    /// For each incoming UDP packet:
    /// 1. Receive the raw bytes and client address
    /// 2. Spawn an asynchronous task to handle the request
    /// 3. Parse the DNS message from wire format
    /// 4. Call the request handler to process the query
    /// 5. Serialize the response back to wire format
    /// 6. Send the response back to the client
    ///
    /// ## Concurrency
    ///
    /// Each DNS query is processed in a separate tokio task, allowing the server
    /// to handle multiple concurrent requests efficiently. The main loop continues
    /// to accept new requests while existing ones are being processed.
    ///
    /// ## Error Handling
    ///
    /// - Network errors during packet reception are logged but don't stop the server
    /// - Request processing errors are logged per-request and don't affect other requests
    /// - Fatal errors (like socket closure) cause the method to return
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if there's a fatal network error that prevents the server
    /// from continuing to operate, such as the UDP socket being closed unexpectedly.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lazydns::server::{UdpServer, ServerConfig, DefaultHandler};
    /// use std::sync::Arc;
    /// use tokio::signal;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = ServerConfig::default()
    ///     .with_udp_addr("127.0.0.1:5353".parse()?);
    ///
    /// let handler = Arc::new(DefaultHandler::default());
    /// let server = UdpServer::new(config, handler).await?;
    ///
    /// println!("Starting UDP DNS server...");
    ///
    /// // Run until interrupted
    /// tokio::select! {
    ///     result = server.run() => {
    ///         if let Err(e) = result {
    ///             eprintln!("Server error: {}", e);
    ///         }
    ///     }
    ///     _ = signal::ctrl_c() => {
    ///         println!("Shutting down...");
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// This method does not panic under normal circumstances.
    pub async fn run(&self) -> Result<()> {
        let mut buf = vec![0u8; self.config.max_udp_size.max(RECV_BUF_MIN)];
        let shutdown = self.config.shutdown_rx.clone();

        info!("UDP server started");

        loop {
            tokio::select! {
                received = self.socket.recv_from(&mut buf) => {
                    let (len, peer_addr) = match received {
                        Ok(r) => r,
                        Err(e) => {
                            error!("Error receiving UDP packet: {}", e);
                            // Continue serving despite errors
                            continue;
                        }
                    };
                    trace!("Received {} bytes from {}", len, peer_addr);

                    // Try to acquire a permit without waiting
                    // If we're at capacity, drop the request to prevent memory exhaustion
                    let permit = match self.concurrent_limit.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            warn!(
                                "Concurrent request limit reached, dropping packet from {}",
                                peer_addr
                            );
                            continue;
                        }
                    };

                    // Copy the data so we can move it to the spawned task
                    let request_data = buf[..len].to_vec();
                    let handler = Arc::clone(&self.handler);
                    let socket = self.socket.clone();

                    // Spawn a task to handle this request
                    // The permit is held until the task completes
                    tokio::spawn(async move {
                        let _permit = permit; // Hold permit until request is done
                        if let Err(e) =
                            Self::handle_request(&request_data, peer_addr, handler, socket).await
                        {
                            warn!("Error handling request from {}: {}", peer_addr, e);
                        }
                    });
                }
                _ = crate::server::common::await_shutdown(&shutdown) => {
                    info!("UDP server shutting down, draining in-flight requests");
                    break;
                }
            }
        }

        crate::server::common::drain_permits(
            &self.concurrent_limit,
            self.config.max_connections,
            Duration::from_secs(10),
        )
        .await;
        info!("UDP server stopped");
        Ok(())
    }

    /// Handle a single DNS request
    async fn handle_request(
        request_data: &[u8],
        peer_addr: std::net::SocketAddr,
        handler: Arc<dyn RequestHandler>,
        socket: Arc<UdpSocket>,
    ) -> Result<()> {
        // Parse DNS wire format
        let request = Self::parse_request(request_data)?;

        debug!(
            peer = %peer_addr,
            question = ?request.questions(),
            "Processing query ID {} with {} questions from {}",
            request.id(),
            request.question_count(),
            peer_addr
        );

        // Create request context
        let req_id = request.id();
        let max_response = client_udp_limit(&request);
        let ctx = crate::server::RequestContext::with_client(
            request,
            Some(peer_addr),
            crate::server::Protocol::Udp,
        );

        // Handle the request
        let mut response = handler.handle(ctx).await?;
        response.set_id(req_id);

        trace!(
            "Sending response ID {} with {} answers to {}",
            response.id(),
            response.answer_count(),
            peer_addr
        );

        // Serialize and send response
        let response_data = Self::serialize_response(&response)?;
        let response_data = fit_udp_response(&response, response_data, max_response);

        socket
            .send_to(&response_data, peer_addr)
            .await
            .map_err(Error::Io)?;

        Ok(())
    }

    /// Parse DNS request from wire format.
    ///
    /// Thin wrapper over [`crate::dns::wire::parse_message`]; returns an error on
    /// malformed/truncated input or unsupported record types.
    fn parse_request(data: &[u8]) -> Result<Message> {
        crate::dns::wire::parse_message(data)
    }

    /// Serialize DNS response to wire format.
    ///
    /// Thin wrapper over [`crate::dns::wire::serialize_message`]; returns an
    /// error on invalid names/labels.
    fn serialize_response(message: &Message) -> Result<Vec<u8>> {
        crate::dns::wire::serialize_message(message)
    }
}

/// Response size limit for `request`: the client's advertised EDNS payload
/// size, or the classic 512-byte limit without EDNS. Sizes below 512 are
/// ignored (RFC 6891).
fn client_udp_limit(request: &Message) -> usize {
    request
        .additional()
        .iter()
        .find(|rr| rr.rtype() == RecordType::OPT)
        .and_then(|rr| match rr.rclass() {
            RecordClass::Unknown(size) if size >= 512 => Some(size as usize),
            _ => None,
        })
        .unwrap_or(512)
}

/// Send `serialized` as-is while it fits; past the limit, fall back to a
/// TC=1 header plus question section so the client retries over TCP.
fn fit_udp_response(response: &Message, serialized: Vec<u8>, limit: usize) -> Vec<u8> {
    if serialized.len() <= limit {
        return serialized;
    }

    let mut truncated = response.clone();
    truncated.set_truncated(true);
    truncated.answers_mut().clear();
    truncated.authority_mut().clear();
    truncated.additional_mut().clear();
    match crate::dns::wire::serialize_message(&truncated) {
        Ok(data) => {
            debug!(
                limit,
                full_len = serialized.len(),
                truncated_len = data.len(),
                "response exceeds UDP limit, sending TC=1"
            );
            data
        }
        // cannot happen for a message that just serialized, but never
        // fail to answer over a truncation fallback
        Err(_) => serialized,
    }
}

#[async_trait::async_trait]
impl Server for UdpServer {
    async fn from_config(config: ServerConfig) -> Result<Self> {
        let handler = config
            .handler
            .clone()
            .ok_or_else(|| Error::Config("Handler not configured".to_string()))?;
        Self::new(config, handler).await
    }

    async fn run(self) -> Result<()> {
        UdpServer::run(&self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::wire;
    use crate::dns::{Question, RecordClass, RecordType};
    use crate::server::DefaultHandler;

    #[tokio::test]
    async fn test_udp_server_creation() {
        let config = ServerConfig::default().with_udp_addr("127.0.0.1:0".parse().unwrap());
        let handler = Arc::new(DefaultHandler);

        let server = UdpServer::new(config, handler).await;
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_udp_server_local_addr() {
        let config = ServerConfig::default().with_udp_addr("127.0.0.1:0".parse().unwrap());
        let handler = Arc::new(DefaultHandler);

        let server = UdpServer::new(config, handler).await.unwrap();
        let addr = server.local_addr();
        assert!(addr.is_ok());
        assert_eq!(addr.unwrap().ip(), std::net::Ipv4Addr::LOCALHOST);
    }

    #[tokio::test]
    async fn test_udp_server_creation_without_udp_addr() {
        let config = ServerConfig::new(None, None); // No UDP addr configured
        let handler = Arc::new(DefaultHandler);

        let server = UdpServer::new(config, handler).await;
        assert!(server.is_err());
        // Check that it's a config error
        if let Err(Error::Config(_)) = server {
            // Expected
        } else {
            panic!("Expected Config error");
        }
    }

    #[tokio::test]
    async fn test_run_stops_on_shutdown() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let config = ServerConfig::default()
            .with_udp_addr("127.0.0.1:0".parse().unwrap())
            .with_shutdown_rx(rx);
        let handler = Arc::new(DefaultHandler);
        let server = Arc::new(UdpServer::new(config, handler).await.unwrap());

        let task = tokio::spawn({
            let server = Arc::clone(&server);
            async move { UdpServer::run(&server).await }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(true).unwrap();

        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("run() must return after shutdown signal")
            .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_parse_request_with_real_dns_message() {
        // Build a real DNS query message and serialize it, then parse via parse_request
        let mut req = Message::new();
        req.set_id(0x42);
        req.set_query(true);
        req.add_question(Question::new(
            "example.test",
            RecordType::A,
            RecordClass::IN,
        ));

        let data = wire::serialize_message(&req).expect("serialize request");
        let parsed = UdpServer::parse_request(&data).expect("parse request");
        assert_eq!(parsed.id(), 0x42);
        assert_eq!(parsed.question_count(), 1);
        assert!(!parsed.is_response()); // Should be a query, not a response
    }

    #[tokio::test]
    async fn test_serialize_response_with_real_dns_message() {
        // Build a DNS response message and serialize via serialize_response
        let mut resp = Message::new();
        resp.set_id(0x99);
        resp.set_response(true);
        resp.add_question(Question::new(
            "example.test",
            RecordType::A,
            RecordClass::IN,
        ));

        let data = UdpServer::serialize_response(&resp).expect("serialize response");
        assert!(data.len() >= 12); // DNS header is at least 12 bytes

        // Verify we can parse it back
        let parsed = wire::parse_message(&data).expect("parse serialized response");
        assert_eq!(parsed.id(), 0x99);
        assert!(parsed.is_response());
        assert_eq!(parsed.question_count(), 1);
    }

    #[tokio::test]
    async fn test_parse_request_placeholder() {
        let data = vec![0u8; 12];
        let message = UdpServer::parse_request(&data);
        assert!(message.is_ok());
    }

    #[tokio::test]
    async fn test_serialize_response_placeholder() {
        let message = Message::new();
        let data = UdpServer::serialize_response(&message);
        assert!(data.is_ok());
        assert_eq!(data.unwrap().len(), 12); // DNS header size
    }

    #[tokio::test]
    async fn test_parse_request_with_invalid_data() {
        let data = vec![0u8; 5]; // Too short for DNS message
        let message = UdpServer::parse_request(&data);
        assert!(message.is_err());
    }

    #[tokio::test]
    async fn test_serialize_response_with_complex_message() {
        let mut resp = Message::new();
        resp.set_id(0x1234);
        resp.set_response(true);
        resp.set_recursion_available(true);
        resp.add_question(Question::new(
            "complex.example.test",
            RecordType::AAAA,
            RecordClass::IN,
        ));

        let data = UdpServer::serialize_response(&resp).expect("serialize complex response");
        assert!(data.len() > 12); // Should be larger than header

        // Parse back and verify
        let parsed = wire::parse_message(&data).expect("parse complex response");
        assert_eq!(parsed.id(), 0x1234);
        assert!(parsed.is_response());
        assert!(parsed.recursion_available());
    }

    #[test]
    fn test_client_udp_limit() {
        let mut req = Message::new();
        req.set_query(true);
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        // no EDNS: classic 512-byte limit
        assert_eq!(client_udp_limit(&req), 512);

        req.add_additional(crate::dns::ResourceRecord::new(
            ".",
            RecordType::OPT,
            RecordClass::from_u16(1232),
            0,
            crate::dns::RData::OPT {
                extended_rcode: 0,
                version: 0,
                flags: 0,
                options: Vec::new(),
            },
        ));
        assert_eq!(client_udp_limit(&req), 1232);

        // sizes below 512 are invalid per RFC 6891 and must not shrink the limit
        let mut tiny = Message::new();
        tiny.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        tiny.add_additional(crate::dns::ResourceRecord::new(
            ".",
            RecordType::OPT,
            RecordClass::from_u16(100),
            0,
            crate::dns::RData::OPT {
                extended_rcode: 0,
                version: 0,
                flags: 0,
                options: Vec::new(),
            },
        ));
        assert_eq!(client_udp_limit(&tiny), 512);
    }

    #[test]
    fn test_fit_udp_response_truncates() {
        let mut resp = Message::new();
        resp.set_id(0x77);
        resp.set_response(true);
        resp.add_question(Question::new(
            "big.example.com",
            RecordType::A,
            RecordClass::IN,
        ));
        for i in 0..40 {
            resp.add_answer(crate::dns::ResourceRecord::new(
                "big.example.com",
                RecordType::A,
                RecordClass::IN,
                300,
                crate::dns::RData::A(std::net::Ipv4Addr::new(192, 0, 2, i as u8)),
            ));
        }

        let full = UdpServer::serialize_response(&resp).unwrap();
        assert!(full.len() > 512);

        let out = fit_udp_response(&resp, full.clone(), 512);
        assert!(out.len() <= 512, "truncated response must fit the limit");
        let parsed = wire::parse_message(&out).unwrap();
        assert!(parsed.is_truncated(), "TC bit must be set");
        assert_eq!(parsed.answer_count(), 0);

        // a fitting message goes out untouched
        let mut small = Message::new();
        small.set_id(0x78);
        small.set_response(true);
        let small_data = UdpServer::serialize_response(&small).unwrap();
        let same = fit_udp_response(&small, small_data.clone(), 512);
        assert_eq!(same, small_data);
    }
}
