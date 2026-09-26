//! Public networking for nodes and wallets (docs/design/08-network.md).
//!
//! - Transport: iroh QUIC. Direct connections via hole punching / UPnP; falls
//!   back to public relays when both sides are behind strict NATs.
//! - Discovery: free-rides the BitTorrent Mainline DHT. Each node publishes a
//!   pkarr record (signed by its node key) with its current addresses; a client
//!   that knows a node id resolves it from the DHT. No bootstrap server, no DNS.
//! - Protocol `aether/rpc/1`: one JSON-RPC request per bidirectional stream.
//! - Protocol `aether/p2p/1`: validator consensus traffic. Each bidirectional
//!   stream carries one TCP connection of the Commonware p2p stack (see
//!   [`tunnel`]); Commonware's own ed25519 handshake authenticates end to end.
//! - Published addresses exclude loopback and Tailscale/CGNAT ranges, so peers
//!   reach each other only over public internet paths (or public relays).
//!
//! Clients never trust what they receive here: balances are verified by the
//! light client (finality certificate + state proof).

use anyhow::{anyhow, Context, Result};
use iroh::address_lookup::AddrFilter;
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
pub use iroh::{Endpoint, EndpointId, SecretKey};
use iroh::{EndpointAddr, TransportAddr};
use iroh_mainline_address_lookup::DhtAddressLookup;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub mod paths;
pub mod tunnel;

pub const ALPN_RPC: &[u8] = b"aether/rpc/1";
pub const ALPN_P2P: &[u8] = b"aether/p2p/1";
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// Deterministic node key for devnet validator `i`. Public knowledge; devnet only.
pub fn devnet_node_secret(i: u64) -> SecretKey {
    let seed: [u8; 32] = Sha256::digest([b"aether-devnet-iroh-node".as_slice(), &i.to_be_bytes()].concat()).into();
    SecretKey::from_bytes(&seed)
}

pub fn devnet_node_id(i: u64) -> EndpointId {
    devnet_node_secret(i).public()
}

/// Addresses that are only meaningful inside a private overlay or this host.
pub fn is_overlay_or_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // 100.64.0.0/10 (CGNAT, used by Tailscale), loopback, link-local.
            (o[0] == 100 && (o[1] & 0xc0) == 64) || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            // fd7a:115c:a1e0::/48 (Tailscale ULA), loopback, link-local.
            (s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0) || v6.is_loopback() || (s[0] & 0xffc0) == 0xfe80 || v6.is_unspecified()
        }
    }
}

/// Publish relay and real network addresses only.
pub fn public_addr_filter() -> AddrFilter {
    AddrFilter::new(|addrs| Cow::Owned(addrs.iter().filter(|a| !matches!(a, TransportAddr::Ip(sa) if is_overlay_or_local(sa.ip()))).cloned().collect()))
}

/// Detect a dead peer (e.g. a restarted validator) within seconds, not the
/// default ~30s, so links reconnect quickly. Heartbeats keep healthy idle
/// links open.
const IDLE_TIMEOUT: Duration = Duration::from_secs(6);
const KEEP_ALIVE: Duration = Duration::from_secs(1);

fn transport_config() -> iroh::endpoint::QuicTransportConfig {
    iroh::endpoint::QuicTransportConfig::builder()
        .keep_alive_interval(KEEP_ALIVE)
        .max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("idle timeout fits")))
        .build()
}

/// Bind an endpoint that resolves peers through the Mainline DHT. With a
/// `secret`, it also publishes its own addresses there (direct + relay).
pub async fn bind(secret: Option<SecretKey>, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let publish = secret.is_some();
    let mut dht = DhtAddressLookup::builder().addr_filter(public_addr_filter());
    if !publish {
        dht = dht.no_publish();
    }
    // Minimal preset: discovery comes only from the Mainline DHT (no n0 DNS);
    // public relays are kept as the NAT fallback.
    let mut b = Endpoint::builder(presets::Minimal)
        .relay_mode(iroh::RelayMode::Default)
        .alpns(alpns)
        .addr_filter(public_addr_filter())
        .transport_config(transport_config())
        .path_selector(Arc::new(paths::PublicPathSelector))
        .address_lookup(dht);
    if let Some(s) = secret {
        b = b.secret_key(s);
    }
    b.bind().await.map_err(|e| anyhow!("bind endpoint: {e}"))
}

type Handler = Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Value> + Send>> + Send + Sync>;

#[derive(Clone)]
struct RpcProtocol(Handler);

impl std::fmt::Debug for RpcProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RpcProtocol")
    }
}

impl ProtocolHandler for RpcProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                break;
            };
            let handler = self.0.clone();
            tokio::spawn(async move {
                let Ok(bytes) = recv.read_to_end(MAX_MESSAGE).await else {
                    return;
                };
                let resp = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(req) => handler(req).await,
                    Err(e) => {
                        serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })
                    }
                };
                let _ = send.write_all(&serde_json::to_vec(&resp).unwrap_or_default()).await;
                let _ = send.finish();
            });
        }
        Ok(())
    }
}

