//! Public networking for nodes and wallets (docs/design/08-network.md).
//!
//! - Transport: iroh QUIC. Direct connections via hole punching / UPnP; falls
//!   back to public relays when both sides are behind strict NATs.
//! - Discovery: free-rides the BitTorrent Mainline DHT. Each node publishes a
//!   pkarr record (signed by its node key) with its current addresses; a client
//!   that knows a node id resolves it from the DHT. No bootstrap server, no DNS.
//! - Protocol `aether/rpc/1`: one JSON-RPC request per bidirectional stream.
//!
//! Clients never trust what they receive here: balances are verified by the
//! light client (finality certificate + state proof).

use anyhow::{anyhow, Context, Result};
use iroh::address_lookup::AddrFilter;
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use iroh_mainline_address_lookup::DhtAddressLookup;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub const ALPN_RPC: &[u8] = b"aether/rpc/1";
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// Deterministic node key for devnet validator `i`. Public knowledge; devnet only.
pub fn devnet_node_secret(i: u64) -> SecretKey {
    let seed: [u8; 32] = Sha256::digest([b"aether-devnet-iroh-node".as_slice(), &i.to_be_bytes()].concat()).into();
    SecretKey::from_bytes(&seed)
}

pub fn devnet_node_id(i: u64) -> EndpointId {
    devnet_node_secret(i).public()
}

/// Bind an endpoint that resolves peers through the Mainline DHT. With a
/// `secret`, it also publishes its own addresses there (direct + relay).
pub async fn bind(secret: Option<SecretKey>, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let publish = secret.is_some();
    let mut dht = DhtAddressLookup::builder().addr_filter(AddrFilter::unfiltered());
    if !publish {
        dht = dht.no_publish();
    }
    // Minimal preset: discovery comes only from the Mainline DHT (no n0 DNS);
    // public relays are kept as the NAT fallback.
    let mut b = Endpoint::builder(presets::Minimal).relay_mode(iroh::RelayMode::Default).alpns(alpns).address_lookup(dht);
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
            let Ok((mut send, mut recv)) = conn.accept_bi().await else { break };
            let handler = self.0.clone();
            tokio::spawn(async move {
                let Ok(bytes) = recv.read_to_end(MAX_MESSAGE).await else { return };
                let resp = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(req) => handler(req).await,
                    Err(e) => serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }),
                };
                let _ = send.write_all(&serde_json::to_vec(&resp).unwrap_or_default()).await;
                let _ = send.finish();
            });
        }
        Ok(())
    }
}

/// Serve JSON-RPC over `aether/rpc/1` on `endpoint`.
pub fn serve_rpc<F, Fut>(endpoint: Endpoint, handler: F) -> Router
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    let h: Handler = Arc::new(move |v| Box::pin(handler(v)));
    Router::builder(endpoint).accept(ALPN_RPC, RpcProtocol(h)).spawn()
}

/// A client talking to any of a set of known nodes, located via the DHT.
pub struct RpcClient {
    endpoint: Endpoint,
    nodes: Vec<EndpointId>,
    current: tokio::sync::Mutex<Option<(EndpointId, Connection)>>,
}

impl RpcClient {
    pub async fn new(nodes: Vec<EndpointId>) -> Result<Self> {
        Ok(RpcClient { endpoint: bind(None, vec![]).await?, nodes, current: tokio::sync::Mutex::new(None) })
    }

    async fn connection(&self) -> Result<(EndpointId, Connection)> {
        let mut cur = self.current.lock().await;
        if let Some((id, c)) = cur.as_ref() {
            if c.close_reason().is_none() {
                return Ok((*id, c.clone()));
            }
        }
        let mut last = anyhow!("no nodes configured");
        for id in &self.nodes {
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
}
