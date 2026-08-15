/// TCP DNS server
///
/// Handles DNS queries over TCP. TCP is used when responses are larger than
/// the UDP path can support (>512 bytes) or when a stream-oriented,
/// connection-based transport is desired.
///
/// The server accepts TCP connections, reads a single length-prefixed DNS
/// message from the connection, dispatches it to the configured
/// `RequestHandler`, and writes a length-prefixed DNS response back to the
/// client. Each connection is handled on a spawned task so multiple clients
/// can be served concurrently.
///
/// Notes:
/// - DNS over TCP uses a 2-byte big-endian length prefix for each message.
/// - `handle_connection` encapsulates the read/parse/handle/serialize/write
///   flow and is suitable for unit testing without running the full accept
///   loop.
///
/// # Example
///
/// ```rust,no_run
/// use lazydns::server::{TcpServer, ServerConfig, DefaultHandler};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = ServerConfig::default();
/// let handler = Arc::new(DefaultHandler::default());
/// let server = TcpServer::new(config, handler).await?;
/// // `server.run().await` runs indefinitely; execute it in a background task.
/// # Ok(())
/// # }
/// ```
use crate::server::common::{parse_dns_request, serialize_dns_response};
use crate::server::{RequestHandler, Server, ServerConfig};
use crate::{Error, Result};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, trace, warn};

pub struct TcpServer {
    listener: TcpListener,
    handler: Arc<dyn RequestHandler>,
    config: ServerConfig,
    /// Semaphore to limit concurrent connection handling
    concurrent_limit: Arc<Semaphore>,
}

impl TcpServer {
    /// Create a new TCP server
    ///
    /// # Arguments
    ///
    /// * `config` - Server configuration
    /// * `handler` - Request handler for processing queries
    ///
    /// # Errors
    ///
    /// Returns an error if the listener cannot be bound to the configured address.
    pub async fn new(config: ServerConfig, handler: Arc<dyn RequestHandler>) -> Result<Self> {
        let addr = config
            .tcp_addr
            .ok_or_else(|| Error::Config("TCP address not configured".to_string()))?;

        let listener = TcpListener::bind(addr).await.map_err(Error::Io)?;

        // Create semaphore to limit concurrent connection handling
        // Use max_connections from config (default: 1000)
        let max_concurrent = config.max_connections;
        let concurrent_limit = Arc::new(Semaphore::new(max_concurrent));

        info!(
            "TCP server listening on {} (max_concurrent: {})",
            addr, max_concurrent
        );

        Ok(Self {
            listener,
            handler,
            config,
            concurrent_limit,
        })
    }

    /// Run the TCP server
    ///
    /// This method runs the server loop, accepting connections and handling queries.
    /// It will run indefinitely until an error occurs or the task is cancelled.
    ///
    /// # Errors
    ///
    /// Returns an error if there is a network or processing error.
    pub async fn run(&self) -> Result<()> {
        let shutdown = self.config.shutdown_rx.clone();

        info!("TCP server started");

        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, peer_addr) = match accepted {
                        Ok(conn) => conn,
                        Err(e) => {
                            error!("Error accepting TCP connection: {}", e);
                            // Continue serving despite errors
                            continue;
                        }
                    };
                    debug!("Accepted connection from {}", peer_addr);

