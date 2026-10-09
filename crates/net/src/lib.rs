//! Public networking for nodes and wallets (docs/design/08-network.md).
//!
//! - Transport: iroh QUIC. Direct connections via hole punching / UPnP; falls
//!   back to public relays when both sides are behind strict NATs.
//! - Discovery: free-rides the BitTorrent Mainline DHT. Each node publishes a
//!   pkarr record (signed by its node key) with its current addresses; a client
//!   that knows a node id resolves it from the DHT. No bootstrap server, no DNS.
//! - Protocol `aether/rpc/1`: one JSON-RPC request per bidirectional stream,
//!   bounded by a global and a per-peer concurrency limit plus a per-peer
//!   token bucket (see [`RpcGate`]); over-limit requests get a JSON-RPC error.
//! - Wallet-server discovery (`aether_announceWalletServer` /
//!   `aether_walletServers`) rides the same protocol (see [`WalletServers`]):
//!   follower Macs announce where they serve reads, so phones need not ask
//!   the validators directly. An announcement is signed by a registered
//!   candidate's voting key over the announcing endpoint id (red-team
//!   2026-09-29 §3), and a node lists it only if its own finalized registry
//!   state knows the key.
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
use iroh::endpoint::presets;
pub use iroh::endpoint::Connection;
pub use iroh::protocol::Router;
use iroh::protocol::{AcceptError, ProtocolHandler};
pub use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, TransportAddr};
use iroh_mainline_address_lookup::DhtAddressLookup;
use rand::seq::SliceRandom as _;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod paths;
pub mod public_read;
pub mod contact;
pub mod tunnel;
pub use public_read::{serve_with_public_read, PublicRead, ReadBudget, Reservation, ALPN_READ};

pub const ALPN_RPC: &[u8] = b"aether/rpc/1";
pub const ALPN_P2P: &[u8] = b"aether/p2p/1";
/// Background committee reshare (`aether run`): a validator's node forwards it
/// to the reshare running next to it, so both share one public node id.
pub const ALPN_RESHARE: &[u8] = b"aether/reshare/1";
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// DoS limits on public RPC streams (audit §5): at most this many requests are
/// read and served at once, across all peers.
const MAX_RPC_STREAMS: usize = 256;
/// Concurrent streams one peer (one node id) may run; the rest get an error.
const RPC_STREAMS_PER_PEER: usize = 16;
/// A peer's request rate: a burst of `RPC_BURST`, then `RPC_RATE_PER_SEC` a
/// second. A wallet or a catch-up download stays far under it.
const RPC_BURST: u32 = 64;
const RPC_RATE_PER_SEC: u32 = 32;
/// What a peer refused for concurrency is told to wait: one short request's time.
const BUSY_CONCURRENCY_RETRY: Duration = Duration::from_millis(100);

/// The text every "server busy" answer starts with. Clients of every version
/// match on it (old followers, wallets): it never changes; the retry hint
/// rides after it.
pub const BUSY: &str = "server busy: rpc concurrency or rate limit reached, retry later";

/// The answer to a request over the limits: [`BUSY`], plus how long to wait
/// (`retry_after_ms`, in the message for clients that only read the text and
/// in `data` for those that read the object) — the transport's Retry-After.
fn busy_answer(retry: Duration) -> Value {
    let ms = retry.as_millis() as u64;
    serde_json::json!({
        "jsonrpc": "2.0", "id": null,
        "error": { "code": -32000, "message": format!("{BUSY}; retry_after_ms={ms}"), "data": { "retry_after_ms": ms } }
    })
}

/// The wait a busy answer asks for (`retry_after_ms=` in its text), if any:
/// older servers send none and the caller backs off on its own.
pub fn busy_retry_after(message: &str) -> Option<Duration> {
    if !message.contains(BUSY) {
        return None;
    }
    let (_, rest) = message.split_once("retry_after_ms=")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u64>().ok().map(Duration::from_millis)
}

/// Peers remembered for limiting (an entry is a semaphore and a bucket).
const MAX_RPC_PEERS: usize = 1024;
/// Reading a request or writing an answer may take at most this long, so a
/// peer that stalls its stream cannot squat on its permits. Generous for a
/// 16 MiB message over a relayed path.
const RPC_IO: Duration = Duration::from_secs(30);

/// Follower Macs that announced themselves as wallet servers (capacity review
/// 2026-09-29): the list light clients spread their reads over, so ~1M phones
/// polling every 2 s do not land on the validators. An announcement is the
/// connection it arrives on, so nobody can announce a node they do not run —
/// and a node here proves nothing anyway: every answer a wallet gets is
/// verified against the committee certificate, so a wallet server can only be
/// slow or stale, never lie. Red-team 2026-09-29 §3: the announcement is also
/// signed (see [`WALLET_SERVER_NAMESPACE`]) by a key the node's own finalized
/// registry state lists as a registered candidate, so an unregistered Mac
/// cannot fill the list, and samples spread over distinct operators.
pub struct WalletServers {
    /// The registry check (a key is a registered candidate, run by whom).
    registered: Option<RegisteredCandidate>,
    announced: Mutex<HashMap<EndpointId, Announced>>,
}

/// What a wallet-server signature covers: the voting key's namespace over the
/// announcing endpoint's own id, so a captured signature is worthless on any
/// other endpoint.
pub const WALLET_SERVER_NAMESPACE: &[u8] = b"aether-wallet-server";

/// The registry check behind listing (red-team 2026-09-29 §3): asks the node's
/// finalized registry state whether `key` belongs to a registered candidate,
/// answered with that candidate's operator (what samples spread across).
/// `None` refuses the announcement.
pub type RegisteredCandidate = Arc<dyn Fn(&[u8; 32]) -> Option<[u8; 20]> + Send + Sync>;

/// One listed wallet server: who operates it, and when it last announced.
struct Announced {
    operator: [u8; 20],
    at: Instant,
}

/// How long an announcement counts (followers re-announce every minute).
const WALLET_SERVER_TTL: Duration = Duration::from_secs(15 * 60);
/// Announcers remembered: the bound on a flood of announcements.
const WALLET_SERVERS_KEPT: usize = 4_096;
/// How many one `aether_walletServers` answer carries. Spread over distinct
/// operators first, so the clients of one validator do not all land on the
/// same operator's Macs.
const WALLET_SERVERS_SAMPLED: usize = 64;

impl Default for WalletServers {
    fn default() -> Self {
        WalletServers { registered: None, announced: Mutex::new(HashMap::new()) }
    }
}

impl WalletServers {
    /// With the registry check of a node that knows the finalized registry
    /// (validators); without one, announcements are refused.
    pub fn new(registered: Option<RegisteredCandidate>) -> Self {
        WalletServers { registered, announced: Mutex::new(HashMap::new()) }
    }

    /// Verify and record one announcement: `key` and `sig` are the raw bytes
    /// of a candidate's voting key and its signature over `remote` (the
    /// connection the announcement arrived on) under
    /// [`WALLET_SERVER_NAMESPACE`]; the registry check must list the key as a
    /// registered candidate. Refused announcements are an `Err` for the
    /// follower's log.
    pub fn announce(&self, remote: EndpointId, key: &[u8], sig: &[u8]) -> Result<(), String> {
        use commonware_codec::DecodeExt;
        use commonware_cryptography::Verifier;
        let key: [u8; 32] = key.try_into().map_err(|_| "validator key must be 32 bytes".to_string())?;
        let sig = commonware_cryptography::ed25519::Signature::decode(sig).map_err(|_| "signature must be 64 bytes".to_string())?;
        let pk = commonware_cryptography::ed25519::PublicKey::decode(key.as_slice()).map_err(|_| "validator key is not an ed25519 key".to_string())?;
        if !pk.verify(WALLET_SERVER_NAMESPACE, remote.as_bytes(), &sig) {
            return Err("the signature is not the key's over this endpoint id".into());
        }
        let operator = self
            .registered
            .as_ref()
            .ok_or_else(|| "this node does not list wallet servers".to_string())?
            (&key)
            .ok_or_else(|| "not a registered candidate on this chain".to_string())?;
        self.announce_at(remote, operator, Instant::now());
        Ok(())
    }

