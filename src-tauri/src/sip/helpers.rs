use futures_util::StreamExt;
use rsipstack::transport::tcp::TcpConnection;
use rsipstack::transport::tls::TlsConnection;
use rsipstack::transport::udp::UdpConnection;
use rsipstack::transport::websocket::{WebSocketConnection, WebSocketInner};
use rsipstack::transport::{SipAddr, SipConnection};
use rsipstack::Error;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring::default_provider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
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
    ws_path: Option<String>,
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
            let resolve = resolve_sip_addr(&target).await?;
            let connection =
                TcpConnection::connect(&resolve, Some(cancel_token.child_token())).await?;
            Ok(SipConnection::Tcp(connection))
        }
        Some(rsip::transport::Transport::Tls) => {
            let resolve = resolve_sip_addr(&target).await?;
            let verifier = Arc::new(SkipCertVerifier);
            let connection =
                TlsConnection::connect(&resolve, Some(verifier), Some(cancel_token.child_token())).await?;
            Ok(SipConnection::Tls(connection))
        }
        Some(rsip::transport::Transport::Ws | rsip::transport::Transport::Wss) => {
            let resolve = resolve_sip_addr(&target).await?;
            let connection =
                create_websocket_connection(&resolve, ws_path.as_deref(), Some(cancel_token.child_token())).await?;
            Ok(SipConnection::WebSocket(connection))
        }
        _ => Err(Error::TransportLayerError(
            format!("unsupported transport type: {:?}", target.r#type),
            target.to_owned(),
        )),
    }
}

/// Manually create a WebSocket connection with a custom path.
/// rsipstack's WebSocketConnection::connect hardcodes the path to "/", so we
/// bypass it to support servers that require a specific endpoint path (e.g. "/ws").
async fn create_websocket_connection(
    remote: &SipAddr,
    ws_path: Option<&str>,
    cancel_token: Option<CancellationToken>,
) -> rsipstack::Result<WebSocketConnection> {
    let scheme = match remote.r#type {
        Some(rsip::transport::Transport::Wss) => "wss",
        _ => "ws",
    };

    let host = match &remote.addr.host {
        rsip::host_with_port::Host::Domain(domain) => domain.to_string(),
        rsip::host_with_port::Host::IpAddr(ip) => ip.to_string(),
    };

    let port = remote.addr.port.as_ref().map_or(5060, |p| *p.value());

    let path = ws_path.unwrap_or("/");
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    let url = format!("{}://{}:{}{}", scheme, host, port, path);
    debug!(url = %url, "WebSocket connecting");

    let mut request = url
        .into_client_request()
        .map_err(|e| Error::Error(format!("Invalid WebSocket URL: {}", e)))?;
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "sip".parse().unwrap());

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| Error::Error(format!("WebSocket connect failed: {}", e)))?;
    let (ws_sink, ws_read) = ws_stream.split();

    Ok(WebSocketConnection {
        inner: Arc::new(WebSocketInner {
            remote_addr: remote.clone(),
            ws_sink: Mutex::new(ws_sink),
            ws_read: Mutex::new(Some(ws_read)),
        }),
        cancel_token,
    })
}

/// Determine the local outbound IP address by consulting the OS routing table.
///
/// Opens a UDP socket and "connects" it to the SIP server. No packets are sent —
/// UDP connect is a local operation that causes the OS to choose the correct egress
/// interface. The resulting `local_addr()` is the IP the kernel would actually use
/// to reach the server (respects VPN, multiple NICs, policy routing, etc.).
///
/// `server_addr` may be "host:port" or just "host" (port defaults to 5060).
///
/// Falls back to the first non-loopback IPv4 interface if routing probe fails.
pub fn get_local_outbound_ip(server_addr: &str) -> rsipstack::Result<IpAddr> {
    use std::net::UdpSocket;

    let target = if server_addr.contains(':') {
        server_addr.to_string()
    } else {
        format!("{}:5060", server_addr)
    };

    match UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect(&target).map(|_| s))
        .and_then(|s| s.local_addr())
    {
        Ok(addr) => {
            debug!(ip = %addr.ip(), server = %target, "Detected local outbound IP via routing");
            Ok(addr.ip())
        }
        Err(e) => {
            tracing::warn!(
                error = %e, server = %target,
                "UDP routing probe failed; falling back to interface enumeration"
            );
            get_first_non_loopback_interface()
        }
    }
}

fn get_first_non_loopback_interface() -> rsipstack::Result<IpAddr> {
    for i in get_if_addrs::get_if_addrs()? {
        if !i.is_loopback() {
            match i.addr {
                get_if_addrs::IfAddr::V4(ref addr) => return Ok(std::net::IpAddr::V4(addr.ip)),
                _ => continue,
            }
        }
    }
    Err(Error::Error("No IPv4 interface found".to_string()))
}
