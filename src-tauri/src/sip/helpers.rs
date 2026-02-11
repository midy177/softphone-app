use rsipstack::transport::tcp::TcpConnection;
use rsipstack::transport::tls::TlsConnection;
use rsipstack::transport::udp::UdpConnection;
use rsipstack::transport::websocket::WebSocketConnection;
use rsipstack::transport::{SipAddr, SipConnection};
use rsipstack::Error;
use std::net::{IpAddr, SocketAddr};
use tokio_util::sync::CancellationToken;

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
            let connection =
                TcpConnection::connect(&target, Some(cancel_token.child_token())).await?;
            Ok(SipConnection::Tcp(connection))
        }
        Some(rsip::transport::Transport::Tls) => {
            let connection =
                TlsConnection::connect(&target, None, Some(cancel_token.child_token())).await?;
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
