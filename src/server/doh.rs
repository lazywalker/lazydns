//! DNS over HTTPS (DoH) server implementation
//!
//! Implements RFC 8484 (DNS Queries over HTTPS).
//!
//! This module provides a minimal DoH server implementation suitable for
//! embedding into the test-suite and simple deployments. It supports the
//! two DoH request styles defined in RFC 8484 Section 4.1:
//!
//! - GET: query parameter `dns` containing a base64url (no padding) encoded
//!   DNS wire-format query. Example: `/dns-query?dns=<base64url>`.
//! - POST: binary `application/dns-message` request body containing the DNS
//!   wire-format query.
//!
//! The server returns responses with the `application/dns-message` media type
//! and mirrors common status codes for malformed requests or handler errors
//! (400 Bad Request, 415 Unsupported Media Type, 500 Internal Server Error).
//!
//! Notes:
//! - This implementation focuses on correctness and testability rather than
//!   production-grade performance. For a production server, prefer using
//!   `axum-server`/`hyper` with proper TLS termination and HTTP/2 support.
//! - Functions in this module accept and return crate-level `Result` values
//!   for consistent error handling inside the server.

use crate::dns::Message;
use crate::error::{Error, Result};
use crate::server::{RequestHandler, Server, ServerConfig, TlsConfig};
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, Query as AxumQuery, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
#[cfg(feature = "doh")]
use axum_server::bind_rustls as axum_bind_rustls;
#[cfg(feature = "doh")]
use axum_server::tls_rustls::RustlsConfig as AxumRustlsConfig;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, trace};

/// DNS over HTTPS server
///
/// Implements RFC 8484 DoH protocol over HTTP/2.
pub struct DohServer {
    /// Server listening address
    addr: String,
    /// TLS configuration
    _tls_config: TlsConfig,
    /// Request handler
    handler: Arc<dyn RequestHandler>,
    /// DoH path (default: /dns-query)
    path: String,
    /// Honor `X-Forwarded-For` from a trusted reverse proxy (default: false)
    trust_forwarded_for: bool,
    /// Shared shutdown bus
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
}

/// Per-server state shared with the axum handlers.
#[derive(Clone)]
struct DohAppState {
    handler: Arc<dyn RequestHandler>,
    trust_forwarded_for: bool,
}

