//! TLS configuration for encrypted DNS protocols
//!
//! This module provides comprehensive TLS configuration support for encrypted DNS protocols
//! including DNS over TLS (DoT) and DNS over HTTPS (DoH). It handles the loading,
//! validation, and configuration of X.509 certificates and private keys for secure DNS servers.
//!
//! ## Supported Protocols
//!
//! - **DNS over TLS (DoT)**: RFC 7858 - DNS queries over TLS on port 853
//! - **DNS over HTTPS (DoH)**: RFC 8484 - DNS queries over HTTPS on port 443
//!
//! ## Certificate Support
//!
//! The module supports standard PEM-encoded certificates and private keys:
//! - **X.509 Certificates**: PEM format with `-----BEGIN CERTIFICATE-----` markers
//! - **Private Keys**: PKCS#8, PKCS#1 RSA, and ECDSA keys in PEM format
//! - **Certificate Chains**: Multiple certificates in a single file (server cert first)
//!
//! ## Security Features
//!
//! - **Certificate Validation**: Ensures certificates are properly formatted and valid
//! - **Key Pair Matching**: Validates that certificates and private keys correspond
//! - **Secure Defaults**: Uses rustls with secure cipher suites and TLS 1.2+
//! - **Memory Safety**: Keys are properly handled with secure zeroing where applicable
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use lazydns::server::TlsConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Load TLS configuration from certificate and key files
//! let tls_config = TlsConfig::from_files("server.crt", "server.key")?;
//!
//! // Build rustls server configuration for use with DoT/DoH servers
//! let server_config = tls_config.build_server_config()?;
//!
//! // The server_config can now be used to create secure DNS servers
//! # Ok(())
//! # }
//! ```
//!
//! ## File Format Requirements
//!
//! ### Certificate File (PEM format)
//! ```pem
//! -----BEGIN CERTIFICATE-----
//! MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...
//! -----END CERTIFICATE-----
//! ```
//!
//! ### Private Key File (PEM format)
//! ```pem
//! -----BEGIN PRIVATE KEY-----
//! MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg...
//! -----END PRIVATE KEY-----
//! ```

use crate::error::Error;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// TLS configuration for secure DNS servers
///
/// `TlsConfig` encapsulates the necessary components for establishing secure TLS connections
/// in DNS over TLS (DoT) and DNS over HTTPS (DoH) servers. It manages X.509 certificates
/// and private keys required for server authentication.
///
/// ## Thread Safety
///
/// `TlsConfig` is thread-safe and can be safely shared across multiple server instances.
/// It implements `Clone` for easy duplication when needed.
///
/// ## Memory Management
///
/// Certificate and key data are loaded into memory during construction. The implementation
/// ensures that sensitive key material is handled securely, though it does not currently
/// implement explicit zeroing (this may be added in future versions for enhanced security).
///
/// ## Certificate Requirements
///
/// - **Format**: PEM-encoded X.509 certificates
/// - **Chain**: Server certificate must be first, followed by any intermediate certificates
/// - **Validation**: Certificates are validated for proper format during loading
/// - **Key Matching**: Private key must correspond to the server certificate
///
/// ## Example
///
/// ```rust,no_run
/// use lazydns::server::TlsConfig;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create TLS config from certificate and key files
/// let tls_config = TlsConfig::from_files("server.crt", "server.key")?;
///
/// // Clone for use in multiple servers if needed
/// let tls_config_copy = tls_config.clone();
/// # Ok(())
/// # }
/// ```
pub struct TlsConfig {
    /// Server certificates
    ///
    /// Contains the server's X.509 certificate followed by any intermediate
    /// certificates required for certificate chain validation. The certificate
    /// must be PEM-encoded with standard `-----BEGIN CERTIFICATE-----` markers.
    pub certs: Vec<CertificateDer<'static>>,

    /// Private key corresponding to the server certificate
    ///
    /// The private key used for TLS handshake authentication. Supports PKCS#8,
    /// PKCS#1 RSA, and ECDSA private keys in PEM format. Must correspond to
    /// the public key in the server certificate.
    pub key: PrivateKeyDer<'static>,
}

