//! DNS over QUIC (DoQ) server implementation using `quinn`.
//!
//! This module implements a small, test-friendly DoQ server built on top of
//! the `quinn` QUIC implementation. Design notes:
//!
//! - Each incoming QUIC connection accepts bi-directional streams. Each
//!   bi-directional stream is used for a single DNS query/response exchange
//!   using the same 2-byte length-prefixed wire framing as used by TCP/DoT.
//! - TLS certificates are read from PEM files and converted into a `rustls`
//!   server configuration which is then converted into the `quinn` crypto
//!   configuration. The helper `build_quic_server_config` performs this
//!   conversion and performs basic validation of the parsed materials.
//! - The implementation is intentionally small and test-oriented; it expects
//!   the caller to provide a `RequestHandler` implementation that performs
//!   DNS business logic and returns a `dns::Message` response. The server
//!   handles connection/stream accept loops and maps IO/TLS errors into the
//!   crate `Error` type.
//!
//! Note about rustls providers: `rustls` v0.23 requires a process-level
//! crypto provider (for example the `ring` feature) or an explicit runtime
//! installation. Tests and binaries should ensure a provider is installed
//! (see `rustls::crypto::ring::default_provider().install_default()`).

use crate::server::RequestHandler;
use crate::{Result, server::Server};
use quinn::{Endpoint, ServerConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, info};

/// DNS over QUIC server
pub struct DoqServer {
    addr: String,
    cert_path: String,
    key_path: String,
    handler: Arc<dyn RequestHandler>,
}

impl std::fmt::Debug for DoqServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoqServer")
            .field("addr", &self.addr)
            .field("cert_path", &self.cert_path)
            .field("key_path", &self.key_path)
            .finish_non_exhaustive()
    }
}

impl DoqServer {
    /// Create a new `DoqServer`.
    ///
    /// - `addr` is the socket address to bind (such as "127.0.0.1:784").
    /// - `cert_path` and `key_path` are filesystem paths to PEM-encoded
    ///   certificate and private key files respectively.
    /// - `handler` is an `Arc` to a `RequestHandler` which will be invoked
    ///   for each parsed DNS request.
    pub fn new(
        addr: impl Into<String>,
        cert_path: impl Into<String>,
        key_path: impl Into<String>,
        handler: Arc<dyn RequestHandler>,
    ) -> Self {
        Self {
            addr: addr.into(),
            cert_path: cert_path.into(),
            key_path: key_path.into(),
            handler,
        }
    }

