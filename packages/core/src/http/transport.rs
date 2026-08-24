//! Transport selection: QUIC (UDP) vs TCP fallback.
//!
//! LocalSend+ prefers QUIC for bulk transfers: TCP's single ordered stream
//! suffers head-of-line blocking when one segment is lost, which QUIC (UDP,
//! independent streams) avoids. QUIC is an opt-in build feature
//! (`--features quic`); when it is not compiled in, or the runtime cannot bind
//! a UDP socket, transfers transparently fall back to the TCP/HTTP transport
//! and a log line records why — so a missing QUIC build never breaks transfers.

use std::net::SocketAddr;

/// The transport used for a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// QUIC (UDP-based, no head-of-line blocking).
    Quic,
    /// TCP (HTTP/1.1), the protocol-v2 default.
    Tcp,
}

impl Transport {
    /// The protocol identifier used in logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Quic => "quic",
            Transport::Tcp => "tcp",
        }
    }
}

/// The outcome of selecting a transport, including a human-readable reason
/// (intended to be logged by the caller).
#[derive(Debug)]
pub struct TransportSelection {
    /// The selected transport.
    pub transport: Transport,
    /// Why this transport was selected.
    pub reason: &'static str,
}

/// Whether QUIC support was compiled in (`--features quic`).
pub fn quic_compiled_in() -> bool {
    cfg!(feature = "quic")
}

/// Selects the transport for a bulk transfer to `remote`.
///
/// * `prefer_quic` is caller policy (e.g. a user setting).
/// * QUIC is only chosen when it was compiled in AND a UDP socket can be bound
///   to reach `remote`; otherwise TCP is returned with a reason to log.
///
/// The decision is also logged here so operators can confirm the fallback.
pub async fn select_transport(remote: SocketAddr, prefer_quic: bool) -> TransportSelection {
    if !prefer_quic {
        return TransportSelection {
            transport: Transport::Tcp,
            reason: "QUIC disabled by policy",
        };
    }
    if !quic_compiled_in() {
        tracing::info!("QUIC not compiled in (build without `--features quic`); falling back to TCP");
        return TransportSelection {
            transport: Transport::Tcp,
            reason: "QUIC not compiled in",
        };
    }
    match quic_runtime_available(remote).await {
        Ok(()) => TransportSelection {
            transport: Transport::Quic,
            reason: "QUIC available",
        },
        Err(err) => {
            tracing::warn!("QUIC unavailable ({err}); falling back to TCP");
            TransportSelection {
                transport: Transport::Tcp,
                reason: "QUIC unavailable at runtime",
            }
        }
    }
}

/// Probes whether a QUIC (UDP) endpoint can be bound for `remote`.
///
/// The probe itself is dependency-free; the real QUIC backend lives behind the
/// `quic` feature. This keeps the fallback decision real even when the QUIC
/// backend is not compiled in.
#[cfg(feature = "quic")]
async fn quic_runtime_available(remote: SocketAddr) -> Result<(), String> {
    crate::http::quic::is_available(remote).await
}

#[cfg(not(feature = "quic"))]
async fn quic_runtime_available(_remote: SocketAddr) -> Result<(), String> {
    Err("not compiled in".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_names_are_stable() {
        assert_eq!(Transport::Quic.as_str(), "quic");
        assert_eq!(Transport::Tcp.as_str(), "tcp");
    }

    #[tokio::test]
    async fn policy_off_selects_tcp() {
        let selection = select_transport("127.0.0.1:1".parse().unwrap(), false).await;
        assert_eq!(selection.transport, Transport::Tcp);
        assert_eq!(selection.reason, "QUIC disabled by policy");
    }
}