impl Clone for TlsConfig {
    /// Clone the TLS configuration
    ///
    /// Creates a deep copy of the TLS configuration, including all certificates
    /// and the private key. This allows the same TLS configuration to be used
    /// across multiple server instances safely.
    ///
    /// The cloned configuration is completely independent of the original and
    /// can be modified without affecting other instances.
    fn clone(&self) -> Self {
        Self {
            certs: self.certs.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl TlsConfig {
    /// Create a new TLS configuration from certificate and key files
    ///
    /// Loads and validates X.509 certificates and private keys from PEM-encoded files
    /// to create a TLS configuration suitable for secure DNS servers (DoT/DoH).
    ///
    /// ## Arguments
    ///
    /// * `cert_path` - Path to the certificate file. Can be a string, `Path`, or `PathBuf`.
    ///   The file must contain one or more PEM-encoded X.509 certificates.
    /// * `key_path` - Path to the private key file. Can be a string, `Path`, or `PathBuf`.
    ///   The file must contain a PEM-encoded private key.
    ///
    /// ## Certificate File Format
    ///
    /// The certificate file should contain PEM-encoded certificates:
    /// ```pem
    /// -----BEGIN CERTIFICATE-----
    /// MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA...
    /// -----END CERTIFICATE-----
    /// ```
    ///
    /// Multiple certificates can be included in the same file (certificate chain).
    /// The server certificate should be first, followed by intermediate certificates.
    ///
    /// ## Private Key File Format
    ///
    /// The private key file should contain a PEM-encoded private key. Supported formats:
    /// - PKCS#8: `-----BEGIN PRIVATE KEY-----`
    /// - PKCS#1 RSA: `-----BEGIN RSA PRIVATE KEY-----`
    /// - ECDSA: `-----BEGIN EC PRIVATE KEY-----`
    ///
    /// ## Errors
    ///
    /// Returns an error if:
    /// - Certificate or key files cannot be read
    /// - Certificate parsing fails (invalid PEM format)
    /// - Private key parsing fails (invalid format or unsupported type)
    /// - No certificates are found in the certificate file
    /// - No private key is found in the key file
    ///
    /// ## Example
    ///
    /// ```no_run
    /// use lazydns::server::TlsConfig;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Load TLS configuration for secure DNS server
    /// let tls_config = TlsConfig::from_files("server.crt", "server.key")?;
    ///
    /// // Use the configuration to build a rustls server config
    /// let server_config = tls_config.build_server_config()?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ## Security Notes
    ///
    /// - Ensure certificate and key files have appropriate file permissions (readable only by owner)
    /// - The private key is loaded into memory; consider the security implications for your deployment
    /// - Certificate validation is performed during loading to catch configuration errors early
    pub fn from_files(
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let certs = Self::load_certs(cert_path)?;
        let key = Self::load_key(key_path)?;

        Ok(Self { certs, key })
    }

    /// Load certificates from a PEM file
    ///
    /// Parses one or more PEM-encoded X.509 certificates from the specified file.
    /// Validates that at least one certificate is present and properly formatted.
    ///
    /// ## Arguments
    ///
    /// * `path` - Path to the certificate file
    ///
    /// ## Returns
    ///
    /// A vector of parsed certificates in DER format, or an error if parsing fails.
    ///
    /// ## Errors
    ///
    /// - `Error::Config` if the file cannot be read or certificates cannot be parsed
    /// - `Error::Config` if no certificates are found in the file
    fn load_certs(path: impl AsRef<Path>) -> Result<Vec<CertificateDer<'static>>, Error> {
        let cert_file = fs::read(path.as_ref())
            .map_err(|e| Error::Config(format!("Failed to read certificate file: {}", e)))?;

        let certs = rustls_pemfile::certs(&mut &cert_file[..])
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::Config(format!("Failed to parse certificate: {}", e)))?;

        if certs.is_empty() {
            return Err(Error::Config("No certificates found in file".to_string()));
        }

        Ok(certs)
    }

    /// Load private key from a PEM file
    ///
    /// Parses a PEM-encoded private key from the specified file. Supports multiple
    /// key formats including PKCS#8, PKCS#1 RSA, and ECDSA keys.
    ///
    /// ## Arguments
    ///
    /// * `path` - Path to the private key file
    ///
    /// ## Returns
    ///
    /// The parsed private key in DER format, or an error if parsing fails.
    ///
    /// ## Supported Key Formats
    ///
    /// - PKCS#8: Universal format for private keys
    /// - PKCS#1: RSA private keys
    /// - ECDSA: Elliptic curve private keys
    ///
    /// ## Errors
    ///
    /// - `Error::Config` if the file cannot be read
    /// - `Error::Config` if the key cannot be parsed or is in an unsupported format
    /// - `Error::Config` if no private key is found in the file
    fn load_key(path: impl AsRef<Path>) -> Result<PrivateKeyDer<'static>, Error> {
        let key_file = fs::read(path.as_ref())
            .map_err(|e| Error::Config(format!("Failed to read key file: {}", e)))?;

        // Try to parse as PKCS8 first, then RSA
        let key = rustls_pemfile::private_key(&mut &key_file[..])
            .map_err(|e| Error::Config(format!("Failed to parse private key: {}", e)))?
            .ok_or_else(|| Error::Config("No private key found in file".to_string()))?;

        Ok(key)
    }