    fn announce_at(&self, id: EndpointId, operator: [u8; 20], now: Instant) {
        let mut g = self.announced.lock().expect("wallet servers lock");
        if g.len() >= WALLET_SERVERS_KEPT && !g.contains_key(&id) {
            // Full: forget the one that announced longest ago.
            if let Some(oldest) = g.iter().min_by_key(|(_, a)| a.at).map(|(k, _)| *k) {
                g.remove(&oldest);
            }
        }
        g.insert(id, Announced { operator, at: now });
    }

    /// A sample of the ids that announced recently, spread over distinct
    /// operators first (one Mac per operator before any operator's second).
    pub fn sample(&self) -> Vec<EndpointId> {
        let mut live = self.live_at(Instant::now());
        // Buckets in a random order, each holding that operator's Macs.
        let mut buckets: HashMap<[u8; 20], Vec<EndpointId>> = HashMap::new();
        for (id, operator) in live.drain(..) {
            buckets.entry(operator).or_default().push(id);
        }
        let mut buckets: Vec<_> = buckets.into_values().collect();
        buckets.shuffle(&mut rand::rng());
        let mut out = Vec::with_capacity(WALLET_SERVERS_SAMPLED);
        while out.len() < WALLET_SERVERS_SAMPLED {
            let mut took = false;
            for bucket in buckets.iter_mut() {
                if let Some(id) = bucket.pop() {
                    out.push(id);
                    took = true;
                    if out.len() == WALLET_SERVERS_SAMPLED {
                        break;
                    }
                }
            }
            if !took {
                break;
            }
        }
        out
    }

    fn live_at(&self, now: Instant) -> Vec<(EndpointId, [u8; 20])> {
        let g = self.announced.lock().expect("wallet servers lock");
        let mut ids: Vec<_> = g
            .iter()
            .filter(|(_, a)| now.saturating_duration_since(a.at) < WALLET_SERVER_TTL)
            .map(|(k, a)| (*k, a.operator))
            .collect();
        ids.shuffle(&mut rand::rng());
        ids
    }
}

/// Write `resp` and close our side, or give up when the peer will not read.
async fn answer(send: &mut iroh::endpoint::SendStream, resp: &Value) {
    let bytes = serde_json::to_vec(resp).unwrap_or_default();
    let _ = tokio::time::timeout(RPC_IO, async {
        let _ = send.write_all(&bytes).await;
        let _ = send.finish();
    })
    .await;
}

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

/// Public advertisement policy is stricter than native path selection: a NAT's
/// LAN interface may carry a public connection but is never a DHT dial hint.
pub fn is_public_ip(ip: IpAddr) -> bool {
    if is_overlay_or_local(ip) { return false; }
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private() || v4.is_multicast() || v4.is_broadcast() || v4.is_documentation()
                || o[0] == 0 || o[0] >= 240 || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            !(v6.is_multicast() || (s[0] & 0xfe00) == 0xfc00 || (s[0] == 0x2001 && s[1] == 0xdb8)
                || v6.to_ipv4_mapped().is_some_and(|ip| !is_public_ip(IpAddr::V4(ip))))
        }
    }
}