impl DohServer {
    /// Create a new DoH server
    ///
    /// # Arguments
    ///
    /// * `addr` - Address to listen on (e.g., "0.0.0.0:443")
    /// * `tls_config` - TLS configuration with certificates
    /// * `handler` - Request handler for processing DNS queries
    ///
    /// # Example
    ///
    /// ```no_run
    /// use lazydns::server::{DohServer, TlsConfig, DefaultHandler};
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let tls = TlsConfig::from_files("cert.pem", "key.pem")?;
    /// let handler = Arc::new(DefaultHandler);
    /// let server = DohServer::new("0.0.0.0:443", tls, handler);
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
            _tls_config: tls_config,
            handler,
            path: "/dns-query".to_string(),
            trust_forwarded_for: false,
            shutdown_rx: tokio::sync::watch::channel(false).1,
        }
    }

    /// Set the DoH query path
    pub fn with_path(mut self, path: String) -> Self {
        self.path = path;
        self
    }

    /// Set whether to honor `X-Forwarded-For` (only safe behind a trusted
    /// reverse proxy)
    pub fn with_trust_forwarded_for(mut self, trust: bool) -> Self {
        self.trust_forwarded_for = trust;
        self
    }

    /// Attach the launcher-wide shutdown bus.
    pub fn with_shutdown_rx(mut self, shutdown_rx: tokio::sync::watch::Receiver<bool>) -> Self {
        self.shutdown_rx = shutdown_rx;
        self
    }

    /// Start the DoH server.
    ///
    /// Listens for HTTPS connections and processes DNS queries over HTTP/2.
    /// Uses axum-server; not tuned for high-throughput production frontends.
    pub async fn run(self) -> Result<()> {
        let state = DohAppState {
            handler: Arc::clone(&self.handler),
            trust_forwarded_for: self.trust_forwarded_for,
        };

        // Create router with ConnectInfo support for client address tracking
        let app = Router::new()
            .route(&self.path, post(handle_post_query).get(handle_get_query))
            .with_state(state)
            .into_make_service_with_connect_info::<SocketAddr>();

        info!(
            "DoH server listening on {} (path: {})",
            self.addr, self.path
        );

        let shutdown = self.shutdown_rx.clone();

        // If compiled with `--features doh`, run axum-server with Rustls.
        // This enables proper TLS termination and HTTP/2 for DoH.
        #[cfg(feature = "doh")]
        {
            // Build TLS config only when TLS feature is enabled to avoid
            // unnecessary work in the default (non-TLS) build.
            let tls_config = self._tls_config.build_server_config()?;

            // Convert our rustls ServerConfig (Arc) into axum-server's RustlsConfig
            let axum_tls = AxumRustlsConfig::from_config(tls_config.clone());

            info!(
                "Starting DoH server with TLS on {} (path: {})",
                self.addr, self.path
            );

            let bind_addr: std::net::SocketAddr = self
                .addr
                .parse()
                .map_err(|e| Error::Config(format!("Invalid bind address: {}", e)))?;

            // axum-server drives graceful shutdown through a Handle
            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                crate::server::common::await_shutdown(&shutdown).await;
                info!("DoH server shutting down");
                shutdown_handle.graceful_shutdown(Some(Duration::from_secs(10)));
            });

            axum_bind_rustls(bind_addr, axum_tls)
                .handle(handle)
                .serve(app)
                .await
                .map_err(|e| Error::Other(format!("Server error: {}", e)))?;
        }

        // Default (no-tls)
        #[cfg(not(feature = "doh"))]
        {
            // Default (no-tls) fallback for test and lightweight deployments: plain TCP
            tracing::warn!(
                "DoH server running without TLS; enable `tls` feature for production TLS support"
            );

            let listener = tokio::net::TcpListener::bind(&self.addr)
                .await
                .map_err(Error::Io)?;

            // Serve without TLS
            let shutdown = shutdown.clone();
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    crate::server::common::await_shutdown(&shutdown).await;
                    info!("DoH server shutting down");
                })
                .await
                .map_err(|e| Error::Other(format!("Server error: {}", e)))?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl Server for DohServer {
    async fn from_config(config: ServerConfig) -> crate::Result<Self> {
        let addr = config
            .tcp_addr
            .ok_or_else(|| Error::Config("TCP address not configured for DoH".to_string()))?
            .to_string();

        let tls_config = config
            .tls_config
            .ok_or_else(|| Error::Config("TLS config not configured for DoH".to_string()))?;

        let handler = config
            .handler
            .ok_or_else(|| Error::Config("Handler not configured".to_string()))?;

        let mut server = Self::new(addr, tls_config, handler);
        if let Some(path) = config.doh_path {
            server = server.with_path(path);
        }
        server = server.with_trust_forwarded_for(config.trust_forwarded_for);
        server = server.with_shutdown_rx(config.shutdown_rx);
        Ok(server)
    }

    async fn run(self) -> crate::Result<()> {
        DohServer::run(self).await
    }
}

/// Extract client address from DoH HTTP request.
///
/// `X-Forwarded-For` is honored only when `trust_forwarded_for` is set: on a
/// direct connection the header is attacker-controlled and would allow
/// ACL/rate-limit bypass via a spoofed client IP.
fn extract_client_addr(
    headers: &HeaderMap,
    connect_addr: Option<SocketAddr>,
    trust_forwarded_for: bool,
) -> Option<SocketAddr> {
    if trust_forwarded_for
        && let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.split(',').next())
            .and_then(|ip_str| ip_str.trim().parse::<std::net::IpAddr>().ok())
    {
        let port = connect_addr.map(|a| a.port()).unwrap_or(443);
        return Some(SocketAddr::new(forwarded, port));
    }

    connect_addr
}

