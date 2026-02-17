use rsipstack::transport::tcp::TcpConnection;
use rsipstack::transport::tls::TlsConnection;
use rsipstack::transport::udp::UdpConnection;
use rsipstack::transport::websocket::WebSocketConnection;
use rsipstack::transport::{SipAddr, SipConnection};
use rsipstack::Error;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring::default_provider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::debug;

/// TLS verifier that skips certificate chain validation (accepts self-signed certs).
/// Signature verification is still performed to prevent MITM attacks.
#[derive(Debug)]
struct SkipCertVerifier;

impl ServerCertVerifier for SkipCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Protocol enum to represent SIP transport protocols
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Udp,
    Tcp,
    Tls,
    TlsSctp,
    Sctp,
    Ws,
    Wss,
}

impl Protocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Protocol::Udp => "UDP",
            Protocol::Tcp => "TCP",
            Protocol::Tls => "TLS",
            Protocol::TlsSctp => "TLS-SCTP",
            Protocol::Sctp => "SCTP",
            Protocol::Ws => "WS",
            Protocol::Wss => "WSS",
        }
    }
}

impl From<Protocol> for rsip::transport::Transport {
    fn from(protocol: Protocol) -> Self {
        match protocol {
            Protocol::Udp => rsip::transport::Transport::Udp,
            Protocol::Tcp => rsip::transport::Transport::Tcp,
            Protocol::Tls => rsip::transport::Transport::Tls,
            Protocol::TlsSctp => rsip::transport::Transport::Tls,
            Protocol::Sctp => rsip::transport::Transport::Sctp,
            Protocol::Ws => rsip::transport::Transport::Ws,
            Protocol::Wss => rsip::transport::Transport::Wss,
        }
    }
}

/// Extract transport protocol from SIP URI
pub fn extract_protocol_from_uri(uri: &rsip::Uri) -> Protocol {
    for param in &uri.params {
        if let rsip::Param::Transport(transport) = param {
            return match transport {
                rsip::transport::Transport::Udp => Protocol::Udp,
                rsip::transport::Transport::Tcp => Protocol::Tcp,
                rsip::transport::Transport::Tls => Protocol::Tls,
                rsip::transport::Transport::TlsSctp => Protocol::TlsSctp,
                rsip::transport::Transport::Sctp => Protocol::Sctp,
                rsip::transport::Transport::Ws => Protocol::Ws,
                rsip::transport::Transport::Wss => Protocol::Wss,
            };
        }
    }

    if let Some(rsip::Scheme::Sips) = uri.scheme {
        return Protocol::Tls;
    }

    Protocol::Udp
}

/// Resolve the hostname in a SipAddr to an IP address via DNS.
/// TCP/TLS connections require a resolved SocketAddr; UDP does not.
async fn resolve_sip_addr(target: &SipAddr) -> rsipstack::Result<SipAddr> {
    let host_str = target.addr.to_string();
    // If it already parses as SocketAddr (i.e. it's an IP), return as-is
    if host_str.parse::<SocketAddr>().is_ok() {
        return Ok(target.clone());
    }
    debug!(host = %host_str, "Resolving hostname via DNS");
    let mut addrs = tokio::net::lookup_host(&host_str)
        .await
        .map_err(|e| Error::Error(format!("DNS resolution failed for '{}': {}", host_str, e)))?;
    let resolved: SocketAddr = addrs
        .next()
        .ok_or_else(|| Error::Error(format!("No address found for '{}'", host_str)))?;
    debug!(host = %host_str, resolved = %resolved, "DNS resolved");
    Ok(SipAddr {
        r#type: target.r#type,
        addr: resolved.into(),
    })
}

/// Create transport connection based on protocol
pub async fn create_transport_connection(
    local_addr: SocketAddr,
    target: SipAddr,
    cancel_token: CancellationToken,
) -> rsipstack::Result<SipConnection> {
    match target.r#type {
        Some(rsip::transport::Transport::Udp) => {
            let connection = UdpConnection::create_connection(
                local_addr,
                None,
                Some(cancel_token.child_token()),
            )
            .await?;
            Ok(SipConnection::Udp(connection))
        }
        Some(rsip::transport::Transport::Tcp) => {
            let resolved = resolve_sip_addr(&target).await?;
            let connection =
                TcpConnection::connect(&resolved, Some(cancel_token.child_token())).await?;
            Ok(SipConnection::Tcp(connection))
        }
        Some(rsip::transport::Transport::Tls) => {
            let resolved = resolve_sip_addr(&target).await?;
            let verifier = Arc::new(SkipCertVerifier);
            let connection =
                TlsConnection::connect(&resolved, Some(verifier), Some(cancel_token.child_token())).await?;
            Ok(SipConnection::Tls(connection))
        }
        Some(rsip::transport::Transport::Ws | rsip::transport::Transport::Wss) => {
            let connection =
                WebSocketConnection::connect(&target, Some(cancel_token.child_token())).await?;
            Ok(SipConnection::WebSocket(connection))
        }
        _ => Err(Error::TransportLayerError(
            format!("unsupported transport type: {:?}", target.r#type),
            target.to_owned(),
        )),
    }
}

pub fn get_first_non_loopback_interface() -> rsipstack::Result<IpAddr> {
    for i in get_if_addrs::get_if_addrs()? {
        if !i.is_loopback() {
            match i.addr {
                get_if_addrs::IfAddr::V4(ref addr) => return Ok(std::net::IpAddr::V4(addr.ip)),
                _ => continue,
            }
        }
    }
    Err(Error::Error("No IPV4 interface found".to_string()))
}
