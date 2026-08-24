//! QUIC transport backend (opt-in via the `quic` feature).
//!
//! # Status
//!
//! This backend is compiled only with `--features quic`. It follows the
//! **quinn 0.11** API; the call sites below were written against that version
//! and must be verified against the exact revision resolved by `Cargo.lock`
//! before enabling the feature in production builds. The transport selector in
//! [`super::transport`] always falls back to TCP when this backend is
//! unavailable, so a non-`quic` build (and any runtime QUIC failure) never
//! breaks transfers.
//!
//! # Why QUIC
//!
//! TCP delivers one ordered byte stream; a single lost segment stalls every
//! subsequent byte (head-of-line blocking). QUIC multiplexes independent
//! streams over UDP, so one file's stall does not block the others, and a
//! single lost packet is retransmitted without reordering the whole session.

use std::net::SocketAddr;
use std::sync::Arc;

/// The ALPN protocol negotiated for the LocalSend+ QUIC transport.
pub const ALPN: &[u8] = b"localsend/quic/1";

/// Probes whether a QUIC endpoint can be bound for the given remote address.
///
/// A QUIC endpoint needs a UDP socket. Binding a wildcard UDP socket of the
/// same address family is a cheap, reliable availability probe that does not
/// require a certificate.
pub async fn is_available(remote: SocketAddr) -> Result<(), String> {
    let bind_addr = match remote {
        SocketAddr::V4(_) => "0.0.0.0:0",
        SocketAddr::V6(_) => "[::]:0",
    };
    std::net::UdpSocket::bind(bind_addr)
        .map(|_| ())
        .map_err(|e| format!("cannot bind UDP socket for QUIC: {e}"))
}

/// A QUIC server endpoint carrying the same TLS identity as the HTTPS
/// transport (the per-device on-the-fly certificate).
pub struct QuicServerEndpoint {
    #[allow(dead_code)] // The endpoint owns the server socket and tasks.
    endpoint: quinn::Endpoint,
}

impl QuicServerEndpoint {
    /// Binds a QUIC server endpoint to `addr`, using the PEM-encoded
    /// certificate and private key of the device.
    pub fn server(addr: SocketAddr, cert: &str, key: &str) -> Result<Self, String> {
        let rustls_config = rustls_server_config(cert, key)?;
        let quic_crypto = Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(rustls_config)
                .map_err(|e| format!("failed to build QUIC crypto config: {e}"))?,
        );
        let mut server_config = quinn::ServerConfig::with_crypto(quic_crypto);
        server_config.alpn_protocols = vec![ALPN.to_vec()];

        let endpoint = quinn::Endpoint::server(server_config, addr)
            .map_err(|e| format!("failed to bind QUIC endpoint: {e}"))?;

        tracing::info!("QUIC server listening on {addr}");
        Ok(Self { endpoint })
    }

    /// The local address the endpoint is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, String> {
        self.endpoint
            .local_addr()
            .map_err(|e| format!("failed to read QUIC local address: {e}"))
    }
}

/// Builds a [`rustls::ServerConfig`] from the device's PEM certificate and key,
/// reusing the same identity as the HTTPS transport.
fn rustls_server_config(cert: &str, key: &str) -> Result<rustls::ServerConfig, String> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let certs = vec![
        CertificateDer::from_pem_slice(cert.as_bytes()).map_err(|e| e.to_string())?,
    ];
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).map_err(|e| e.to_string())?;

    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("failed to build QUIC TLS config: {e}"))
}