/// Handle DoH GET requests (RFC 8484 Section 4.1)
///
/// Expected behavior:
/// - Expects a `dns` query parameter which is a base64url (no padding)
///   encoded DNS wire-format query.
/// - Returns `200 OK` with `application/dns-message` and the serialized
///   DNS response on success.
/// - Returns `400 Bad Request` for missing/invalid parameters or malformed
///   DNS messages.
/// - Returns `500 Internal Server Error` when the request handler fails.
///
/// This function is intended to be used as an `axum` handler and therefore
/// takes the `State` and `Query` extracts. It returns an `axum::Response`
/// so it can map directly to HTTP status codes and body bytes.
async fn handle_get_query(
    State(state): State<DohAppState>,
    ConnectInfo(connect_addr): ConnectInfo<SocketAddr>,
    AxumQuery(params): AxumQuery<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    // GET requests use ?dns= query parameter with base64url-encoded DNS message (RFC 8484 Section 4.1)
    debug!("Handling DoH GET request");

    // Extract client address
    let client_addr = extract_client_addr(&headers, Some(connect_addr), state.trust_forwarded_for);

    // Extract the 'dns' parameter
    let dns_param = match params.get("dns") {
        Some(param) => param,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "Missing 'dns' query parameter. Usage: /dns-query?dns=<base64url-encoded-query>",
            )
                .into_response();
        }
    };

    trace!(dns_param, "DoH GET query parameters");

    // Decode base64url-encoded DNS message
    let dns_data = match URL_SAFE_NO_PAD.decode(dns_param.as_bytes()) {
        Ok(data) => data,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid base64url encoding: {}", e),
            )
                .into_response();
        }
    };

    trace!("Decoded DNS query: {} bytes", dns_data.len());

    // Parse DNS query
    let request = match parse_dns_message(&dns_data) {
        Ok(msg) => msg,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid DNS message: {}", e),
            )
                .into_response();
        }
    };

    // Log parsed query details similar to UDP/TCP handlers
    debug!(
        question = ?request.questions(),
        "Processing query ID {} with {} questions",
        request.id(),
        request.question_count()
    );

    // Create request context with client address
    let ctx = crate::server::RequestContext::with_client(
        request,
        client_addr,
        crate::server::Protocol::DoH,
    );

    // Process query
    let response = match state.handler.handle(ctx).await {
        Ok(resp) => resp,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Query processing failed: {}", e),
            )
                .into_response();
        }
    };

    // Serialize response
    let response_data = match serialize_dns_message(&response) {
        Ok(data) => data,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response serialization failed: {}", e),
            )
                .into_response();
        }
    };

    debug!("DoH GET handler processed query successfully");
    trace!("Sending DoH response: {} bytes", response_data.len());

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/dns-message")],
        response_data,
    )
        .into_response()
}

/// Handle DoH POST requests (RFC 8484 Section 4.1)
///
/// Expected behavior:
/// - Requires `Content-Type: application/dns-message` header.
/// - The request body must be the DNS wire-format query bytes.
/// - Returns `200 OK` with `application/dns-message` and the serialized
///   DNS response on success.
/// - Returns `400 Bad Request` when `Content-Type` is missing or when the
///   DNS message is malformed.
/// - Returns `415 Unsupported Media Type` when a different content type is
///   provided.
/// - Returns `500 Internal Server Error` when the request handler fails.
///
/// Like `handle_get_query`, this function is an `axum` handler and returns
/// an `axum::Response`.
async fn handle_post_query(
    State(state): State<DohAppState>,
    ConnectInfo(connect_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    debug!("Handling DoH POST request");

    // Extract client address
    let client_addr = extract_client_addr(&headers, Some(connect_addr), state.trust_forwarded_for);
    // Verify content type
    if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        if content_type != "application/dns-message" {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/dns-message",
            )
                .into_response();
        }
    } else {
        return (StatusCode::BAD_REQUEST, "Content-Type header required").into_response();
    }
    trace!(content_length = body.len(), "DoH POST body length");

    // Parse DNS query
    let request = match parse_dns_message(&body) {
        Ok(msg) => {
            trace!(bytes = body.len(), "Parsed DNS POST query");
            msg
        }
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid DNS message: {}", e),
            )
                .into_response();
        }
    };

    // Log parsed query details similar to UDP/TCP handlers
    debug!(
        question = ?request.questions(),
        "Processing query ID {} with {} questions",
        request.id(),
        request.question_count()
    );

    // Create request context with client address
    let ctx = crate::server::RequestContext::with_client(
        request,
        client_addr,
        crate::server::Protocol::DoH,
    );

    // Process query
    let response = match state.handler.handle(ctx).await {
        Ok(resp) => {
            debug!("DoH POST handler processed query successfully");
            resp
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Query processing failed: {}", e),
            )
                .into_response();
        }
    };

    // Serialize response
    let response_data = match serialize_dns_message(&response) {
        Ok(data) => {
            trace!(bytes = data.len(), "Serialized DoH POST response");
            data
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Response serialization failed: {}", e),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/dns-message")],
        response_data,
    )
        .into_response()
}