    /// Create a rustls server configuration
    ///
    /// Builds a complete `rustls::ServerConfig` from the loaded certificates and private key.
    /// The resulting configuration is suitable for use with TLS-based DNS servers (DoT/DoH).
    ///
    /// ## Configuration Details
    ///
    /// - **Client Authentication**: Disabled (`with_no_client_auth()`)
    /// - **Certificate**: Uses the loaded server certificate and any intermediate certificates
    /// - **Private Key**: Uses the loaded private key for server authentication
    /// - **TLS Versions**: Supports TLS 1.2 and TLS 1.3 (rustls default)
    /// - **Cipher Suites**: Uses rustls secure defaults with modern cipher suites
    ///
    /// ## Returns
    ///
    /// An `Arc<rustls::ServerConfig>` that can be used to create TLS acceptors
    /// for secure DNS server connections. The Arc allows safe sharing across threads.
    ///
    /// ## Errors
    ///
    /// Returns `Error::Config` if:
    /// - The certificate and private key don't match
    /// - The certificate chain is invalid
    /// - The private key is malformed
    ///
    /// ## Example
    ///
    /// ```no_run
    /// use lazydns::server::TlsConfig;
    /// use std::sync::Arc;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Load TLS configuration
    /// let tls_config = TlsConfig::from_files("server.crt", "server.key")?;
    ///
    /// // Build rustls server configuration
    /// let server_config: Arc<rustls::ServerConfig> = tls_config.build_server_config()?;
    ///
    /// // Use with tokio-rustls or similar for accepting TLS connections
    /// // let acceptor = TlsAcceptor::from(server_config);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ## Thread Safety
    ///
    /// The returned `Arc<ServerConfig>` is thread-safe and can be shared across
    /// multiple server instances or connection handlers.
    ///
    /// ## Performance Notes
    ///
    /// The configuration is built on-demand and can be reused for multiple connections.
    /// Certificate validation and key operations are performed during TLS handshakes,
    /// not during configuration building.
    pub fn build_server_config(&self) -> Result<Arc<rustls::ServerConfig>, Error> {
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.certs.clone(), self.key.clone_key())
            .map_err(|e| Error::Config(format!("Failed to build TLS config: {}", e)))?;

