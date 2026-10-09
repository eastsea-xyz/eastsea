//! Read-only iroh service. Limits cover accepted application bytes on both
//! directions, including error answers; QUIC/relay framing is not included.

use super::{Connection, Handler, Router, RpcGate, RpcProtocol, WalletServers};
use iroh::endpoint::{RecvStream, SendStream, VarInt};
use iroh::protocol::{AcceptError, ProtocolHandler};
use rand::seq::SliceRandom as _;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

pub const ALPN_READ: &[u8] = b"eastsea/read/1";
pub const MAX_READ_REQUEST: usize = 64 * 1024;
pub const MAX_READ_RESPONSE: usize = 16 * 1024 * 1024;
const READ_IO: Duration = Duration::from_secs(10);
const READ_WORK: Duration = Duration::from_secs(30);
const READ_STREAMS: usize = 64;
const READ_STREAMS_PER_PEER: usize = 4;
const READ_BURST: u32 = 16;
const READ_RATE: u32 = 8;

/// A durable reservation made before touching a stream. Unused input capacity
/// is returned after FIN; interrupted reads/writes remain charged conservatively.
#[derive(Clone, Copy, Debug)]
pub struct Reservation {
    pub period: u64,
    pub bytes: usize,
}

/// Implemented by the node's hot-reloaded, persisted operator policy.
pub trait ReadBudget: Send + Sync {
    /// Return up to `maximum` bytes, or refuse disabled/exhausted policy.
    fn reserve(&self, maximum: usize) -> Result<Reservation, String>;
    fn refund(&self, reservation: Reservation, unused: usize) -> Result<(), String>;
}

/// The same budget is attached to legacy public RPC, except configured validator
/// peers whose identity is authenticated by iroh. Followers have no exceptions.
#[derive(Clone)]
pub struct PublicRead {
    handler: Handler,
    pub(crate) budget: Arc<dyn ReadBudget>,
    rpc_exempt: Vec<super::EndpointId>,
}

impl PublicRead {
    pub fn new<F, Fut>(
        handler: F,
        budget: Arc<dyn ReadBudget>,
        rpc_exempt: Vec<super::EndpointId>,
    ) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Value> + Send + 'static,
    {
        Self {
            handler: Arc::new(move |v| Box::pin(handler(v))),
            budget,
            rpc_exempt,
        }
    }

    pub(crate) fn rpc_budget(&self, peer: super::EndpointId) -> Option<Arc<dyn ReadBudget>> {
        (!self.rpc_exempt.contains(&peer)).then(|| self.budget.clone())
    }
}

impl std::fmt::Debug for PublicRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublicRead")
    }
}

pub(crate) async fn read_request(
    recv: &mut RecvStream,
    limit: usize,
    budget: Option<&Arc<dyn ReadBudget>>,
) -> Result<Vec<u8>, String> {
    let Some(budget) = budget else {
        return tokio::time::timeout(READ_IO, recv.read_to_end(limit))
            .await
            .map_err(|_| "request timed out".to_string())?
            .map_err(|e| format!("request: {e}"));
    };
    // One additional byte distinguishes an exact-limit FIN from an oversized
    // request. Reading into slices never consumes more than reserved capacity.
    let reservation = budget.reserve(limit.saturating_add(1))?;
    let mut bytes = Vec::with_capacity(reservation.bytes.min(limit));
    let received = tokio::time::timeout(READ_IO, async {
        let mut buf = [0u8; 4096];
        loop {
            let room = reservation.bytes.saturating_sub(bytes.len());
            if room == 0 {
                return Err("public read request exceeds available byte budget".to_string());
            }
            let size = room.min(buf.len());
            match recv
                .read(&mut buf[..size])
                .await
                .map_err(|e| format!("request: {e}"))?
            {
                None => return Ok(()),
                Some(n) => {
                    bytes.extend_from_slice(&buf[..n]);
                    if bytes.len() > limit {
                        return Err("public read request is too large".to_string());
                    }
                }
            }
        }
    })
    .await;
    // This process knows exactly how much plaintext it read even on timeout.
    budget.refund(reservation, reservation.bytes.saturating_sub(bytes.len()))?;
    received.map_err(|_| "request timed out".to_string())??;
    Ok(bytes)
}

pub(crate) async fn answer_capped(
    send: &mut SendStream,
    resp: &Value,
    budget: Option<&Arc<dyn ReadBudget>>,
) {
    let Ok(bytes) = serde_json::to_vec(resp) else {
        return;
    };
    if bytes.len() > MAX_READ_RESPONSE {
        let _ = send.reset(VarInt::from_u32(1));
        return;
    }
    if let Some(budget) = budget {
        let Ok(reservation) = budget.reserve(bytes.len()) else {
            let _ = send.reset(VarInt::from_u32(1));
            return;
        };
        if reservation.bytes != bytes.len() {
            let _ = budget.refund(reservation, reservation.bytes);
            let _ = send.reset(VarInt::from_u32(1));
            return;
        }
    }
    // Reserve all output before writing. A failed or interrupted write remains
    // charged, so a crash cannot reset an operator's allowance.
    let _ = tokio::time::timeout(READ_IO, async {
        if send.write_all(&bytes).await.is_ok() {
            let _ = send.finish();
        }
    })
    .await;
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message.into()}})
}