                    // Try to acquire a permit without waiting
                    // If we're at capacity, close the connection immediately
                    let permit = match self.concurrent_limit.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            warn!(
                                "Concurrent connection limit reached, rejecting connection from {}",
                                peer_addr
                            );
                            // Drop the stream to close the connection
                            drop(stream);
                            continue;
                        }
                    };

                    let handler = Arc::clone(&self.handler);
                    let max_size = self.config.max_tcp_size;
                    let read_timeout = self.config.timeout;

                    // Spawn a task to handle this connection
                    // The permit is held until the task completes
                    tokio::spawn(async move {
                        let _permit = permit; // Hold permit until connection is done
                        if let Err(e) = Self::handle_connection(
                            stream,
                            peer_addr,
                            handler,
                            max_size,
                            read_timeout,
                        )
                        .await
                        {
                            error!("Error handling connection from {}: {}", peer_addr, e);
                        }
                    });
                }
                _ = crate::server::common::await_shutdown(&shutdown) => {
                    info!("TCP server shutting down, draining in-flight connections");
                    break;
                }
            }
        }

        crate::server::common::drain_permits(
            &self.concurrent_limit,
            self.config.max_connections,
            std::time::Duration::from_secs(10),
        )
        .await;
        info!("TCP server stopped");
        Ok(())
    }

    /// Handle a single TCP connection
    ///
    /// Reads a single length-prefixed DNS request from `stream`, calls the
    /// provided `handler` to obtain a response, and writes a length-prefixed
    /// response back to the client. If the incoming message length exceeds
    /// `max_size`, the function returns an error.
    ///
    /// The function consumes the `TcpStream` so the connection is closed when
    /// the call returns. It is intended to be spawned as a task from the
    /// accept loop.
    async fn handle_connection(
        mut stream: TcpStream,
        peer_addr: std::net::SocketAddr,
        handler: Arc<dyn RequestHandler>,
        max_size: usize,
        read_timeout: std::time::Duration,
    ) -> Result<()> {
        // Read message length (2 bytes, big-endian)
        let mut len_buf = [0u8; 2];
        tokio::time::timeout(read_timeout, stream.read_exact(&mut len_buf))
            .await
            .map_err(|_| Error::Other("timed out reading message length".to_string()))?
            .map_err(Error::Io)?;

        let msg_len = u16::from_be_bytes(len_buf) as usize;

        if msg_len > max_size {
            return Err(Error::DnsProtocol(format!(
                "Message too large: {} > {}",
                msg_len, max_size
            )));
        }

        trace!("Reading {} bytes", msg_len);

        // Read message data
        let mut buf = vec![0u8; msg_len];
        tokio::time::timeout(read_timeout, stream.read_exact(&mut buf))
            .await
            .map_err(|_| Error::Other("timed out reading message body".to_string()))?
            .map_err(Error::Io)?;

        // Parse request
        let request = parse_dns_request(&buf)?;

        debug!(
            peer = %peer_addr,
            question = ?request.questions(),
            "Processing query ID {} with {} questions",
            request.id(),
            request.question_count()
        );

        // Create request context
        let ctx = crate::server::RequestContext::with_client(
            request,
            Some(peer_addr),
            crate::server::Protocol::Tcp,
        );

        // Handle the request
        let response = handler.handle(ctx).await?;

        trace!(
            "Sending response ID {} with {} answers",
            response.id(),
            response.answer_count()
        );

        // Serialize response
        let response_data = serialize_dns_response(&response)?;

        // Write length prefix
        let len = u16::try_from(response_data.len()).map_err(|_| {
            Error::Other(format!(
                "response too large for TCP DNS framing: {} bytes (max 65535)",
                response_data.len()
            ))
        })?;
        stream
            .write_all(&len.to_be_bytes())
            .await
            .map_err(Error::Io)?;

        // Write response data
        stream.write_all(&response_data).await.map_err(Error::Io)?;

        stream.flush().await.map_err(Error::Io)?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl Server for TcpServer {
    async fn from_config(config: ServerConfig) -> Result<Self> {
        let handler = config
            .handler
            .clone()
            .ok_or_else(|| Error::Config("Handler not configured".to_string()))?;
        Self::new(config, handler).await
    }

    async fn run(self) -> Result<()> {
        TcpServer::run(&self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::wire;
    use crate::dns::{Message, Question, RecordClass, RecordType};
    use crate::server::DefaultHandler;
    use tokio::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn test_tcp_server_creation() {
        let config = ServerConfig::default().with_tcp_addr("127.0.0.1:0".parse().unwrap());
        let handler = Arc::new(DefaultHandler);

        let server = TcpServer::new(config, handler).await;
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_parse_request_and_serialize_response() {
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
        let parsed = parse_dns_request(&data).expect("parse request");
        assert_eq!(parsed.id(), 0x42);
        assert_eq!(parsed.question_count(), 1);

        // Turn parsed message into a response and serialize via serialize_response
        let mut resp = parsed.clone();
        resp.set_response(true);
        let resp_data = serialize_dns_response(&resp).expect("serialize response");
        assert!(resp_data.len() >= 12);
    }

    #[tokio::test]
    async fn test_handle_connection_roundtrip() {
        // Create a listener and accept one connection to exercise handle_connection
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn a server-side task that accepts one connection and runs handle_connection
        let server_task = tokio::spawn(async move {
            if let Ok((stream, _peer)) = listener.accept().await {
                let handler = Arc::new(DefaultHandler);
                // allow reasonably large messages
                let _ = TcpServer::handle_connection(
                    stream,
                    "127.0.0.1:12345".parse().unwrap(),
                    handler,
                    64 * 1024,
                    std::time::Duration::from_secs(5),
                )
                .await;
            }
        });

        // Create a client connection and send a request
        let mut client = TcpStream::connect(addr).await.unwrap();

        let mut req = Message::new();
        req.set_id(0x99);
        req.set_query(true);
        req.add_question(Question::new(
            "roundtrip.test",
            RecordType::AAAA,
            RecordClass::IN,
        ));

        let req_data = wire::serialize_message(&req).expect("serialize client request");
        let len = req_data.len() as u16;
        client
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| eprintln!("write len: {}", e))
            .ok();
        client
            .write_all(&req_data)
            .await
            .map_err(|e| eprintln!("write data: {}", e))
            .ok();

        // Read response length
        let mut len_buf = [0u8; 2];
        client.read_exact(&mut len_buf).await.unwrap();
        let resp_len = u16::from_be_bytes(len_buf) as usize;
        let mut resp_buf = vec![0u8; resp_len];
        client.read_exact(&mut resp_buf).await.unwrap();

        // Parse response and validate it was converted to a response by DefaultHandler
        let response = wire::parse_message(&resp_buf).expect("parse response");
        assert!(response.is_response());
        assert_eq!(response.id(), 0x99);

        let _ = server_task.await;
    }
}