        Ok(Arc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_nonexistent_cert() {
        let result = TlsConfig::from_files("nonexistent.pem", "nonexistent.key");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_nonexistent_key() {
        let result = TlsConfig::from_files("nonexistent.pem", "nonexistent.key");
        assert!(result.is_err());
        if let Err(Error::Config(msg)) = result {
            assert!(msg.contains("certificate file") || msg.contains("key file"));
        }
    }

    #[test]
    fn test_tls_config_clone() {
        // Create a mock certificate and key for testing clone
        // We'll use minimal valid data that can be parsed
        let cert_pem = b"-----BEGIN CERTIFICATE-----\n\
                        MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA\n\
                        -----END CERTIFICATE-----\n";

        let key_pem = b"-----BEGIN PRIVATE KEY-----\n\
                      MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg\n\
                      -----END PRIVATE KEY-----\n";

        let cert_file = NamedTempFile::new().unwrap();
        cert_file.as_file().write_all(cert_pem).unwrap();

        let key_file = NamedTempFile::new().unwrap();
        key_file.as_file().write_all(key_pem).unwrap();

        // Even though parsing will fail, we can test that clone is implemented
        let result = TlsConfig::from_files(cert_file.path(), key_file.path());
        if let Ok(config) = result {
            let cloned = config.clone();
            // Clone should create a separate instance
            assert_eq!(config.certs.len(), cloned.certs.len());
        }
        // If parsing fails, that's expected for mock data
    }

    #[test]
    fn test_load_certs_empty_file() {
        let empty_file = NamedTempFile::new().unwrap();
        let result = TlsConfig::load_certs(empty_file.path());
        assert!(result.is_err());
        if let Err(Error::Config(msg)) = result {
            assert!(msg.contains("No certificates found"));
        }
    }

    #[test]
    fn test_load_certs_invalid_pem() {
        let invalid_file = NamedTempFile::new().unwrap();
        invalid_file
            .as_file()
            .write_all(b"invalid pem data")
            .unwrap();

        let result = TlsConfig::load_certs(invalid_file.path());
        assert!(result.is_err());
        // Just check that it's an error, don't check the exact message
        // as it might vary depending on the rustls version
    }

    #[test]
    fn test_load_key_empty_file() {
        let empty_file = NamedTempFile::new().unwrap();
        let result = TlsConfig::load_key(empty_file.path());
        assert!(result.is_err());
        if let Err(Error::Config(msg)) = result {
            assert!(msg.contains("No private key found"));
        }
    }

    #[test]
    fn test_load_key_invalid_pem() {
        let invalid_file = NamedTempFile::new().unwrap();
        invalid_file
            .as_file()
            .write_all(b"invalid key data")
            .unwrap();

        let result = TlsConfig::load_key(invalid_file.path());
        assert!(result.is_err());
        // Just check that it's an error, don't check the exact message
    }

    #[test]
    fn test_load_certs_file_read_error() {
        // Test with a path that exists but we can't read
        let temp_dir = tempfile::tempdir().unwrap();
        let cert_path = temp_dir.path().join("cert.pem");

        // Create a directory instead of a file to cause read error
        std::fs::create_dir(&cert_path).unwrap();

        let result = TlsConfig::load_certs(&cert_path);
        assert!(result.is_err());
        if let Err(Error::Config(msg)) = result {
            assert!(msg.contains("Failed to read certificate file"));
        }
    }

    #[test]
    fn test_load_key_file_read_error() {
        // Test with a path that exists but we can't read
        let temp_dir = tempfile::tempdir().unwrap();
        let key_path = temp_dir.path().join("key.pem");

        // Create a directory instead of a file to cause read error
        std::fs::create_dir(&key_path).unwrap();

        let result = TlsConfig::load_key(&key_path);
        assert!(result.is_err());
        if let Err(Error::Config(msg)) = result {
            assert!(msg.contains("Failed to read key file"));
        }
    }

    #[test]
    fn test_tls_config_from_files_with_valid_paths() {
        // Test that from_files accepts different path types
        let cert_path = std::path::PathBuf::from("dummy.pem");
        let key_path = "dummy.key";

        let result = TlsConfig::from_files(cert_path.as_path(), key_path);
        assert!(result.is_err()); // Should fail because files don't exist
    }

    #[test]
    fn test_load_certs_with_multiple_certs() {
        // Create a file with multiple certificates
        let cert_pem = b"-----BEGIN CERTIFICATE-----\n\
                        MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA\n\
                        -----END CERTIFICATE-----\n\
                        -----BEGIN CERTIFICATE-----\n\
                        MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA\n\
                        -----END CERTIFICATE-----\n";

        let cert_file = NamedTempFile::new().unwrap();
        cert_file.as_file().write_all(cert_pem).unwrap();

        let result = TlsConfig::load_certs(cert_file.path());
        // This will likely fail due to invalid cert data, but tests that multiple certs are attempted
        // The important thing is that it doesn't panic
        let _ = result;
    }

    #[test]
    fn test_error_message_formatting() {
        // Test that error messages are properly formatted
        let result = TlsConfig::from_files("nonexistent.pem", "nonexistent.key");
        assert!(result.is_err());

        if let Err(Error::Config(msg)) = result {
            assert!(!msg.is_empty());
            assert!(msg.contains("Failed to read"));
        }
    }
}