/// Serve JSON-RPC over `aether/rpc/1` on `endpoint`; with `p2p_target`, also
/// accept validator tunnels (`aether/p2p/1`) and forward them to that local
/// Commonware p2p listener.
pub fn serve<F, Fut>(endpoint: Endpoint, handler: F, p2p_target: Option<std::net::SocketAddr>) -> Router
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    let h: Handler = Arc::new(move |v| Box::pin(handler(v)));
    let mut r = Router::builder(endpoint).accept(ALPN_RPC, RpcProtocol(h));
    if let Some(target) = p2p_target {
        r = r.accept(ALPN_P2P, tunnel::Inbound { target });
    }
    r.spawn()
}

/// Accept validator links only (no RPC), forwarding to the local p2p listener.
pub fn serve_p2p(endpoint: Endpoint, p2p_target: std::net::SocketAddr) -> Router {
    Router::builder(endpoint).accept(ALPN_P2P, tunnel::Inbound { target: p2p_target }).spawn()
}

/// Serve JSON-RPC only.
pub fn serve_rpc<F, Fut>(endpoint: Endpoint, handler: F) -> Router
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    serve(endpoint, handler, None)
}

/// A client talking to any of a set of known nodes, located via the DHT.
pub struct RpcClient {
    endpoint: Endpoint,
    nodes: Vec<EndpointId>,
    current: tokio::sync::Mutex<Option<(EndpointId, Connection)>>,
    /// Where the next connection attempt starts in `nodes` (moved by `rotate`).
    start: std::sync::atomic::AtomicUsize,
}

impl RpcClient {
    pub async fn new(nodes: Vec<EndpointId>) -> Result<Self> {
        Ok(RpcClient { endpoint: bind(None, vec![]).await?, nodes, current: tokio::sync::Mutex::new(None), start: std::sync::atomic::AtomicUsize::new(0) })
    }

    /// Drop the current node and prefer the next one (e.g. it is answering but
    /// lagging behind, or answering with data that does not verify).
    pub async fn rotate(&self) {
        *self.current.lock().await = None;
        self.start.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    async fn connection(&self) -> Result<(EndpointId, Connection)> {
        let mut cur = self.current.lock().await;
        if let Some((id, c)) = cur.as_ref() {
            if c.close_reason().is_none() {
                return Ok((*id, c.clone()));
            }
        }
        let mut last = anyhow!("no nodes configured");
        let start = self.start.load(std::sync::atomic::Ordering::Relaxed);
        for id in self.nodes.iter().cycle().skip(start % self.nodes.len().max(1)).take(self.nodes.len()) {
            match tokio::time::timeout(Duration::from_secs(20), self.endpoint.connect(EndpointAddr::from(*id), ALPN_RPC)).await {
                Ok(Ok(c)) => {
                    *cur = Some((*id, c.clone()));
                    return Ok((*id, c));
                }
                Ok(Err(e)) => last = anyhow!("connect {}: {e}", id.fmt_short()),
                Err(_) => last = anyhow!("connect {}: timed out", id.fmt_short()),
            }
        }
        Err(last)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let (_, conn) = self.connection().await?;
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let fut = async {
            let (mut send, mut recv) = conn.open_bi().await.context("open stream")?;
            send.write_all(&serde_json::to_vec(&req)?).await?;
            send.finish()?;
            let bytes = recv.read_to_end(MAX_MESSAGE).await?;
            anyhow::Ok(serde_json::from_slice::<Value>(&bytes)?)
        };
        let resp = match tokio::time::timeout(Duration::from_secs(15), fut).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                *self.current.lock().await = None;
                return Err(e);
            }
            Err(_) => {
                *self.current.lock().await = None;
                return Err(anyhow!("rpc timed out"));
            }
        };
        if let Some(err) = resp.get("error") {
            return Err(anyhow!("{}", err["message"].as_str().unwrap_or("rpc error")));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Human-readable description of the current path (for UIs).
    pub async fn describe(&self) -> String {
        let cur = self.current.lock().await;
        match cur.as_ref() {
            Some((id, c)) => {
                let path = match c.paths().iter().find(|p| p.is_selected()) {
                    Some(p) if p.is_relay() => "relay",
                    Some(_) => "direct",
                    None => "connecting",
                };
                format!("node {} · {path}", id.fmt_short())
            }
            None => "not connected".into(),
        }
    }

    /// The selected path's remote address, for diagnostics.
    pub async fn remote_path(&self) -> Option<String> {
        let cur = self.current.lock().await;
        let (_, c) = cur.as_ref()?;
        c.paths().iter().find(|p| p.is_selected()).map(|p| format!("{:?}", p.remote_addr()))
    }
}