/// Parse a DNS message from wire-format bytes
///
/// This thin wrapper forwards to the crate's `dns::wire::parse_message`
/// helper and returns the crate `Result` type. The function is intentionally
/// small so tests and handlers can rely on a single parse entry point.
fn parse_dns_message(data: &[u8]) -> Result<Message> {
    crate::dns::wire::parse_message(data)
}

/// Serialize DNS message to wire format
fn serialize_dns_message(message: &Message) -> Result<Vec<u8>> {
    crate::dns::wire::serialize_message(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{RequestContext, RequestHandler};
    use async_trait::async_trait;
    use axum::body::Bytes as AxumBytes;
    use axum::body::to_bytes;
    use axum::http::header::CONTENT_TYPE;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::collections::HashMap;

    struct TestHandler;

    fn test_state(
        handler: Arc<dyn RequestHandler>,
        trust_forwarded_for: bool,
    ) -> State<DohAppState> {
        State(DohAppState {
            handler,
            trust_forwarded_for,
        })
    }

    #[async_trait]
    impl RequestHandler for TestHandler {
        async fn handle(&self, ctx: RequestContext) -> crate::Result<Message> {
            // mark as response and return the same message
            let mut request = ctx.into_message();
            request.set_response(true);
            Ok(request)
        }
    }

    #[tokio::test]
    async fn test_parse_dns_message_placeholder() {
        let data = vec![0u8; 12];
        let result = parse_dns_message(&data);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_serialize_dns_message_placeholder() {
        let message = Message::new();
        let result = serialize_dns_message(&message);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 12);
    }

    #[tokio::test]
    async fn test_base64url_encoding_decoding() {
        // Test data (minimal DNS query header)
        let original_data = vec![
            0x00, 0x01, // ID
            0x01, 0x00, // Flags
            0x00, 0x01, // QDCOUNT
            0x00, 0x00, // ANCOUNT
            0x00, 0x00, // NSCOUNT
            0x00, 0x00, // ARCOUNT
        ];

        // Encode
        let encoded = URL_SAFE_NO_PAD.encode(&original_data);

        // Decode
        let decoded = URL_SAFE_NO_PAD.decode(encoded.as_bytes()).unwrap();

        assert_eq!(original_data, decoded);
    }

    #[tokio::test]
    async fn test_handle_get_query_success() {
        // build a minimal DNS request
        let mut req = Message::new();
        req.set_id(0x1234);
        req.set_query(true);

        let data = crate::dns::wire::serialize_message(&req).unwrap();
        let encoded = URL_SAFE_NO_PAD.encode(&data);

        let mut params = HashMap::new();
        params.insert("dns".to_string(), encoded);

        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp = handle_get_query(
            test_state(Arc::new(TestHandler), false),
            ConnectInfo(connect_addr),
            AxumQuery(params),
            HeaderMap::new(),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::OK);
        let headers = resp.headers();
        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap(),
            "application/dns-message"
        );
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let parsed = crate::dns::wire::parse_message(&body).unwrap();
        assert!(parsed.is_response());
        assert_eq!(parsed.id(), 0x1234);
    }

    #[tokio::test]
    async fn test_handle_post_query_success() {
        let mut req = Message::new();
        req.set_id(0x9a);
        req.set_query(true);
        let data = crate::dns::wire::serialize_message(&req).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "application/dns-message".parse().unwrap());

        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp = handle_post_query(
            test_state(Arc::new(TestHandler), false),
            ConnectInfo(connect_addr),
            headers,
            AxumBytes::from(data.clone()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let headers = resp.headers();
        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap(),
            "application/dns-message"
        );
        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let parsed = crate::dns::wire::parse_message(&body).unwrap();
        assert!(parsed.is_response());
        assert_eq!(parsed.id(), 0x9a);
    }

    #[tokio::test]
    async fn test_handle_post_query_missing_content_type() {
        let mut req = Message::new();
        req.set_id(0x55);
        req.set_query(true);
        let data = crate::dns::wire::serialize_message(&req).unwrap();

        let headers = HeaderMap::new();
        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp = handle_post_query(
            test_state(Arc::new(TestHandler), false),
            ConnectInfo(connect_addr),
            headers,
            AxumBytes::from(data),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_post_query_unsupported_media_type() {
        let mut req = Message::new();
        req.set_id(0x66);
        req.set_query(true);
        let data = crate::dns::wire::serialize_message(&req).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "text/plain".parse().unwrap());
        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp = handle_post_query(
            test_state(Arc::new(TestHandler), false),
            ConnectInfo(connect_addr),
            headers,
            AxumBytes::from(data),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    struct TestHandlerErr;

    #[async_trait]
    impl RequestHandler for TestHandlerErr {
        async fn handle(&self, _ctx: RequestContext) -> crate::Result<Message> {
            Err(crate::Error::Plugin("handler failure".to_string()))
        }
    }

    /// Records the client address each invocation saw.
    struct ClientCaptureHandler(std::sync::Mutex<Option<SocketAddr>>);

    #[async_trait]
    impl RequestHandler for ClientCaptureHandler {
        async fn handle(&self, ctx: RequestContext) -> crate::Result<Message> {
            *self.0.lock().unwrap() = ctx.client_addr().copied();
            let mut request = ctx.into_message();
            request.set_response(true);
            Ok(request)
        }
    }

    fn dns_query_param() -> HashMap<String, String> {
        let mut req = Message::new();
        req.set_id(0x4321);
        req.set_query(true);
        let data = crate::dns::wire::serialize_message(&req).unwrap();
        let mut params = HashMap::new();
        params.insert("dns".to_string(), URL_SAFE_NO_PAD.encode(&data));
        params
    }

    #[tokio::test]
    async fn test_xff_ignored_by_default() {
        let captured = Arc::new(ClientCaptureHandler(std::sync::Mutex::new(None)));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "8.8.8.8".parse().unwrap());

        let connect_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        handle_get_query(
            test_state(Arc::clone(&captured) as Arc<dyn RequestHandler>, false),
            ConnectInfo(connect_addr),
            AxumQuery(dns_query_param()),
            headers,
        )
        .await;

        assert_eq!(*captured.0.lock().unwrap(), Some(connect_addr));
    }

    #[tokio::test]
    async fn test_xff_honored_when_trusted() {
        let captured = Arc::new(ClientCaptureHandler(std::sync::Mutex::new(None)));
        // multiple hops: first entry is the client
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "8.8.8.8, 10.0.0.1".parse().unwrap());

        let connect_addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        handle_get_query(
            test_state(Arc::clone(&captured) as Arc<dyn RequestHandler>, true),
            ConnectInfo(connect_addr),
            AxumQuery(dns_query_param()),
            headers,
        )
        .await;

        let seen = captured.0.lock().unwrap();
        assert_eq!(seen.map(|a| a.ip()).unwrap().to_string(), "8.8.8.8");
    }

    #[tokio::test]
    async fn test_handle_get_query_missing_param() {
        let params: HashMap<String, String> = HashMap::new();
        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp = handle_get_query(
            test_state(Arc::new(TestHandler), false),
            ConnectInfo(connect_addr),
            AxumQuery(params),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_get_query_invalid_base64() {
        let mut params = HashMap::new();
        params.insert("dns".to_string(), "!!not_base64!!".to_string());
        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp = handle_get_query(
            test_state(Arc::new(TestHandler), false),
            ConnectInfo(connect_addr),
            AxumQuery(params),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_get_query_invalid_dns_message() {
        let bad = vec![1u8, 2, 3];
        let encoded = URL_SAFE_NO_PAD.encode(&bad);
        let mut params = HashMap::new();
        params.insert("dns".to_string(), encoded);
        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp = handle_get_query(
            test_state(Arc::new(TestHandler), false),
            ConnectInfo(connect_addr),
            AxumQuery(params),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn handler_error_get_post_internal() {
        // GET
        let mut req = Message::new();
        req.set_id(0x77);
        req.set_query(true);
        let data = crate::dns::wire::serialize_message(&req).unwrap();
        let encoded = URL_SAFE_NO_PAD.encode(&data);
        let mut params = HashMap::new();
        params.insert("dns".to_string(), encoded);

        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp_get = handle_get_query(
            test_state(Arc::new(TestHandlerErr), false),
            ConnectInfo(connect_addr),
            AxumQuery(params.clone()),
            HeaderMap::new(),
        )
        .await;
        assert_eq!(resp_get.status(), StatusCode::INTERNAL_SERVER_ERROR);

        // POST
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "application/dns-message".parse().unwrap());
        let connect_addr = "127.0.0.1:12345".parse().unwrap();
        let resp_post = handle_post_query(
            test_state(Arc::new(TestHandlerErr), false),
            ConnectInfo(connect_addr),
            headers,
            AxumBytes::from(data),
        )
        .await;
        assert_eq!(resp_post.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // (Integration test moved to tests/integration_tls_doh_dot.rs)
}