/// Publish relay and real network addresses only.
pub fn public_addr_filter() -> AddrFilter {
    AddrFilter::new(|addrs| Cow::Owned(addrs.iter().filter(|a| !matches!(a, TransportAddr::Ip(sa) if !is_public_ip(sa.ip()))).cloned().collect()))
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
    let relay_mode = match std::env::var("AETHER_IROH_RELAY_URL") {
        Ok(url) => {
            let url = url.parse::<iroh::RelayUrl>().map_err(|e| anyhow!("AETHER_IROH_RELAY_URL: {e}"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
                return Err(anyhow!("AETHER_IROH_RELAY_URL must be an HTTP(S) relay URL without credentials"));
            }
            iroh::RelayMode::custom([url])
        }
        Err(std::env::VarError::NotPresent) => iroh::RelayMode::Default,
        Err(e) => return Err(anyhow!("AETHER_IROH_RELAY_URL: {e}")),
    };
    let mut b = Endpoint::builder(presets::Minimal)
        .relay_mode(relay_mode)
        .alpns(alpns)
        .addr_filter(public_addr_filter())
        .transport_config(transport_config())
        .path_selector(Arc::new(paths::PublicPathSelector));
    if std::env::var("AETHER_IROH_NO_DHT").as_deref() != Ok("1") {
        b = b.address_lookup(dht);
    }
    if let Some(s) = secret {
        b = b.secret_key(s);
    }
    b.bind().await.map_err(|e| anyhow!("bind endpoint: {e}"))
}

type Handler = Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Value> + Send>> + Send + Sync>;

/// One peer's request budget: `burst` requests at once, then `per_sec` a
/// second. Integral refill: whole tokens for the elapsed whole milliseconds.
struct TokenBucket {
    tokens: u32,
    burst: u32,
    per_sec: u32,
    last: Instant,
}

impl TokenBucket {
    fn new(burst: u32, per_sec: u32) -> Self {
        TokenBucket { tokens: burst, burst, per_sec, last: Instant::now() }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed_ms = now.saturating_duration_since(self.last).as_millis() as u64;
        let refill = (elapsed_ms.saturating_mul(self.per_sec as u64) / 1000).min(self.burst as u64);
        if refill > 0 {
            self.tokens = self.tokens.saturating_add(refill as u32).min(self.burst);
            self.last = now;
        }
    }

    fn is_full(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens == self.burst
    }

    /// Take one token if the bucket holds one after refilling for `now - last`.
    fn take(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

/// What one peer (one node id) is limited by: a concurrency semaphore and a
/// token bucket. Live owners and in-flight permits prevent eviction.
struct PeerLimit {
    inflight: Arc<tokio::sync::Semaphore>,
    bucket: Mutex<TokenBucket>,
}

/// The limits every `aether/rpc/1` connection on an endpoint shares: a global
/// semaphore, and a semaphore and token bucket per peer. One peer cannot
/// occupy every worker, and a flood of requests is answered with an error
/// instead of being read and served (audit §5).
struct RpcGate {
    global: Arc<tokio::sync::Semaphore>,
    peers: Mutex<HashMap<EndpointId, Arc<PeerLimit>>>,
    /// New identities get no work permits while every retained entry is live
    /// or still owes tokens. Sharing this refusal avoids unbounded map growth.
    denied: Arc<PeerLimit>,
    per_peer: usize,
    burst: u32,
    per_sec: u32,
}

/// Both permits of one in-flight request; released when dropped.
struct RpcGuard {
    _global: tokio::sync::OwnedSemaphorePermit,
    _peer: tokio::sync::OwnedSemaphorePermit,
}

impl RpcGate {
    fn new(global: usize, per_peer: usize, burst: u32, per_sec: u32) -> Self {
        RpcGate {
            global: Arc::new(tokio::sync::Semaphore::new(global)),
            peers: Mutex::new(HashMap::new()),
            denied: Arc::new(PeerLimit {
                inflight: Arc::new(tokio::sync::Semaphore::new(0)),
                bucket: Mutex::new(TokenBucket::new(0, 0)),
            }),
            per_peer,
            burst,
            per_sec,
        }
    }

    /// One limiter for every live identity, including work outliving its
    /// connection. Eviction is safe only after owners/permits are gone and its
    /// rate bucket is full: reconnecting then gains no extra burst. If none is
    /// safe to forget, the returned refusal limiter admits no request.
    fn peer(&self, id: EndpointId) -> Arc<PeerLimit> {
        let mut peers = self.peers.lock().expect("rpc peer map");
        if let Some(p) = peers.get(&id) {
            return p.clone();
        }
        if peers.len() >= MAX_RPC_PEERS {
            let now = Instant::now();
            let forgotten = peers.iter().find_map(|(id, peer)| {
                (Arc::strong_count(peer) == 1
                    && peer.inflight.available_permits() == self.per_peer
                    && peer.bucket.lock().expect("rpc token bucket").is_full(now))
                    .then_some(*id)
            });
            if let Some(forgotten) = forgotten {
                peers.remove(&forgotten);
            } else {
                return self.denied.clone();
            }
        }
        let p = Arc::new(PeerLimit {
            inflight: Arc::new(tokio::sync::Semaphore::new(self.per_peer)),
            bucket: Mutex::new(TokenBucket::new(self.burst, self.per_sec)),
        });
        peers.insert(id, p.clone());
        p
    }

    /// Admit one request of `peer`, or `Err(retry_after)` when a limit is hit
    /// (the global or the peer's concurrency, or the peer's rate): how long
    /// the peer should wait before its next request has a chance. Permits are
    /// held until the returned guard is dropped.
    fn enter(&self, peer: &PeerLimit) -> std::result::Result<RpcGuard, Duration> {
        let global = self.global.clone().try_acquire_owned().map_err(|_| BUSY_CONCURRENCY_RETRY)?;
        let inflight = peer.inflight.clone().try_acquire_owned().map_err(|_| BUSY_CONCURRENCY_RETRY)?;
        if !peer.bucket.lock().expect("rpc token bucket").take(Instant::now()) {
            // One token comes back every 1/per_sec seconds.
            return Err(Duration::from_millis(1000u64.div_ceil(u64::from(self.per_sec.max(1))).max(1)));
        }
        Ok(RpcGuard { _global: global, _peer: inflight })
    }
}

#[derive(Clone)]
struct RpcProtocol {
    handler: Handler,
    gate: Arc<RpcGate>,
    /// Wallet-server discovery, when this endpoint is a place followers
    /// announce to (any node serving `aether/rpc/1`): with the registry
    /// check, announcements are verified and listed; without it, refused.
    wallets: Option<Arc<WalletServers>>,
    /// Unregistered public clients share the read service's operator cap.
    public_read: Option<PublicRead>,
}

impl std::fmt::Debug for RpcProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RpcProtocol")
    }
}

/// A successful JSON-RPC answer to `req`.
fn rpc_ok(req: &Value, result: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": req.get("id").cloned().unwrap_or(Value::Null), "result": result })
}

impl ProtocolHandler for RpcProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        let peer = self.gate.peer(remote);
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                break;
            };
            let this = self.clone();
            let peer = peer.clone();
            let budget = this.public_read.as_ref().and_then(|read| read.rpc_budget(remote));
            tokio::spawn(async move {
                // Limits first, before reading anything: an over-limit request
                // costs one small error answer, not a 16 MiB read and a handler.
                let _guard = match this.gate.enter(&peer) {
                    Ok(guard) => guard,
                    Err(retry) => {
                        let busy = busy_answer(retry);
                        if let Some(budget) = &budget {
                            let _ = public_read::read_request(&mut recv, MAX_MESSAGE, Some(budget)).await;
                            let _ = recv.stop(iroh::endpoint::VarInt::from_u32(1));
                            public_read::answer_capped(&mut send, &busy, Some(budget)).await;
                        } else { answer(&mut send, &busy).await; }
                        return;
                    }
                };
                // A peer that stalls mid-request (or never reads its answer)
                // must not hold its permits forever: the connection's idle
                // timeout does not fire while its other streams carry traffic.
                let bytes = if let Some(budget) = &budget {
                    match public_read::read_request(&mut recv, MAX_MESSAGE, Some(budget)).await {
                        Ok(bytes) => bytes,
                        Err(_) => { let _ = recv.stop(iroh::endpoint::VarInt::from_u32(1)); return; }
                    }
                } else {
                    match tokio::time::timeout(RPC_IO, recv.read_to_end(MAX_MESSAGE)).await {
                        Ok(Ok(bytes)) => bytes,
                        _ => return,
                    }
                };
                let resp = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(req) => match (req["method"].as_str(), this.wallets.as_ref()) {
                        // Wallet-server discovery rides the transport, inside
                        // the DoS limits, so an announcement is bound to the
                        // connection it arrived on (see [`WalletServers`]):
                        // `[validatorKey (hex), signature (hex)]`, the key's
                        // signature over that endpoint id.
                        (Some("aether_announceWalletServer"), Some(w)) => {
                            let param = |i: usize| {
                                req.get("params").and_then(|p| p.get(i)).and_then(Value::as_str).map(str::to_string)
                            };
                            let parsed = match (param(0), param(1)) {
                                (Some(k), Some(s)) => match (hex::decode(k.trim_start_matches("0x")), hex::decode(s.trim_start_matches("0x"))) {
                                    (Ok(k), Ok(s)) => w.announce(remote, &k, &s),
                                    _ => Err("validator key and signature must be hex".to_string()),
                                },
                                _ => Err("params: [validatorKey, signature]".to_string()),
                            };
                            match parsed {
                                Ok(()) => rpc_ok(&req, serde_json::json!({ "ok": true })),
                                Err(e) => serde_json::json!({
                                    "jsonrpc": "2.0", "id": req.get("id").cloned().unwrap_or(Value::Null),
                                    "error": { "code": -32000, "message": format!("announcement refused: {e}") }
                                }),
                            }
                        }
                        (Some("aether_walletServers"), Some(w)) => rpc_ok(
                            &req,
                            serde_json::json!(w
                                .sample()
                                .iter()
                                .map(|i| i.to_string())
                                .collect::<Vec<_>>()),
                        ),
                        _ => (this.handler)(req).await,
                    },
                    Err(e) => {
                        serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })
                    }
                };
                if budget.is_some() { public_read::answer_capped(&mut send, &resp, budget.as_ref()).await; }
                else { answer(&mut send, &resp).await; }
            });
        }
        Ok(())
    }
}

/// Serve JSON-RPC over `aether/rpc/1` on `endpoint`; with `p2p_target`, also
/// accept validator tunnels (`aether/p2p/1`) and forward them to that local
/// Commonware p2p listener. `registered` is the finalized-registry check
/// wallet-server announcements are listed under (validators pass one;
/// `None` refuses announcements).
pub fn serve<F, Fut>(
    endpoint: Endpoint,
    handler: F,
    p2p_target: Option<std::net::SocketAddr>,
    registered: Option<RegisteredCandidate>,
) -> Router
where
    F: Fn(Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    let h: Handler = Arc::new(move |v| Box::pin(handler(v)));
    let gate = Arc::new(RpcGate::new(MAX_RPC_STREAMS, RPC_STREAMS_PER_PEER, RPC_BURST, RPC_RATE_PER_SEC));
    let mut r = Router::builder(endpoint).accept(
        ALPN_RPC,
        RpcProtocol {
            handler: h,
            gate,
            wallets: Some(Arc::new(WalletServers::new(registered))),
            public_read: None,
        },
    );
    if let Some(target) = p2p_target {
        r = r.accept(ALPN_P2P, tunnel::Inbound { target });
        // The background reshare listens on the next port.
        let reshare = std::net::SocketAddr::new(target.ip(), target.port() + 1);
        r = r.accept(ALPN_RESHARE, tunnel::Inbound { target: reshare });
    }
    r.spawn()
}

