//! Validator links over iroh.
//!
//! Commonware's authenticated p2p speaks TCP to fixed socket addresses. Each
//! validator therefore listens for p2p on loopback only, and reaches every
//! other validator through a local port that forwards over iroh:
//!
//! ```text
//! validator A                                        validator B
//! commonware ─tcp→ 127.0.0.1:<port for B> ─iroh QUIC→ Inbound ─tcp→ 127.0.0.1:<B p2p>
//! ```
//!
//! One QUIC bidirectional stream per TCP connection. iroh picks the path (hole
//! punched direct UDP, or a public relay) and finds B through the Mainline DHT.
//! Nothing here is trusted: Commonware's ed25519 handshake runs end to end
//! inside the stream, so a wrong or malicious forwarder only causes a failed
//! handshake.

use crate::ALPN_P2P;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Copy both directions until either side closes.
async fn pipe<R1, W1, R2, W2>(mut a_r: R1, mut a_w: W1, mut b_r: R2, mut b_w: W2)
where
    R1: AsyncRead + Unpin,
    W1: AsyncWrite + Unpin,
    R2: AsyncRead + Unpin,
    W2: AsyncWrite + Unpin,
{
    let up = async {
        let _ = tokio::io::copy(&mut a_r, &mut b_w).await;
        let _ = b_w.shutdown().await;
    };
    let down = async {
        let _ = tokio::io::copy(&mut b_r, &mut a_w).await;
        let _ = a_w.shutdown().await;
    };
    tokio::join!(up, down);
}

/// Accepts `aether/p2p/1` connections and forwards each stream to the local
/// Commonware p2p listener.
#[derive(Debug, Clone)]
pub struct Inbound {
    pub target: SocketAddr,
}

impl ProtocolHandler for Inbound {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        tracing::debug!(peer = %conn.remote_id().fmt_short(), "p2p link accepted");
        loop {
            let Ok((send, recv)) = conn.accept_bi().await else {
                break;
            };
            let target = self.target;
            tokio::spawn(async move {
                match TcpStream::connect(target).await {
                    Ok(tcp) => {
                        let _ = tcp.set_nodelay(true);
                        let (r, w) = tcp.into_split();
                        pipe(recv, send, r, w).await;
                    }
                    Err(e) => tracing::warn!(?e, %target, "local p2p listener unreachable"),
                }
            });
        }
        Ok(())
    }
}

/// Where the link to one remote validator currently runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkPath {
    Down,
    Direct(SocketAddr),
    Relay,
}

impl std::fmt::Display for LinkPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkPath::Down => f.write_str("down"),
            LinkPath::Direct(a) => write!(f, "direct {a}"),
            LinkPath::Relay => f.write_str("relay"),
        }
    }
}

/// Outbound link to one remote validator: a loopback listener whose
/// connections are carried over a shared iroh connection.
pub struct Outbound {
    endpoint: Endpoint,
    remote: EndpointId,
    conn: Mutex<Option<Connection>>,
}

impl Outbound {
    /// Listen on `local` and forward to `remote`. Runs until the endpoint closes.
    pub async fn spawn(endpoint: Endpoint, remote: EndpointId, local: SocketAddr) -> std::io::Result<Arc<Self>> {
        let listener = TcpListener::bind(local).await?;
        let link = Arc::new(Outbound { endpoint, remote, conn: Mutex::new(None) });
        let l = link.clone();
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    break;
                };
                let l = l.clone();
                tokio::spawn(async move { l.forward(tcp).await });
            }
        });
        Ok(link)
    }

    async fn connection(&self) -> Option<Connection> {
        let mut cur = self.conn.lock().await;
        if let Some(c) = cur.as_ref() {
            if c.close_reason().is_none() {
                return Some(c.clone());
            }
        }
        match tokio::time::timeout(CONNECT_TIMEOUT, self.endpoint.connect(EndpointAddr::from(self.remote), ALPN_P2P)).await {
            Ok(Ok(c)) => {
                tracing::info!(peer = %self.remote.fmt_short(), "p2p link up");
                *cur = Some(c.clone());
                Some(c)
            }
            Ok(Err(e)) => {
                tracing::debug!(peer = %self.remote.fmt_short(), %e, "p2p link connect failed");
                None
            }
            Err(_) => {
                tracing::debug!(peer = %self.remote.fmt_short(), "p2p link connect timed out");
                None
            }
        }
    }

    async fn forward(&self, tcp: TcpStream) {
        let Some(conn) = self.connection().await else {
            return;
        };
        let Ok((send, recv)) = conn.open_bi().await else {
            *self.conn.lock().await = None;
            return;
        };
        let _ = tcp.set_nodelay(true);
        let (r, w) = tcp.into_split();
        pipe(r, w, recv, send).await;
    }

    pub fn remote(&self) -> EndpointId {
        self.remote
    }

    /// The path iroh selected for this link.
    pub async fn path(&self) -> LinkPath {
        let cur = self.conn.lock().await;
        let Some(c) = cur.as_ref().filter(|c| c.close_reason().is_none()) else {
            return LinkPath::Down;
        };
        match c.paths().iter().find(|p| p.is_selected()) {
            Some(p) if p.is_relay() => LinkPath::Relay,
            Some(p) => match p.remote_addr() {
                iroh::TransportAddr::Ip(a) => LinkPath::Direct(*a),
                _ => LinkPath::Relay,
            },
            None => LinkPath::Down,
        }
    }
}