#[derive(Clone)]
struct ReadProtocol {
    read: PublicRead,
    gate: Arc<RpcGate>,
    wallets: Arc<WalletServers>,
}

/// Only admitted, live announcements supplement configured network hints.
/// An advertisement is still a dial hint: the browser verifies its chain data.
fn add_read_peer_hints(req: &Value, response: &mut Value, wallets: &WalletServers) {
    if let (Some(requests), Some(answers)) = (req.as_array(), response.as_array_mut()) {
        for (request, answer) in requests.iter().zip(answers.iter_mut()) {
            add_read_peer_hints(request, answer, wallets);
        }
        return;
    }
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if method != "aether_readPeers" && method != "eastsea_readPeers" {
        return;
    }
    let limit = req
        .get("params")
        .and_then(|p| p.get(0))
        .and_then(Value::as_u64)
        .unwrap_or(32)
        .min(32) as usize;
    let Some(hints) = response.get_mut("result").and_then(Value::as_array_mut) else {
        return;
    };
    let mut ids: Vec<String> = hints
        .iter()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect();
    ids.extend(wallets.sample().into_iter().map(|id| id.to_string()));
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids.shuffle(&mut rand::rng());
    ids.truncate(limit);
    *hints = ids.into_iter().map(Value::String).collect();
}

impl std::fmt::Debug for ReadProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadProtocol")
    }
}

impl ProtocolHandler for ReadProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = self.gate.peer(conn.remote_id());
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                break;
            };
            let guard = self.gate.enter(&peer);
            let this = self.clone();
            if guard.is_err() {
                // Drain bounded accepted input under the same byte cap, without
                // parsing or executing it. Serial refusals avoid spawned floods.
                let _ = read_request(&mut recv, MAX_READ_REQUEST, Some(&this.read.budget)).await;
                let _ = recv.stop(VarInt::from_u32(1));
                answer_capped(
                    &mut send,
                    &error(
                        Value::Null,
                        -32002,
                        "public read rate or concurrency limit reached",
                    ),
                    Some(&this.read.budget),
                )
                .await;
                continue;
            }
            tokio::spawn(async move {
                let _guard = guard;
                let bytes = match read_request(&mut recv, MAX_READ_REQUEST, Some(&this.read.budget))
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(message) => {
                        let _ = recv.stop(VarInt::from_u32(1));
                        answer_capped(
                            &mut send,
                            &error(Value::Null, -32002, message),
                            Some(&this.read.budget),
                        )
                        .await;
                        return;
                    }
                };
                let response = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(req) => {
                        let id = req.get("id").cloned().unwrap_or(Value::Null);
                        let peer_request = req.clone();
                        match tokio::time::timeout(READ_WORK, (this.read.handler)(req)).await {
                            Ok(mut response) => {
                                add_read_peer_hints(&peer_request, &mut response, &this.wallets);
                                response
                            }
                            Err(_) => error(id, -32002, "public read work timed out"),
                        }
                    }
                    Err(_) => error(Value::Null, -32700, "invalid JSON-RPC request"),
                };
                answer_capped(&mut send, &response, Some(&this.read.budget)).await;
            });
        }
        Ok(())
    }
}

/// Install the read ALPN beside existing authenticated native RPC/tunnels.
pub fn serve_with_public_read<F, Fut>(
    endpoint: super::Endpoint,
    handler: F,
    p2p_target: Option<std::net::SocketAddr>,
    registered: Option<super::RegisteredCandidate>,
    read: PublicRead,
) -> Router
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    let wallets = Arc::new(WalletServers::new(registered));
    let mut router = Router::builder(endpoint)
        .accept(
            super::ALPN_RPC,
            RpcProtocol {
                handler: Arc::new(move |v| Box::pin(handler(v))),
                gate: Arc::new(RpcGate::new(
                    super::MAX_RPC_STREAMS,
                    super::RPC_STREAMS_PER_PEER,
                    super::RPC_BURST,
                    super::RPC_RATE_PER_SEC,
                )),
                wallets: Some(wallets.clone()),
                public_read: Some(read.clone()),
            },
        )
        .accept(
            ALPN_READ,
            ReadProtocol {
                read,
                wallets,
                gate: Arc::new(RpcGate::new(
                    READ_STREAMS,
                    READ_STREAMS_PER_PEER,
                    READ_BURST,
                    READ_RATE,
                )),
            },
        );
    if let Some(target) = p2p_target {
        router = router.accept(super::ALPN_P2P, super::tunnel::Inbound { target });
        let reshare = std::net::SocketAddr::new(target.ip(), target.port() + 1);
        router = router.accept(
            super::ALPN_RESHARE,
            super::tunnel::Inbound { target: reshare },
        );
    }
    router.spawn()
}