    /// Run the DoQ server listening on the configured address.
    pub async fn run(self) -> Result<()> {
        info!(addr = %self.addr, "Starting DoQ server");

        // Build QUIC server config from TLS certificates
        let server_config = build_quic_server_config(&self.cert_path, &self.key_path)?;

        let addr: SocketAddr = self
            .addr
            .parse()
            .map_err(|e| crate::Error::Config(format!("Invalid DoQ bind address: {}", e)))?;

        let endpoint = Endpoint::server(server_config, addr)?;
        let local = endpoint
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        info!(local = %local, "DoQ listening");

        // Accept incoming QUIC connections
        while let Some(incoming) = endpoint.accept().await {
            let handler = Arc::clone(&self.handler);

            // Spawn a task per accepted connection; each connection will
            // accept bi-directional streams and spawn a task per stream.
            tokio::spawn(async move {
                match incoming.await {
                    Ok(connection) => {
                        info!(remote = %connection.remote_address(), "Accepted QUIC connection");
                        // Per-connection: accept bi-directional streams
                        loop {
                            match connection.accept_bi().await {
                                Ok((send, recv)) => {
                                    let handler = Arc::clone(&handler);
                                    tokio::spawn(async move {
                                        if let Err(e) = handle_stream(recv, send, handler).await {
                                            debug!("DoQ stream error: {}", e);
                                        }
                                    });
                                }
                                Err(e) => {
                                    debug!("Connection stream accept error: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to accept incoming QUIC connection: {}", e);
                    }
                }
            });
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl Server for DoqServer {
    async fn from_config(config: crate::server::ServerConfig) -> crate::Result<Self> {
        let addr = config
            .tcp_addr
            .ok_or_else(|| crate::Error::Config("Address not configured for DoQ".to_string()))?
            .to_string();

        let cert_path = config
            .cert_path
            .ok_or_else(|| crate::Error::Config("Cert path not configured for DoQ".to_string()))?;

        let key_path = config
            .key_path
            .ok_or_else(|| crate::Error::Config("Key path not configured for DoQ".to_string()))?;

        let handler = config
            .handler
            .ok_or_else(|| crate::Error::Config("Handler not configured".to_string()))?;

        Ok(Self::new(addr, cert_path, key_path, handler))
    }

    async fn run(self) -> crate::Result<()> {
        DoqServer::run(self).await
    }
}

/// Handle a single bi-directional stream carrying a DNS query/response.
async fn handle_stream(
    mut recv: quinn::RecvStream,
    mut send: quinn::SendStream,
    handler: Arc<dyn RequestHandler>,
) -> Result<()> {
    // Read 2-byte length prefix (network byte order)
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| crate::Error::Io(std::io::Error::other(e)))?;
    let msg_len = u16::from_be_bytes(len_buf) as usize;

    // Read DNS message
    let mut buf = vec![0u8; msg_len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| crate::Error::Io(std::io::Error::other(e)))?;

    // Parse DNS message and handle
    let request = crate::dns::wire::parse_message(&buf)?;
    let ctx = crate::server::RequestContext::new(request, crate::server::Protocol::DoQ);
    let response = handler.handle(ctx).await?;
    let resp_data = crate::dns::wire::serialize_message(&response)?;

    // Write response with length prefix

    // Send a 2-byte length prefix followed by the DNS message body.
    // QUIC `SendStream::write_all` is async and may error; map into crate IO
    // errors for uniform handling.
    send.write_all(&(resp_data.len() as u16).to_be_bytes())
        .await
        .map_err(|e| crate::Error::Io(std::io::Error::other(e)))?;
    send.write_all(&resp_data)
        .await
        .map_err(|e| crate::Error::Io(std::io::Error::other(e)))?;

    // Finalize the send side of the stream. `finish()` will return an
    // error if the underlying connection is terminated; map that to the
    // crate IO error type as well.
    send.finish()
        .map_err(|e| crate::Error::Io(std::io::Error::other(e)))?;

    Ok(())
}

/// Build a QUIC server config from PEM-encoded TLS certificate and key files.
fn build_quic_server_config(cert_path: &str, key_path: &str) -> Result<ServerConfig> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::fs;

    let cert_bytes = fs::read(cert_path)
        .map_err(|e| crate::Error::Config(format!("Failed to read cert: {}", e)))?;
    let key_bytes = fs::read(key_path)
        .map_err(|e| crate::Error::Config(format!("Failed to read key: {}", e)))?;

    // Parse certificates from PEM: certs() returns an iterator that yields Result<CertificateDer, io::Error>
    let certs_result: std::result::Result<Vec<CertificateDer>, _> =
        rustls_pemfile::certs(&mut &cert_bytes[..]).collect();
    let certs = certs_result
        .map_err(|e| crate::Error::Config(format!("Failed to parse cert PEM: {}", e)))?;
    if certs.is_empty() {
        return Err(crate::Error::Config(
            "No certificates found in cert file".to_string(),
        ));
    }

    // Parse private key from PEM using `read_one`. `rustls_pemfile::read_one`
    // will return the first PEM item; we support common key encodings used in
    // tests and production (PKCS#8, PKCS#1/SEC1). If an unsupported item is
    // encountered an error is returned to the caller.
    let mut key_reader = &key_bytes[..];
    let key = rustls_pemfile::read_one(&mut key_reader)
        .map_err(|e| crate::Error::Config(format!("Failed to parse key PEM: {}", e)))?
        .ok_or_else(|| crate::Error::Config("No private key found in key file".to_string()))?;

    let key_der = match key {
        rustls_pemfile::Item::Pkcs8Key(k) => PrivateKeyDer::Pkcs8(k),
        rustls_pemfile::Item::Sec1Key(k) => PrivateKeyDer::Sec1(k),
        rustls_pemfile::Item::Pkcs1Key(k) => PrivateKeyDer::Pkcs1(k),
        _ => {
            return Err(crate::Error::Config(
                "Unsupported private key type".to_string(),
            ));
        }
    };

    // Build rustls ServerConfig from parsed certificate and private key.
    // This may fail if the certificate and key do not match or are invalid.
    let rustls_cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key_der)
        .map_err(|e| crate::Error::Config(format!("Failed to build rustls config: {}", e)))?;

    // Convert to quinn QuicServerConfig. `quinn` expects a specific crypto
    // configuration derived from `rustls::ServerConfig`; this conversion may
    // surface configuration incompatibilities and is mapped into a Config
    // error on failure.
    let quic_crypto =
        quinn::crypto::rustls::QuicServerConfig::try_from(rustls_cfg).map_err(|e| {
            crate::Error::Config(format!("Failed to convert rustls -> quinn crypto: {}", e))
        })?;
    let server_config = ServerConfig::with_crypto(Arc::new(quic_crypto));

    Ok(server_config)
}

#[cfg(all(test, feature = "doq"))]
mod tests {
    use super::*;
    use rcgen::generate_simple_self_signed;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_build_quic_server_config_from_pem() {
        // Ensure a CryptoProvider is installed for rustls v0.23
        let _ = rustls::crypto::ring::default_provider().install_default();

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let key_pem = cert.signing_key.serialize_pem();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_str().unwrap().to_string();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        let cfg = build_quic_server_config(&cert_path, &key_path).expect("build quic cfg");
        // Basic sanity: ensure conversion succeeded (cfg constructed)
        let _ = cfg;
    }

    #[test]
    fn test_build_quic_server_config_missing_cert() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Generate a valid key file but use non-existent cert path
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key_pem = cert.signing_key.serialize_pem();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        let result = build_quic_server_config("/nonexistent/cert.pem", &key_path);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::Error::Config(msg) => assert!(msg.contains("Failed to read cert")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[test]
    fn test_build_quic_server_config_missing_key() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Generate a valid cert file but use non-existent key path
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_str().unwrap().to_string();

        let result = build_quic_server_config(&cert_path, "/nonexistent/key.pem");
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::Error::Config(msg) => assert!(msg.contains("Failed to read key")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[test]
    fn test_build_quic_server_config_invalid_cert_pem() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Create files with invalid PEM content
        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(b"not valid PEM content").unwrap();
        let cert_path = cert_file.path().to_str().unwrap().to_string();

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key_pem = cert.signing_key.serialize_pem();
        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        let result = build_quic_server_config(&cert_path, &key_path);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::Error::Config(msg) => {
                assert!(
                    msg.contains("No certificates found"),
                    "Expected 'No certificates found', got: {}",
                    msg
                );
            }
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[test]
    fn test_build_quic_server_config_empty_cert_file() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Create empty cert file
        let cert_file = NamedTempFile::new().unwrap();
        let cert_path = cert_file.path().to_str().unwrap().to_string();

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let key_pem = cert.signing_key.serialize_pem();
        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        let result = build_quic_server_config(&cert_path, &key_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_quic_server_config_empty_key_file() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_str().unwrap().to_string();

        // Create empty key file
        let key_file = NamedTempFile::new().unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        let result = build_quic_server_config(&cert_path, &key_path);
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::Error::Config(msg) => {
                assert!(
                    msg.contains("No private key found"),
                    "Expected 'No private key found', got: {}",
                    msg
                );
            }
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[test]
    fn test_doq_server_new() {
        struct DummyHandler;
        #[async_trait::async_trait]
        impl RequestHandler for DummyHandler {
            async fn handle(
                &self,
                ctx: crate::server::RequestContext,
            ) -> crate::Result<crate::dns::Message> {
                let req = ctx.into_message();
                Ok(req)
            }
        }

        let server = DoqServer::new(
            "127.0.0.1:8853",
            "/path/to/cert.pem",
            "/path/to/key.pem",
            Arc::new(DummyHandler),
        );

        assert_eq!(server.addr, "127.0.0.1:8853");
        assert_eq!(server.cert_path, "/path/to/cert.pem");
        assert_eq!(server.key_path, "/path/to/key.pem");
    }

    #[tokio::test]
    async fn test_doq_server_from_config_missing_addr() {
        let config = crate::server::ServerConfig {
            tcp_addr: None, // Explicitly clear the default address
            ..Default::default()
        };
        let result = DoqServer::from_config(config).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        match &err {
            crate::Error::Config(msg) => assert!(
                msg.contains("Address not configured"),
                "Expected error message to contain 'Address not configured', got: {}",
                msg
            ),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_doq_server_from_config_missing_cert() {
        let config = crate::server::ServerConfig {
            tcp_addr: Some("127.0.0.1:8853".parse().unwrap()),
            ..Default::default()
        };
        let result = DoqServer::from_config(config).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        match &err {
            crate::Error::Config(msg) => assert!(msg.contains("Cert path not configured")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_doq_server_from_config_missing_key() {
        let config = crate::server::ServerConfig {
            tcp_addr: Some("127.0.0.1:8853".parse().unwrap()),
            cert_path: Some("/path/to/cert.pem".to_string()),
            ..Default::default()
        };
        let result = DoqServer::from_config(config).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        match &err {
            crate::Error::Config(msg) => assert!(msg.contains("Key path not configured")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_doq_server_from_config_missing_handler() {
        let config = crate::server::ServerConfig {
            tcp_addr: Some("127.0.0.1:8853".parse().unwrap()),
            cert_path: Some("/path/to/cert.pem".to_string()),
            key_path: Some("/path/to/key.pem".to_string()),
            ..Default::default()
        };
        let result = DoqServer::from_config(config).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        match &err {
            crate::Error::Config(msg) => assert!(msg.contains("Handler not configured")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_doq_server_run_invalid_address() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Generate valid cert/key
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_pem = cert.cert.pem();
        let key_pem = cert.signing_key.serialize_pem();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let cert_path = cert_file.path().to_str().unwrap().to_string();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        struct DummyHandler;
        #[async_trait::async_trait]
        impl RequestHandler for DummyHandler {
            async fn handle(
                &self,
                ctx: crate::server::RequestContext,
            ) -> crate::Result<crate::dns::Message> {
                let req = ctx.into_message();
                Ok(req)
            }
        }

        // Invalid address should fail to parse
        let server = DoqServer::new(
            "not-a-valid-addr",
            cert_path,
            key_path,
            Arc::new(DummyHandler),
        );
        let result = server.run().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::Error::Config(msg) => assert!(msg.contains("Invalid DoQ bind address")),
            other => panic!("Expected Config error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_doq_server_from_config_complete() {
        struct DummyHandler;
        #[async_trait::async_trait]
        impl RequestHandler for DummyHandler {
            async fn handle(
                &self,
                ctx: crate::server::RequestContext,
            ) -> crate::Result<crate::dns::Message> {
                let req = ctx.into_message();
                Ok(req)
            }
        }

        let config = crate::server::ServerConfig {
            tcp_addr: Some("127.0.0.1:8853".parse().unwrap()),
            cert_path: Some("/path/to/cert.pem".to_string()),
            key_path: Some("/path/to/key.pem".to_string()),
            handler: Some(Arc::new(DummyHandler)),
            ..Default::default()
        };

        let result = DoqServer::from_config(config).await;
        assert!(result.is_ok());
        let server = result.unwrap();
        assert_eq!(server.addr, "127.0.0.1:8853");
    }
}