/// Accept reshare links only, forwarding to the local reshare listener.
pub fn serve_reshare(endpoint: Endpoint, target: std::net::SocketAddr) -> Router {
    Router::builder(endpoint).accept(ALPN_RESHARE, tunnel::Inbound { target }).spawn()
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
    serve(endpoint, handler, None, None)
}

/// How one JSON-RPC roundtrip failed.
#[derive(Debug)]
pub enum RpcError {
    /// The server answered, but refused (a JSON-RPC error object).
    Server { code: i64, message: String },
    /// The request never completed (connect, stream, timeout, framing).
    Transport(anyhow::Error),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Server { message, .. } => f.write_str(message),
            RpcError::Transport(e) => write!(f, "{e:#}"),
        }
    }
}

/// How long one request may take once connected (both for `RpcClient` and for
/// the wallet read spread, which shares this framing).
const RPC_CALL: Duration = Duration::from_secs(15);

/// Connect to `addr` for one JSON-RPC conversation, within `within`.
pub async fn connect_rpc(
    endpoint: &Endpoint,
    addr: &EndpointAddr,
    within: Duration,
) -> Result<Connection> {
    match tokio::time::timeout(within, endpoint.connect(addr.clone(), ALPN_RPC)).await {
        Ok(c) => c.with_context(|| format!("connect {}: ", addr.id.fmt_short())),
        Err(_) => Err(anyhow!("connect {}: timed out", addr.id.fmt_short())),
    }
}

/// One JSON-RPC request over one bidirectional stream of `conn`, ending in
/// the answer (a JSON-RPC error object included, as `RpcError::Server`).
pub async fn rpc_call(
    conn: &Connection,
    method: &str,
    params: Value,
) -> std::result::Result<Value, RpcError> {
    let req = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let fut = async {
        let (mut send, mut recv) = conn.open_bi().await.context("open stream")?;
        send.write_all(&serde_json::to_vec(&req)?).await?;
        // A server over its limits stops reading and answers with an
        // error; that answer is still worth reading, so a failed finish
        // (the stream was reset) is not a failure of the call.
        let _ = send.finish();
        let bytes = recv.read_to_end(MAX_MESSAGE).await?;
        anyhow::Ok(serde_json::from_slice::<Value>(&bytes)?)
    };
    let resp = match tokio::time::timeout(RPC_CALL, fut).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(RpcError::Transport(e)),
        Err(_) => return Err(RpcError::Transport(anyhow!("rpc timed out"))),
    };
    match resp.get("error") {
        Some(e) => Err(RpcError::Server {
            code: e["code"].as_i64().unwrap_or_default(),
            message: e["message"].as_str().unwrap_or("rpc error").to_string(),
        }),
        None => Ok(resp.get("result").cloned().unwrap_or(Value::Null)),
    }
}

/// How long one connect attempt to a known node may take.
const CONNECT_ATTEMPT: Duration = Duration::from_secs(20);
/// How long one whole scan for a live node may take (the incident of
/// 2026-10-05: after a reboot every validator had moved, each attempt burned
/// its full 20 s, and four of them under the client's lock made every queued
/// read wait 80 s — repeatedly, because nothing bounded the scan as a whole).
/// A scan that exhausts the budget fails; the next scan starts where this one
/// stopped, so one dead node is not retried forever while the rest go untried.
const CONNECT_SCAN: Duration = Duration::from_secs(20);

/// A client talking to any of a set of known nodes, located via the DHT.
pub struct RpcClient {
    endpoint: Endpoint,
    nodes: Vec<EndpointAddr>,
    current: tokio::sync::Mutex<Option<(EndpointId, Connection)>>,
    /// Where the next connection attempt starts in `nodes` (moved by `rotate`).
    start: std::sync::atomic::AtomicUsize,
    /// One attempt's and one scan's budget (tests shrink both).
    attempt: Duration,
    scan: Duration,
    /// Connections by index into `nodes`, for [`RpcClient::call_at`]: a
    /// follower catching up spreads its requests over every node, each on a
    /// connection of its own, instead of hammering the current one.
    pool: std::sync::Mutex<HashMap<usize, Connection>>,
    /// Nodes whose last connect failed, and when: skipped for
    /// [`DOWN_FOR`] so a dead node costs one connect timeout, not one per request.
    down: std::sync::Mutex<HashMap<usize, Instant>>,
    /// One connect at a time per node for `call_at`: concurrent callers wait
    /// for it and share its connection (or its failure) instead of each
    /// dialing the same node.
    connecting: Vec<tokio::sync::Mutex<()>>,
}

/// How long a node that failed to connect is skipped by [`RpcClient::call_at`].
const DOWN_FOR: Duration = Duration::from_secs(120);


impl RpcClient {
    pub async fn new(nodes: Vec<EndpointId>) -> Result<Self> {
        Ok(Self::with_endpoint(bind(None, vec![]).await?, nodes))
    }

    /// A client on an endpoint someone else owns, asking `nodes` by id: a
    /// follower's public endpoint, so the validators it asks (and announces
    /// its wallet serving to) see its published node id.
    pub fn with_endpoint(endpoint: Endpoint, nodes: Vec<EndpointId>) -> Self {
        let connecting = nodes.iter().map(|_| tokio::sync::Mutex::new(())).collect();
        RpcClient {
            endpoint,
            nodes: nodes.into_iter().map(EndpointAddr::from).collect(),
            current: tokio::sync::Mutex::new(None),
            start: std::sync::atomic::AtomicUsize::new(0),
            attempt: CONNECT_ATTEMPT,
            scan: CONNECT_SCAN,
            pool: Default::default(),
            down: Default::default(),
            connecting,
        }
    }

    /// A client on its own endpoint, at explicit addresses (tests, previews).
    pub async fn with_addrs(addrs: Vec<EndpointAddr>) -> Result<Self> {
        let connecting = addrs.iter().map(|_| tokio::sync::Mutex::new(())).collect();
        Ok(RpcClient {
            endpoint: bind(None, vec![]).await?,
            nodes: addrs,
            current: tokio::sync::Mutex::new(None),
            start: std::sync::atomic::AtomicUsize::new(0),
            attempt: CONNECT_ATTEMPT,
            scan: CONNECT_SCAN,
            pool: Default::default(),
            down: Default::default(),
            connecting,
        })
    }

    /// Shrink the connect budgets (tests only; production uses the defaults).
    #[cfg(test)]
    fn with_connect_timeouts(mut self, attempt: Duration, scan: Duration) -> Self {
        self.attempt = attempt;
        self.scan = scan;
        self
    }

    /// The endpoint all calls go out on (shared with the wallet read spread).
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// This client's node id: what the servers it asks see.
    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Drop the current node and prefer the next one (e.g. it is answering but
    /// lagging behind, or answering with data that does not verify).
    pub async fn rotate(&self) {
        *self.current.lock().await = None;
        self.start.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Drop the current node and prefer node `i` of `nodes` next. A recovery
    /// path that already knows which peer had the answer (pre-audit 7b
    /// PA7B-06) moves straight to it instead of one step at a time.
    pub async fn rotate_to(&self, i: usize) {
        *self.current.lock().await = None;
        if !self.nodes.is_empty() {
            self.start.store(i % self.nodes.len(), std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Ask every node EXCEPT the current one, each on its own connection
    /// attempt bounded by `attempt` (pre-audit 7b PA7B-06): corroborating a
    /// claim across peers needs the peers the client is not already talking
    /// to. Answers come back as (index into `nodes`, value); a node that
    /// fails to connect or answer is simply absent — the caller keeps what
    /// it already has. The current node is not touched, so a corroborated
    /// answer never disturbs a connection other reads are using.
    pub async fn ask_others(&self, method: &str, params: Value) -> Vec<(usize, Value)> {
        let skip = self
            .current
            .lock()
            .await
            .as_ref()
            .and_then(|(id, _)| self.nodes.iter().position(|a| a.id == *id));
        let mut out = Vec::new();
        for (i, addr) in self.nodes.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            let connected = match tokio::time::timeout(self.attempt, self.endpoint.connect(addr.clone(), ALPN_RPC)).await {
                Ok(Ok(c)) => c,
                _ => continue,
            };
            if let Ok(v) = rpc_call(&connected, method, params.clone()).await {
                out.push((i, v));
            }
        }
        out
    }

    /// A live connection, scanning `nodes` when the cached one is gone. The
    /// whole scan — not just each attempt — is bounded by `scan`, and every
    /// node it got through moves `start` past itself, so a later scan begins
    /// at the next candidate instead of re-dialing the same dead one (the
    /// incident of 2026-10-05).
    async fn connection(&self) -> Result<(EndpointId, Connection)> {
        let mut cur = self.current.lock().await;
        if let Some((id, c)) = cur.as_ref() {
            if c.close_reason().is_none() {
                return Ok((*id, c.clone()));
            }
        }
        let mut last = anyhow!("no nodes configured");
        let n = self.nodes.len();
        let start = self.start.load(std::sync::atomic::Ordering::Relaxed);
        let deadline = tokio::time::Instant::now() + self.scan;
        let mut tried = 0;
        for i in 0..n {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                last = anyhow!("connect: scan budget of {} ms exhausted", self.scan.as_millis());
                break;
            }
            let addr = &self.nodes[(start + i) % n];
            tried = i + 1;
            match tokio::time::timeout(self.attempt.min(left), self.endpoint.connect(addr.clone(), ALPN_RPC)).await {
                Ok(Ok(c)) => {
                    self.start.store((start + i) % n, std::sync::atomic::Ordering::Relaxed);
                    *cur = Some((addr.id, c.clone()));
                    return Ok((addr.id, c));
                }
                Ok(Err(e)) => last = anyhow!("connect {}: {e}", addr.id.fmt_short()),
                Err(_) => last = anyhow!("connect {}: timed out", addr.id.fmt_short()),
            }
        }
        if tried > 0 {
            self.start.store((start + tried) % n, std::sync::atomic::Ordering::Relaxed);
        }
        Err(last)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let (_, conn) = self.connection().await?;
        match rpc_call(&conn, method, params).await {
            Ok(v) => Ok(v),
            Err(RpcError::Server { message, .. }) => Err(anyhow!(message)),
            Err(RpcError::Transport(e)) => {
                *self.current.lock().await = None;
                Err(e)
            }
        }
    }

    /// `call`, also naming the node that answered: a wallet remembers which
    /// validator admitted its transaction, so it can ask that node — the only
    /// one sure to know it while pending — what became of it (B5 review
    /// round 2, finding 2).
    pub async fn call_tracked(&self, method: &str, params: Value) -> Result<(EndpointId, Value)> {
        let (id, conn) = self.connection().await?;
        match rpc_call(&conn, method, params).await {
            Ok(v) => Ok((id, v)),
            Err(RpcError::Server { message, .. }) => Err(anyhow!(message)),
            Err(RpcError::Transport(e)) => {
                *self.current.lock().await = None;
                Err(e)
            }
        }
    }

    /// Ask one known node by id: on the current connection when it is that
    /// node, else on a connection of its own (one attempt, bounded by the
    /// connect budget) that leaves the current one untouched. An id outside
    /// `nodes` is refused: this never dials an arbitrary peer.
    pub async fn call_node(&self, id: EndpointId, method: &str, params: Value) -> Result<Value> {
        let current = self
            .current
            .lock()
            .await
            .as_ref()
            .filter(|(c, conn)| *c == id && conn.close_reason().is_none())
            .map(|(_, conn)| conn.clone());
        let conn = match current {
            Some(c) => c,
            None => {
                let addr = self
                    .nodes
                    .iter()
                    .find(|a| a.id == id)
                    .ok_or_else(|| anyhow!("node {} is not one of this client's nodes", id.fmt_short()))?;
                tokio::time::timeout(self.attempt, self.endpoint.connect(addr.clone(), ALPN_RPC))
                    .await
                    .map_err(|_| anyhow!("connect {}: timed out", id.fmt_short()))?
                    .map_err(|e| anyhow!("connect {}: {e}", id.fmt_short()))?
            }
        };
        match rpc_call(&conn, method, params).await {
            Ok(v) => Ok(v),
            Err(RpcError::Server { message, .. }) => Err(anyhow!(message)),
            Err(RpcError::Transport(e)) => Err(e),
        }
    }

    /// Node `i`'s id, as text (a stable key for what a caller learns about it).
    pub fn node_key(&self, i: usize) -> Option<String> {
        self.nodes.get(i).map(|a| a.id.to_string())
    }

    /// How many nodes this client knows.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node the next connection scan starts at (where `call` is talking).
    pub fn preferred(&self) -> usize {
        self.start.load(std::sync::atomic::Ordering::Relaxed) % self.nodes.len().max(1)
    }

    /// Ask node `i` of `nodes`, on a pooled connection of its own (never the
    /// shared current one, which other reads use). A node that failed to
    /// connect within the last [`DOWN_FOR`] is refused at once; a transport
    /// failure drops its pooled connection. Server refusals ("server busy")
    /// leave the connection in place: the node is fine, only full.
    pub async fn call_at(&self, i: usize, method: &str, params: Value) -> Result<Value> {
        let conn = self.pooled_connection(i).await?;
        match rpc_call(&conn, method, params).await {
            Ok(v) => Ok(v),
            Err(RpcError::Server { message, .. }) => Err(anyhow!(message)),
            Err(RpcError::Transport(e)) => {
                self.drop_failed_connection(i, &conn);
                Err(e)
            }
        }
    }

    fn drop_failed_connection(&self, i: usize, failed: &Connection) {
        let mut pool = self.pool.lock().expect("rpc pool");
        // Another call may have replaced this connection while the failing
        // stream was still pending. Its delayed error must not evict that one.
        if pool.get(&i).is_some_and(|current| current.stable_id() == failed.stable_id()) {
            pool.remove(&i);
        }
    }

    /// Node `i`'s pooled connection, connecting once when there is none:
    /// concurrent callers wait for that one attempt and share its outcome.
    async fn pooled_connection(&self, i: usize) -> Result<Connection> {
        let addr = self.nodes.get(i).ok_or_else(|| anyhow!("no node {i}"))?.clone();
        let pooled = || self.pool.lock().expect("rpc pool").get(&i).filter(|c| c.close_reason().is_none()).cloned();
        if let Some(c) = pooled() {
            return Ok(c);
        }
        let _one = self.connecting[i].lock().await;
        if let Some(c) = pooled() {
            return Ok(c);
        }
        if self.down.lock().expect("rpc down").get(&i).is_some_and(|at| at.elapsed() < DOWN_FOR) {
            return Err(anyhow!("node {} is unreachable; skipped for now", addr.id.fmt_short()));
        }
        match tokio::time::timeout(self.attempt, self.endpoint.connect(addr.clone(), ALPN_RPC)).await {
            Ok(Ok(c)) => {
                self.down.lock().expect("rpc down").remove(&i);
                self.pool.lock().expect("rpc pool").insert(i, c.clone());
                Ok(c)
            }
            other => {
                self.down.lock().expect("rpc down").insert(i, Instant::now());
                Err(match other {
                    Ok(Err(e)) => anyhow!("connect {}: {e}", addr.id.fmt_short()),
                    _ => anyhow!("connect {}: timed out", addr.id.fmt_short()),
                })
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};

    #[test]
    fn a_busy_answer_carries_a_retry_hint_old_text_first() {
        let a = busy_answer(Duration::from_millis(31));
        let m = a["error"]["message"].as_str().unwrap();
        assert!(m.starts_with(BUSY), "old clients match the unchanged text: {m}");
        assert_eq!(busy_retry_after(m), Some(Duration::from_millis(31)));
        assert_eq!(busy_retry_after(BUSY), None, "an old server's answer has no hint");
        assert_eq!(busy_retry_after("retry_after_ms=5"), None, "only a busy answer is read");
        // The rate limit's hint is one token's time.
        let gate = RpcGate::new(8, 8, 1, 32);
        let peer = gate.peer(iroh::SecretKey::from_bytes(&[3; 32]).public());
        let _held = gate.enter(&peer).ok().expect("the burst admits one");
        assert_eq!(gate.enter(&peer).err(), Some(Duration::from_millis(32)), "rounded up: 31 ms refills nothing at 32/s");
    }

    #[test]
    fn a_token_bucket_bursts_then_refills_by_elapsed_time() {
        let t = Instant::now();
        let mut b = TokenBucket::new(2, 500); // 2 at once, then one per 2 ms
        assert!(b.take(t) && b.take(t), "the burst is allowed");
        assert!(!b.take(t), "and nothing more in the same instant");
        assert!(!b.take(t + Duration::from_millis(1)), "half a token is not a token");
        // The bucket's own clock started nanoseconds after `t`, so the elapsed
        // time truncates a whole millisecond early: 5 ms is safely 2+ tokens.
        assert!(b.take(t + Duration::from_millis(5)), "refilled by the elapsed time");
        assert!(b.take(t + Duration::from_millis(5)), "the refill covered the whole burst");
        assert!(!b.take(t + Duration::from_millis(5)), "and it is spent again");
        assert!(!b.take(t + Duration::from_millis(6)), "1 ms since the last refill is none of one");
    }

    #[test]
    fn the_gate_caps_per_peer_and_global_concurrency() {
        let peer_id = |i| SecretKey::from_bytes(&{ let mut s = [0u8; 32]; s[0] = i; s });
        // The per-peer cap binds while the global one has room.
        let gate = RpcGate::new(8, 2, u32::MAX, u32::MAX);
        let (a, b) = (gate.peer(peer_id(1).public()), gate.peer(peer_id(2).public()));
        let g1 = gate.enter(&a).expect("first of a");
        let g2 = gate.enter(&a).expect("second of a");
        assert!(gate.enter(&a).is_err(), "a is over its per-peer cap");
        assert!(gate.enter(&b).is_ok(), "another peer still has room");
        drop(g1);
        assert!(gate.enter(&a).is_ok(), "a released permit went back to its semaphore");
        drop(g2);

        // The global cap binds across peers, whatever their own budgets.
        let gate = RpcGate::new(2, 8, u32::MAX, u32::MAX);
        let (a, b, c) = (gate.peer(peer_id(1).public()), gate.peer(peer_id(2).public()), gate.peer(peer_id(3).public()));
        let g1 = gate.enter(&a).expect("first of a");
        let g2 = gate.enter(&b).expect("first of b");
        assert!(gate.enter(&c).is_err(), "the global cap is spent");
        drop(g1);
        assert!(gate.enter(&c).is_ok(), "a released permit went back to the pool");
        drop(g2);
    }

    #[test]
    fn remembered_peers_stay_bounded() {
        let gate = RpcGate::new(8, 8, u32::MAX, u32::MAX);
        for _ in 0..=MAX_RPC_PEERS {
            gate.peer(SecretKey::generate().public());
        }
        let len = gate.peers.lock().unwrap().len();
        assert!(len <= MAX_RPC_PEERS, "{len} peers remembered");
    }

    #[test]
    fn full_peer_map_preserves_live_limiters_and_work_after_disconnect() {
        let gate = RpcGate::new(8, 1, 2, 1);
        let id = SecretKey::generate().public();
        let connected = gate.peer(id);
        let held = gate.enter(&connected).expect("first request is in flight");
        let live: Vec<_> = (1..MAX_RPC_PEERS).map(|_| gate.peer(SecretKey::generate().public())).collect();
        let unknown_id = SecretKey::generate().public();
        let refused = gate.peer(unknown_id);
        assert!(gate.enter(&refused).is_err(), "a full map must not evict a live connection's limiter");
        let reconnected = gate.peer(id);
        assert!(Arc::ptr_eq(&connected, &reconnected), "both connections share one identity's limiter");
        assert!(gate.enter(&reconnected).is_err(), "reconnect cannot split the concurrency cap");
        drop(connected);
        drop(reconnected);
        // ReadProtocol's task retains permits without retaining PeerLimit.
        // Even then the live request must prevent replacement of its entry.
        let refused = gate.peer(unknown_id);
        assert!(gate.enter(&refused).is_err(), "work outliving a connection still protects its limiter");
        let reconnected = gate.peer(id);
        assert!(gate.enter(&reconnected).is_err());
        drop(held);
        assert!(gate.enter(&reconnected).is_ok(), "the original limiter recovers its released permit");
        assert_eq!(gate.peers.lock().unwrap().len(), MAX_RPC_PEERS);
        drop(live);
    }

    #[test]
    fn full_peer_map_preserves_depleted_buckets_until_they_refill() {
        let gate = RpcGate::new(8, 1, 1, 1);
        let ids: Vec<_> = (0..MAX_RPC_PEERS).map(|_| SecretKey::generate().public()).collect();
        for id in &ids {
            let peer = gate.peer(*id);
            drop(gate.enter(&peer).expect("spend this identity's burst"));
        }
        // Pin accounting after key generation so the test needs no sleep and
        // cannot accidentally refill early on a contended test machine.
        let now = Instant::now();
        for peer in gate.peers.lock().unwrap().values() { peer.bucket.lock().unwrap().last = now + Duration::from_secs(3600); }
        let reconnected = gate.peer(ids[0]);
        assert!(gate.enter(&reconnected).is_err(), "reconnect cannot earn a new request burst");
        drop(reconnected);
        let new_id = SecretKey::generate().public();
        assert!(gate.enter(&gate.peer(new_id)).is_err(), "idle but depleted entries remain remembered");
        assert!(gate.peers.lock().unwrap().contains_key(&ids[0]));
        // Exactly one idle entry can now refill fully; only that entry may be
        // evicted for a new identity, without changing any live/depleted entry.
        {
            let peers = gate.peers.lock().unwrap();
            peers[&ids[0]].bucket.lock().unwrap().last = now - Duration::from_secs(2);
        }
        let admitted = gate.peer(new_id);
        assert!(gate.enter(&admitted).is_ok(), "fully refilled idle entry makes room");
        let peers = gate.peers.lock().unwrap();
        assert_eq!(peers.len(), MAX_RPC_PEERS);
        assert!(!peers.contains_key(&ids[0]));
        assert!(ids[1..].iter().all(|id| peers.contains_key(id)));
    }

    #[test]
    fn wallet_servers_expire_and_stay_bounded() {
        let id = |i| {
            SecretKey::from_bytes(&{
                let mut s = [0u8; 32];
                s[0] = i;
                s
            })
            .public()
        };
        let w = WalletServers::default();
        let t = Instant::now();
        w.announce_at(id(1), [1; 20], t);
        assert_eq!(w.live_at(t).len(), 1, "a fresh announcement is live");
        assert!(w
            .live_at(t + WALLET_SERVER_TTL - Duration::from_secs(1))
            .iter()
            .all(|(i, _)| *i == id(1)));
        assert!(
            w.live_at(t + WALLET_SERVER_TTL + Duration::from_secs(1))
                .is_empty(),
            "an announcement expires"
        );
        // The registry is bounded, however many announce.
        for i in 0..=(WALLET_SERVERS_KEPT as u8) + 1 {
            w.announce_at(id((i % 254) as u8 + 2), [1; 20], t + Duration::from_secs(i as u64));
        }
        let kept = w.live_at(t + Duration::from_secs(1)).len();
        assert!(
            kept <= WALLET_SERVERS_KEPT,
            "{kept} wallet servers remembered"
        );
        // A sample is a bounded, live-only subset.
        let sample = w.sample();
        assert!(
            sample.len() <= WALLET_SERVERS_SAMPLED,
            "{:?} sampled",
            sample.len()
        );
    }

    /// A candidate's voting key, its signature over an endpoint id, and the
    /// registry check that lists it (what `aether-node` supplies from its
    /// finalized registry state).
    fn candidate(seed: u64, endpoint: &EndpointId) -> (Vec<u8>, Vec<u8>, RegisteredCandidate) {
        use commonware_codec::Encode as _;
        use commonware_cryptography::{ed25519, Signer as _};
        let sk = ed25519::PrivateKey::from_seed(seed);
        let key = sk.public_key().encode().to_vec();
        let sig = sk.sign(WALLET_SERVER_NAMESPACE, endpoint.as_bytes()).encode().to_vec();
        let listed = key.clone();
        let registered: RegisteredCandidate = Arc::new(move |k| (k == listed.as_slice()).then_some([seed as u8; 20]));
        (key, sig, registered)
    }

    /// Only a registered candidate's own signature over the announcing
    /// endpoint id lists it: nobody else's key, nobody else's endpoint, an
    /// unregistered key, or a node without the registry check.
    #[test]
    fn announcements_need_a_registered_candidates_signature() {
        let endpoint = SecretKey::generate().public();
        let (key, sig, registered) = candidate(7, &endpoint);
        let w = WalletServers::new(Some(registered));
        w.announce(endpoint, &key, &sig).expect("a registered candidate's own signature lists it");
        assert_eq!(w.live_at(Instant::now()), vec![(endpoint, [7; 20])]);

        let w = WalletServers::new(Some(candidate(7, &endpoint).2));
        let err = w.announce(endpoint, &key, &sig[..63]).unwrap_err();
        assert!(err.contains("64 bytes"), "{err}");

        // A signature over another endpoint id is refused, so a captured one
        // is worthless (and nobody can announce an endpoint they do not run).
        let other = SecretKey::generate().public();
        let err = w.announce(other, &key, &sig).unwrap_err();
        assert!(err.contains("endpoint id"), "{err}");
        assert!(w.live_at(Instant::now()).is_empty());

        // An unregistered candidate's signature is refused.
        let (stranger, stranger_sig, _) = candidate(8, &endpoint);
        let err = w.announce(endpoint, &stranger, &stranger_sig).unwrap_err();
        assert!(err.contains("not a registered candidate"), "{err}");

        // A node without the registry check (a follower) lists nobody.
        let w = WalletServers::default();
        let err = w.announce(endpoint, &key, &sig).unwrap_err();
        assert!(err.contains("does not list"), "{err}");
    }

    /// Samples spread over distinct operators first: one Mac per operator
    /// before any operator's second, so one operator cannot fill a pool.
    #[test]
    fn samples_spread_over_distinct_operators() {
        let id = |i| {
            SecretKey::from_bytes(&{
                let mut s = [0u8; 32];
                s[0] = i;
                s
            })
            .public()
        };
        let w = WalletServers::default();
        let t = Instant::now();
        // Three operators, 50 Macs each.
        let mut by_operator = [0usize; 3];
        for i in 0..150u8 {
            let operator = [i % 3; 20];
            w.announce_at(id((i % 254) + 2), operator, t);
        }
        let sample = w.sample();
        assert_eq!(sample.len(), WALLET_SERVERS_SAMPLED);
        let live = w.live_at(t);
        for id in &sample {
            let (_, operator) = live.iter().find(|(i, _)| i == id).expect("sampled a live id");
            by_operator[operator[0] as usize] += 1;
        }
        assert!(by_operator.iter().all(|c| *c > 0), "every operator is in the sample: {by_operator:?}");
        let (max, min) = (by_operator.iter().max().copied().unwrap_or(0), by_operator.iter().min().copied().unwrap_or(0));
        assert!(max <= min + 1, "no operator takes more than its even share: {by_operator:?}");
    }

    /// One JSON-RPC call over one fresh bidirectional stream, ending in the
    /// response (an error object included), like `RpcClient` does.
    async fn rpc(conn: &Connection, method: &str) -> Value {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(
            &serde_json::to_vec(
                &serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": [] }),
            )
            .unwrap(),
        )
        .await
        .unwrap();
        let _ = send.finish();
        let bytes = recv.read_to_end(MAX_MESSAGE).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// A stream held in the handler blocks the peer's second stream, which is
    /// answered with a clear error instead of being served; once the first
    /// finishes, the peer may call again. Localhost only: no relay, no DHT.
    #[tokio::test]
    async fn a_peer_over_its_stream_limit_gets_a_jsonrpc_error() {
        let secret = SecretKey::generate();
        let id = secret.public();
        let server = Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .alpns(vec![ALPN_RPC.to_vec()])
            .secret_key(secret.clone())
            .bind()
            .await
            .unwrap();
        let port = server.bound_sockets().into_iter().find(|a| a.is_ipv4()).expect("an IPv4 socket").port();
        let (release, held) = (Arc::new(tokio::sync::Notify::new()), Arc::new(tokio::sync::Notify::new()));
        let (r, h) = (release.clone(), held.clone());
        let handler: Handler = Arc::new(move |req| {
            let (r, h) = (r.clone(), h.clone());
            Box::pin(async move {
                if req["method"] == "hold" {
                    h.notify_one();
                    r.notified().await;
                    serde_json::json!({ "jsonrpc": "2.0", "id": req["id"], "result": "held" })
                } else {
                    serde_json::json!({ "jsonrpc": "2.0", "id": req["id"], "result": "pong" })
                }
            })
        });
        // One concurrent stream per peer, a bucket that never runs dry.
        let router = Router::builder(server)
            .accept(
                ALPN_RPC,
                RpcProtocol {
                    handler,
                    gate: Arc::new(RpcGate::new(4, 1, u32::MAX, u32::MAX)),
                    wallets: None,
                    public_read: None,
                },
            )
            .spawn();

        let client = Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let addr = EndpointAddr::from_parts(
            id,
            [TransportAddr::Ip(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                port,
            ))],
        );
        let conn = client.connect(addr, ALPN_RPC).await.unwrap();

        let holding = tokio::spawn({
            let c = conn.clone();
            async move { rpc(&c, "hold").await }
        });
        tokio::time::timeout(Duration::from_secs(10), held.notified())
            .await
            .expect("the handler started");

        let busy = tokio::time::timeout(Duration::from_secs(10), rpc(&conn, "ping"))
            .await
            .expect("answered, not hung");
        let err = busy["error"]["message"]
            .as_str()
            .expect("a busy error, not a result");
        assert!(err.contains("server busy"), "{err}");
        assert_eq!(busy_retry_after(err), Some(BUSY_CONCURRENCY_RETRY), "the busy answer says when to come back: {err}");
        assert_eq!(busy["error"]["data"]["retry_after_ms"], 100, "also as data");

        release.notify_one();
        let held_answer = tokio::time::timeout(Duration::from_secs(10), holding)
            .await
            .expect("the held call finished")
            .unwrap();
        assert_eq!(held_answer["result"], "held");

        let again = tokio::time::timeout(Duration::from_secs(10), rpc(&conn, "ping"))
            .await
            .expect("the limit was released");
        assert_eq!(again["result"], "pong");
        client.close().await;
        let _ = router.shutdown().await;
    }

    #[tokio::test]
    async fn a_delayed_failure_keeps_the_replacement_pooled_connection() {
        let server = Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .alpns(vec![ALPN_RPC.to_vec()]).bind().await.unwrap();
        let port = server.bound_sockets().into_iter().find(|a| a.is_ipv4()).unwrap().port();
        let addr = EndpointAddr::from_parts(server.id(), [TransportAddr::Ip(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))]);
        let router = serve(server, |req| async move {
            serde_json::json!({ "jsonrpc": "2.0", "id": req["id"], "result": "pong" })
        }, None, None);
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled).bind().await.unwrap();
        let client = RpcClient::with_endpoint(endpoint.clone(), vec![]);
        let old = endpoint.connect(addr.clone(), ALPN_RPC).await.unwrap();
        client.pool.lock().unwrap().insert(0, old.clone());
        client.drop_failed_connection(0, &old); // First failed call removes A.
        let replacement = endpoint.connect(addr, ALPN_RPC).await.unwrap();
        assert_ne!(old.stable_id(), replacement.stable_id());
        client.pool.lock().unwrap().insert(0, replacement.clone());
        client.drop_failed_connection(0, &old); // A second A failure arrives late.
        assert_eq!(client.pool.lock().unwrap()[&0].stable_id(), replacement.stable_id());
        assert_eq!(rpc_call(&replacement, "ping", serde_json::json!([])).await.unwrap(), "pong");
        client.drop_failed_connection(0, &replacement);
        assert!(client.pool.lock().unwrap().is_empty(), "its own failure still removes it");
        endpoint.close().await;
        let _ = router.shutdown().await;
    }

    /// Wallet-server discovery over a real `aether/rpc/1` connection: a
    /// follower announces with its candidate key's signature over its own
    /// endpoint id, the server lists it only if its registry check knows the
    /// key, and another client learns the node id. Localhost only: no relay,
    /// no DHT.
    #[tokio::test]
    async fn wallet_server_discovery_rides_the_rpc_transport() {
        let secret = SecretKey::generate();
        let id = secret.public();
        let server = Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .alpns(vec![ALPN_RPC.to_vec()])
            .secret_key(secret.clone())
            .bind()
            .await
            .unwrap();
        let port = server
            .bound_sockets()
            .into_iter()
            .find(|a| a.is_ipv4())
            .expect("an IPv4 socket")
            .port();
        // The announcer's endpoint id is known before it connects, so its
        // candidate key can sign it and the registry check can list the key.
        let announcer_secret = SecretKey::generate();
        let announcer_id = announcer_secret.public();
        let (key, sig, registered) = candidate(7, &announcer_id);
        // The interception answers before the handler, which never runs here.
        let router = serve(
            server,
            |_req| async move { unreachable!("discovery is answered by the transport") },
            None,
            Some(registered),
        );
        let addr = EndpointAddr::from_parts(
            id,
            [TransportAddr::Ip(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                port,
            ))],
        );

        let announcer_ep = Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .secret_key(announcer_secret)
            .bind()
            .await
            .unwrap();
        let announce = |key: Vec<u8>, sig: Vec<u8>| {
            let (ep, addr) = (announcer_ep.clone(), addr.clone());
            async move {
                let conn = connect_rpc(&ep, &addr, Duration::from_secs(10)).await.unwrap();
                rpc_call(&conn, "aether_announceWalletServer", serde_json::json!([hex::encode(key), hex::encode(sig)])).await
            }
        };
        announce(key, sig).await.expect("announced");
        // An unregistered candidate is refused an answer, and not listed.
        let (stranger, stranger_sig, _) = candidate(8, &announcer_id);
        let refused = announce(stranger, stranger_sig).await.unwrap_err();
        assert!(refused.to_string().contains("announcement refused"), "{refused}");

        let reader = RpcClient::with_addrs(vec![addr]).await.unwrap();
        let listed = reader
            .call("aether_walletServers", serde_json::json!([]))
            .await
            .expect("listed");
        let mine = announcer_id.to_string();
        assert!(
            listed
                .as_array()
                .is_some_and(|a| a.iter().any(|s| s.as_str() == Some(mine.as_str()))),
            "the announcer's own node id {mine} is listed: {listed}"
        );
        announcer_ep.close().await;
        reader.endpoint().close().await;
        let _ = router.shutdown().await;
    }

    /// The incident of 2026-10-05: after a reboot every validator had come
    /// back on a new address, every connect attempt burned its full 20 s, and
    /// a scan of four under the client's lock queued every read behind 80 s —
    /// again and again, because nothing bounded the scan as a whole. The scan
    /// now has its own budget (the call returns promptly even with far more
    /// dead nodes than the budget covers), and a budget-exhausted scan moves
    /// `start` past every node it got through, so the next one begins at the
    /// next candidate — a dead first validator is not re-dialed forever while
    /// the rest go untried.
    #[tokio::test]
    async fn a_scan_for_a_live_node_is_bounded_and_round_robins() {
        // TEST-NET-1: packets go nowhere, so each attempt runs its full budget.
        let blackhole = |i: u8| {
            EndpointAddr::from_parts(
                SecretKey::from_bytes(&[i; 32]).public(),
                [TransportAddr::Ip(SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
                    1,
                ))],
            )
        };
        let client = RpcClient::with_addrs(vec![blackhole(1), blackhole(2), blackhole(3), blackhole(4)])
            .await
            .unwrap()
            .with_connect_timeouts(Duration::from_millis(200), Duration::from_millis(260));
        for scan in 0..3 {
            let t = Instant::now();
            let err = client.connection().await.unwrap_err().to_string();
            assert!(err.contains("connect"), "{err}");
            assert!(
                t.elapsed() < Duration::from_secs(2),
                "scan {scan} took {:?}; the whole scan is bounded, not just each attempt",
                t.elapsed()
            );
            let start = client.start.load(std::sync::atomic::Ordering::Relaxed);
            assert!(start > 0 || scan > 0, "a dead first node must not be retried forever");
        }
        // The cached-connection path is untouched: a scan cannot lock the
        // client for longer than its budget, so `describe` (and any queued
        // `call`) waits behind at most one bounded scan.
        let t = Instant::now();
        let _ = client.describe().await;
        assert!(t.elapsed() < Duration::from_secs(2), "waiting on the scan lock is bounded too");
        client.endpoint().close().await;
    }
}
