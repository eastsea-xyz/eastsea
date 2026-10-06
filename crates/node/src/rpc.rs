//! JSON-RPC 2.0 over HTTP (POST /). Serves the finalized state.
//!
//! `aether_getAccount` returns an EIP-7864 Merkle proof so clients can verify
//! balances against the state root instead of trusting this server.

use crate::chain::Chain;
use aether_execution::validate_stateless;
use aether_state::layout::{basic_data_key, code_hash_key, storage_slot_key};
use aether_state::StateRepository;
use aether_types::{Address, TxEnvelope, TxHash, U256};
use axum::{extract::State, routing::get, routing::post, Json, Router};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Validators serve certified blocks from marshal; a follower from what it verified.
#[derive(Clone)]
pub enum Finality {
    Marshal(Marshal),
    Archive(std::sync::Arc<crate::follow::FinalityArchive>),
}

pub type Marshal = commonware_consensus::marshal::core::Mailbox<aether_light::Scheme, commonware_consensus::marshal::standard::Standard<crate::block::Block>>;

#[derive(Clone)]
pub struct RpcState {
    pub chain: Chain,
    /// Source of finalized blocks and certificates for light clients.
    pub finality: Finality,
    /// Accepted txs are forwarded here for p2p gossip.
    pub gossip: mpsc::UnboundedSender<TxEnvelope>,
    /// Set on nodes run with the faucet key.
    pub faucet: Option<std::sync::Arc<crate::faucet::Faucet>>,
    /// Set on nodes run with a DeviceCheck key (one node identity per Mac).
    pub registrar: Option<std::sync::Arc<crate::devicecheck::Registrar>>,
    /// Validators: their network.json (public), handed to joining voting nodes.
    pub network: Option<Value>,
    /// Followers: where rotation questions go (validators know the running set).
    pub upstream: Option<std::sync::Arc<crate::follow::Upstream>>,
    /// Validators of a DKG committee: signs handoffs of their own staged reshare.
    pub handoff: Option<std::sync::Arc<crate::handoff::Service>>,
    /// The last snapshot served, by height (built once, served in chunks).
    pub snapshot: SnapshotCache,
    /// Set when this node proves blocks (protocol 2).
    pub prover: Option<crate::prover::SharedStatus>,
    /// Set on history v2 networks: the era shards this node holds and checks
    /// (roadmap B5 phase 1).
    pub shards: Option<std::sync::Arc<crate::shards::Shards>>,
    /// The public read-only gateway mode (`--public-read-only`,
    /// docs/ops/read-gateway.md): only the reads the explorer uses pass; every
    /// write, node-local and heavy method is refused, and the caps below apply.
    /// The bind stays loopback either way — exposure goes through a tunnel.
    pub public_read_only: bool,
}

/// What the public read-only gateway lets through `handle_value`: exactly the
/// reads the explorer calls (apps/explorer/js) plus the light-client reads its
/// account verification needs. Everything else — the faucet, every send and
/// registration, snapshots, shards, era chunks — is refused, so a gateway can
/// never relay a transaction or trigger node-side work.
const PUBLIC_READ_METHODS: &[&str] = &[
    "aether_status",
    "aether_recentBlocks",
    "aether_candidates",
    "aether_proverStatus",
    "aether_getBlock",
    "aether_getReceipt",
    "aether_getAccount",
    "aether_getFinalized",
    "aether_history",
    "aether_historyProof",
    "aether_eraInfo",
    "aether_eraProof",
    "aether_rewards",
    "aether_accountHistory",
    "eth_blockNumber",
    "eth_call",
    "eth_getLogs",
];

/// Public-gateway caps (docs/research/public-read-access-2026-10-05.md §8a):
/// a stranger's request may cost at most this much.
/// Largest request body accepted from the public (HTTP body limit).
pub const PUBLIC_MAX_BODY: usize = 1 << 20;
/// Calls per JSON-RPC batch.
pub const PUBLIC_MAX_BATCH: usize = 8;
/// Blocks per `eth_getLogs` query (the same window the handler scans).
pub const PUBLIC_GETLOGS_WINDOW: u64 = 2_000;
/// Gas an `eth_call` may run (the private answer is the block gas limit).
pub const PUBLIC_CALL_GAS: u64 = 1_000_000;
/// Rows per `aether_rewards` answer.
pub const PUBLIC_REWARDS_LIMIT: u64 = 1_000;
/// Rows per `aether_accountHistory` page.
pub const PUBLIC_HISTORY_LIMIT: u64 = 100;

/// The public gateway's gate, run after alias normalization and before any
/// handler or upstream forwarding: method allowlist first, then the per-method
/// caps a request states itself.
fn public_gate(st: &RpcState, method: &str, p: &Value) -> Result<(), (i64, String)> {
    if !PUBLIC_READ_METHODS.contains(&method) {
        return Err((-32601, format!("public read-only gateway: {method} is not a public read method (docs/ops/read-gateway.md); writes and node-local methods are refused")));
    }
    match method {
        // A window wider than the node scans is refused, not silently clamped:
        // the asker learns the cap instead of an answer that pretends completeness.
        "eth_getLogs" => {
            let f = p.get(0).cloned().unwrap_or_default();
            let head = st.chain.lock().finalized.height;
            // The range the request itself states, before the handler clamps
            // it to what this node kept: the cap judges the ask, not the answer.
            let to = block_param(&f, "toBlock", head);
            let from = block_param(&f, "fromBlock", head);
            // An inverted ask is refused here, at the gate (pre-audit 7
            // PA7-02): the handler answers inverted ranges with an error, and
            // the gate holds strangers to the same shape it asks of them.
            if from > to {
                return Err((-32602, format!("public read-only gateway: eth_getLogs fromBlock {from} is above toBlock {to}")));
            }
            if to.saturating_sub(from) >= PUBLIC_GETLOGS_WINDOW {
                return Err((-32002, format!("public read-only gateway: eth_getLogs is capped at {PUBLIC_GETLOGS_WINDOW} blocks per query; ask a narrower range")));
            }
        }
        "eth_call" => {
            if let Some(c) = p.get(0) {
                let asked = match c.get("gas") {
                    Some(Value::String(s)) => u64::from_str_radix(s.trim_start_matches("0x"), 16).ok(),
                    Some(Value::Number(n)) => n.as_u64(),
                    _ => None,
                };
                if asked.is_some_and(|g| g > PUBLIC_CALL_GAS) {
                    return Err((-32002, format!("public read-only gateway: eth_call gas is capped at {PUBLIC_CALL_GAS}")));
                }
            }
        }
        "aether_rewards" => {
            if p.get(1).and_then(Value::as_u64).is_some_and(|l| l > PUBLIC_REWARDS_LIMIT) {
                return Err((-32002, format!("public read-only gateway: aether_rewards limit is capped at {PUBLIC_REWARDS_LIMIT} rows")));
            }
        }
        "aether_accountHistory" => {
            if p.get(2).and_then(Value::as_u64).is_some_and(|l| l > PUBLIC_HISTORY_LIMIT) {
                return Err((-32002, format!("public read-only gateway: aether_accountHistory limit is capped at {PUBLIC_HISTORY_LIMIT} rows")));
            }
        }
        _ => {}
    }
    Ok(())
}

/// The snapshot being served: (height, serialized bytes). One caller builds;
/// other callers get a prompt error instead of lining up behind a long build.
pub type CachedSnapshot = (u64, Arc<Vec<u8>>);

#[derive(Clone, Default)]
pub struct SnapshotCache(Arc<SnapshotCacheInner>);

#[derive(Default)]
struct SnapshotCacheInner {
    bytes: Mutex<Option<CachedSnapshot>>,
    queued: AtomicBool,
    building: AtomicBool,
    last_attempt: Mutex<Option<Instant>>,
    manifest: Mutex<Option<(std::sync::Weak<Vec<u8>>, String)>>,
}

impl SnapshotCache {
    pub fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Option<CachedSnapshot>>> {
        self.0.bytes.lock()
    }

    fn manifest(&self, bytes: &Arc<Vec<u8>>) -> String {
        let mut cached = self.0.manifest.lock().expect("snapshot manifest");
        if let Some((old, digest)) = cached.as_ref() {
            if old.upgrade().is_some_and(|old| Arc::ptr_eq(&old, bytes)) { return digest.clone(); }
        }
        let digest = blake3_hex(bytes);
        *cached = Some((Arc::downgrade(bytes), digest.clone()));
        digest
    }
}

struct SnapshotBuild<'a>(&'a AtomicBool);
impl Drop for SnapshotBuild<'_> {
    fn drop(&mut self) { self.0.store(false, Ordering::Release); }
}

struct QueuedBuild(Arc<SnapshotCacheInner>);
impl Drop for QueuedBuild {
    fn drop(&mut self) { self.0.queued.store(false, Ordering::Release); }
}

/// Bytes per snapshot chunk (well under transport message limits).
pub const SNAPSHOT_CHUNK: usize = 1 << 20;

/// The snapshot of the finalized state, rebuilt at most once per 120 blocks.
fn cached_snapshot(st: &RpcState) -> Result<CachedSnapshot, (i64, String)> {
    let height = st.chain.finalized_height();
    let fresh = || {
        st.snapshot.lock().expect("snapshot cache").as_ref().and_then(|(h, b)|
            (h.saturating_add(120) >= height).then(|| (*h, b.clone())))
    };
    if let Some(hit) = fresh() { return Ok(hit); }
    if st.snapshot.0.building.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err((-32000, "snapshot rebuild already running; retry later".into()));
    }
    let _building = SnapshotBuild(&st.snapshot.0.building);
    if let Some(hit) = fresh() { return Ok(hit); }
    {
        let mut last = st.snapshot.0.last_attempt.lock().expect("snapshot attempt");
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(30)) {
            return Err((-32000, "snapshot rebuild rate limited; retry in 30 seconds".into()));
        }
        *last = Some(Instant::now());
    }
    let source = crate::snapshot::Snapshot::source(&st.chain);
    // The pressure policy and the memory budget live in one place
    // (resources::snapshot_gate_for): critical always refuses, warn and normal
    // refuse only a build that does not fit the budget.
    crate::resources::snapshot_build_gate(source.estimated_peak_bytes()).map_err(|e| (-32000, e))?;
    let s = source.build();
    let bytes = s.to_bytes();
    if bytes.len() > 1 << 30 {
        return Err((-32000, "snapshot build refused: wire size exceeds 1 GiB".into()));
    }
    let built = (s.summary.height, Arc::new(bytes));
    *st.snapshot.0.manifest.lock().expect("snapshot manifest") = Some((Arc::downgrade(&built.1), blake3_hex(&built.1)));
    *st.snapshot.lock().expect("snapshot cache") = Some(built.clone());
    Ok(built)
}

async fn snapshot_manifest(st: &RpcState) -> RpcResult {
    let height = st.chain.finalized_height();
    let cached = st.snapshot.lock().expect("snapshot cache").clone();
    if let Some((h, bytes)) = cached.as_ref() {
        if h.saturating_add(120) >= height {
            return Ok(json!({ "height": h, "size": bytes.len(), "blake3": st.snapshot.manifest(bytes), "chunk": SNAPSHOT_CHUNK }));
        }
    }
    if st.snapshot.0.queued.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err((-32000, "snapshot rebuild already queued; retry later".into()));
    }
    // A request occupies at most one blocking worker. Repeated RPC calls do
    // not queue behind it or block the runtime that drives finalization.
    let state = st.clone();
    tokio::task::spawn_blocking(move || {
        let _queued = QueuedBuild(state.snapshot.0.clone());
        let (h, bytes) = cached_snapshot(&state)?;
        Ok(json!({ "height": h, "size": bytes.len(), "blake3": state.snapshot.manifest(&bytes), "chunk": SNAPSHOT_CHUNK }))
    })
    .await
    .map_err(|e| (-32000, format!("snapshot worker failed: {e}")))?
}

pub fn blake3_hex(b: &[u8]) -> String {
    blake3::hash(b).to_hex().to_string()
}

pub async fn serve(addr: SocketAddr, state: RpcState) -> std::io::Result<()> {
    // Loopback only; any origin may ask (web pages and dApps read through this
    // node; every write still needs the user's signature in the wallet).
    // The public read-only gateway is no exception: it binds loopback too and
    // reaches the internet only through a cloudflared tunnel
    // (docs/ops/read-gateway.md). Refusing a non-loopback bind here beats
    // relying on the operator remembering that at 3 a.m.
    let public = state.public_read_only;
    if public && !addr.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("--public-read-only binds loopback only (asked for {addr}); expose it through a tunnel, docs/ops/read-gateway.md"),
        ));
    }
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([axum::http::Method::POST, axum::http::Method::GET, axum::http::Method::OPTIONS])
        .allow_headers([axum::http::header::CONTENT_TYPE]);
    let mut app = Router::new().route("/", post(handle));
    if !public {
        // Era files as plain GETs (roadmap B6): the same bytes `aether_eraChunk`
        // hands out hex-encoded, for torrent webseeds and curl — a node's own
        // first webseed, on its private (loopback/tunnel) listener. The public
        // read-only gateway never registers the route (pre-audit 7 PA7-03):
        // serving one buffers a whole era file with no public-side need for
        // it, a stranger's lever the explorer never uses; intentional
        // webseeding belongs to an export node's own listener.
        app = app.route("/era/{name}", get(serve_era_file));
    }
    let app = app.layer(cors).with_state(state);
    // The public gateway also caps what one request may make this node parse:
    // an oversized Content-Length is refused at the head, with a 413 the asker
    // can read; DefaultBodyLimit backstops a chunked or lying body.
    let app = if public {
        app.layer(axum::extract::DefaultBodyLimit::max(PUBLIC_MAX_BODY))
            .layer(axum::middleware::from_fn(public_body_cap))
    } else {
        app
    };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await
}

async fn handle(State(st): State<RpcState>, Json(req): Json<Value>) -> Json<Value> {
    Json(handle_value(&st, req).await)
}

/// The public gateway refuses an oversized request at the header stage, so the
/// asker gets a readable 413 instead of a connection dropped mid-body.
async fn public_body_cap(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let over = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .is_some_and(|n| n > PUBLIC_MAX_BODY);
    if over {
        return (axum::http::StatusCode::PAYLOAD_TOO_LARGE, "public read-only gateway: request body is capped at 1 MiB (docs/ops/read-gateway.md)").into_response();
    }
    next.run(req).await
}

/// Concurrent `/era/` transfers this node serves at once (pre-audit 7
/// PA7-03): each one buffers a whole era file, so even the private,
/// loopback-only listener a webseed tunnels through serves a bounded number —
/// a mirror hammering it cannot turn the node into an era-file faucet that
/// crowds out everything else.
const MAX_ERA_TRANSFERS: usize = 4;
static ERA_TRANSFERS: AtomicUsize = AtomicUsize::new(0);

/// A counted slot in a global budget: acquire-or-refuse (a CAS loop), given
/// back on drop. The gateway's bounded-execution budgets share this shape.
struct BudgetSlot(&'static AtomicUsize);
impl BudgetSlot {
    fn acquire(counter: &'static AtomicUsize, max: usize) -> Option<Self> {
        let mut n = counter.load(Ordering::Acquire);
        loop {
            if n >= max {
                return None;
            }
            match counter.compare_exchange(n, n + 1, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(BudgetSlot(counter)),
                Err(now) => n = now,
            }
        }
    }
}
impl Drop for BudgetSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// `GET /era/<file>`: a whole era file from this node's era folder — a
/// webseed (the export manifest's first mirror). Names are exactly
/// `era-<eight digits>.aera`; anything else is a 404, never a path.
async fn serve_era_file(State(st): State<RpcState>, axum::extract::Path(name): axum::extract::Path<String>) -> axum::response::Response {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    // Defense in depth (PA7-03): the public gateway does not register this
    // route; if a future route table ever re-adds it, the handler still
    // refuses rather than serving era files to strangers.
    if st.public_read_only {
        return (StatusCode::NOT_FOUND, "no such era here").into_response();
    }
    let Some(_transfer) = BudgetSlot::acquire(&ERA_TRANSFERS, MAX_ERA_TRANSFERS) else {
        return (StatusCode::SERVICE_UNAVAILABLE, "era transfer budget busy; retry shortly").into_response();
    };
    let era = name
        .strip_prefix("era-")
        .and_then(|n| n.strip_suffix(".aera"))
        .filter(|n| n.len() == 8 && n.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|n| n.parse::<u64>().ok());
    let bytes = era.and_then(|_| {
        let store = st.chain.store()?;
        let path = store.era_dir().join(&name);
        std::fs::metadata(&path).ok().filter(|m| m.is_file() && m.len() <= crate::era_net::MAX_ERA_FILE as u64)?;
        std::fs::read(path).ok()
    });
    match bytes {
        Some(b) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream"), (header::CONTENT_LENGTH, b.len().to_string().as_str())],
            b,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "no such era here").into_response(),
    }
}

/// The project's new name (docs/design/25-rename.md phase 4): `eastsea_*`
/// method spellings alias the `aether_*` ones. Rewritten once here, before any
/// matching, so handlers, responses and error texts stay exactly as they were.
/// Forwarded calls (`upstream`) go out under the canonical old name too.
fn normalize_method(method: &str) -> std::borrow::Cow<'_, str> {
    match method.strip_prefix("eastsea_") {
        Some(rest) => std::borrow::Cow::Owned(format!("aether_{rest}")),
        None => std::borrow::Cow::Borrowed(method),
    }
}

/// Transport-independent JSON-RPC handling (HTTP on loopback, iroh QUIC publicly).
/// A request array is a batch, answered entry by entry; entries may not nest.
pub async fn handle_value(st: &RpcState, req: Value) -> Value {
    if let Value::Array(entries) = &req {
        if entries.is_empty() {
            return json!({ "jsonrpc": "2.0", "id": Value::Null, "error": { "code": -32600, "message": "empty batch" } });
        }
        if st.public_read_only && entries.len() > PUBLIC_MAX_BATCH {
            return json!({ "jsonrpc": "2.0", "id": Value::Null,
                "error": { "code": -32002, "message": format!("public read-only gateway: batches are capped at {PUBLIC_MAX_BATCH} calls") } });
        }
        let mut answers = Vec::with_capacity(entries.len());
        for e in entries {
            answers.push(match e {
                Value::Object(_) => single(st, e.clone()).await,
                _ => json!({ "jsonrpc": "2.0", "id": Value::Null, "error": { "code": -32600, "message": "batch entries must be objects" } }),
            });
        }
        return json!(answers);
    }
    single(st, req).await
}

async fn single(st: &RpcState, req: Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = normalize_method(req.get("method").and_then(Value::as_str).unwrap_or_default());
    let params = req.get("params").cloned().unwrap_or(Value::Array(vec![]));
    // Before any handler or upstream hop: on the public gateway only the
    // allowlisted reads (within their caps) reach the machinery in RpcState.
    if st.public_read_only {
        if let Err((code, msg)) = public_gate(st, &method, &params) {
            return json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } });
        }
    }
    let result = match &*method {
        "aether_getFinalized" => finalized(st, &params).await,
        "eth_call" => eth_call(st, &params, if st.public_read_only { PUBLIC_CALL_GAS } else { PRIVATE_CALL_GAS }).await,
        "aether_getReceiptProof" => receipt_proof(st, &params).await,
        "aether_snapshot" => snapshot_manifest(st).await,
        "aether_registerDevice" => register_device(st, &params).await,
        "aether_sendBeacon" => send_beacon(st, &params).await,
        "aether_sendRegistration" => send_registration(st, &params).await,
        // A follower without the registrar key asks upstream (one hop).
        "aether_reattest" if st.registrar.is_none() && st.upstream.is_some() => {
            st.upstream.as_ref().expect("checked").first("aether_reattest", params.clone()).await.map_err(|e| (-32000, e))
        }
        "aether_reattest" => reattest(st, &params).await,
        // Followers ask validators (one hop: a forwarded question is never forwarded again).
        "aether_rotation" | "aether_network" if st.upstream.is_some() => match params.get(0) {
            Some(Value::Bool(true)) => Ok(Value::Null),
            _ => st.upstream.as_ref().expect("checked").first(&method, json!([true])).await.map_err(|e| (-32000, e)),
        },
        "aether_network" => Ok(st.network.clone().unwrap_or(Value::Null)),
        // One forwarded hop reaches a validator even when a follower is the
        // first RPC peer; a forwarded follower returns null so upstream tries
        // another peer instead of cycling indefinitely.
        "aether_proverProgram" if st.upstream.is_some() => match params.get(0) {
            Some(Value::Bool(true)) => Ok(Value::Null),
            _ => st.upstream.as_ref().expect("checked").first("aether_proverProgram", json!([true])).await.map_err(|e| (-32000, e)),
        },
        "aether_submitProof" => submit_proof(st, &params).await,
        // A pruned height (roadmap B4) or one whose cache copy the memory
        // budget dropped: read back from the era file, fetched and verified first if needed.
        "aether_getBlock" if param::<u64>(&params, 0).is_ok_and(|h| {
            let g = st.chain.lock();
            h < g.pruned_below.max(g.cache_below)
        }) => old_block(st, &params).await,
        _ => dispatch(st, &method, &params),
    };
    match result {
        Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }),
        Err((code, msg)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } }),
    }
}

type RpcResult = Result<Value, (i64, String)>;

/// A finalized receipt with its inclusion proof and the block certificate.
async fn receipt_proof(st: &RpcState, p: &Value) -> RpcResult {
    use commonware_codec::Decode;
    let hash: TxHash = param(p, 0)?;
    let (height, index, receipt, receipts) = {
        let g = st.chain.lock();
        let Some((height, receipt)) = g.receipts.get(&hash) else {
            return Ok(if g.mempool.contains_key(&hash) { json!({ "pending": true }) } else { Value::Null });
        };
        let unavailable = || (-32000, "receipt proof unavailable on this node".to_string());
        let block = g.blocks.get(height).ok_or_else(unavailable)?;
        let index = block.txs.iter().position(|h| h == &hash).ok_or_else(unavailable)?;
        let receipts = block.txs.iter().map(|h| {
            g.receipts.get(h).filter(|(h, _)| h == height).map(|(_, r)| r.clone())
        }).collect::<Option<Vec<_>>>().ok_or_else(unavailable)?;
        (*height, index, receipt.clone(), receipts)
    };
    // Hashing the complete block's receipts can take longer than a map lookup.
    // Leave the chain lock before constructing the path so RPC cannot delay a vote.
    let proof = aether_execution::receipt::receipt_proof(&receipts, index)
        .ok_or((-32000, "receipt proof unavailable on this node".to_string()))?;
    let root = aether_execution::receipt::receipt_root(&receipts);
    let certified_block = finalized(st, &json!([height])).await?;
    if certified_block.is_null() {
        return Err((-32000, "receipt certificate unavailable on this node".into()));
    }
    let bytes = certified_block["block"].as_str().and_then(|b| aether_light::from_hex(b).ok())
        .ok_or((-32000, "invalid receipt certificate block".to_string()))?;
    let block = crate::block::Block::decode_cfg(bytes.as_slice(), &crate::block::Block::codec_config(aether_light::MAX_BLOCK_BYTES))
        .map_err(|_| (-32000, "invalid receipt certificate block".to_string()))?;
    let committed = block.payload().and_then(|payload| payload.receipts_root)
        .ok_or((-32000, "receipt proofs unavailable for legacy blocks".to_string()))?;
    if block.height.get() != height || committed != root {
        return Err((-32000, "stored receipts do not match certified block".into()));
    }
    Ok(json!({ "height": height, "index": index, "receipt": receipt, "proof": proof, "certified_block": certified_block }))
}

/// Codec bytes of finalized block `h` and its finalization certificate.
/// Light clients verify both themselves; nothing here needs to be trusted.
async fn finalized(st: &RpcState, p: &Value) -> RpcResult {
    use commonware_codec::Encode;
    use commonware_consensus::types::Height;
    let h: u64 = param(p, 0)?;
    let pruned_below = st.chain.lock().pruned_below;
    if h < pruned_below {
        return Err((-32001, pruned_message(h, pruned_below)));
    }
    let marshal = match &st.finality {
        Finality::Marshal(m) => m,
        Finality::Archive(a) => return Ok(a.get(h).unwrap_or(Value::Null)),
    };
    let Some(block) = marshal.get_block(Height::new(h)).await else {
        // Below a voting node's floor: the history it verified as a follower.
        let kept = st.chain.store().and_then(|s| s.proof(h).ok().flatten()).and_then(|b| serde_json::from_slice(&b).ok());
        return Ok(kept.unwrap_or(Value::Null));
    };
    // A height finalized as an ancestor has no certificate of its own: prove it
    // through the blocks built on it up to the next certified one (`links`).
    let mut links = Vec::new();
    for k in h..=h + aether_light::MAX_LINKS as u64 {
        if k > h {
            let Some(b) = marshal.get_block(Height::new(k)).await else { return Ok(Value::Null) };
            links.push(aether_light::to_hex(&b.encode()));
        }
        if let Some(fin) = marshal.get_finalization(Height::new(k)).await {
            return Ok(json!({
                "height": h,
                "block": aether_light::to_hex(&block.encode()),
                "finalization": aether_light::to_hex(&fin.encode()),
                "links": links,
            }));
        }
    }
    Ok(Value::Null)
}

/// Why a pruned height has no certificate here (followers look for "pruned").
pub fn pruned_message(h: u64, pruned_below: u64) -> String {
    format!(
        "pruned: this node keeps blocks from height {pruned_below}; block {h} is in era {} (aether_eraInfo / aether_eraChunk / aether_eraProof)",
        h / aether_state::mmr::ERA_LEN
    )
}

/// A pruned block from its era file (roadmap B4): the fields a summary has
/// that the block itself carries, plus its codec bytes. A follower without the
/// file fetches the era from upstream and verifies it first.
async fn old_block(st: &RpcState, p: &Value) -> RpcResult {
    use commonware_codec::Encode;
    use commonware_cryptography::Digestible;
    let h: u64 = param(p, 0)?;
    let chain = st.chain.clone();
    let mut read = {
        let c = chain.clone();
        tokio::task::spawn_blocking(move || c.old_block(h)).await.map_err(|e| (-32000, e.to_string()))?
    };
    if let (Err(_), Some(up)) = (&read, &st.upstream) {
        // Public history reads stay local-only (pre-audit 7 PA7-04): fetching
        // the era would save a whole file this node deliberately pruned, one
        // stranger's request at a time — the pruned error below already tells
        // clients where history lives (eraInfo / eraChunk / eraProof). Private
        // nodes and followers keep the self-healing fetch.
        if !st.public_read_only {
            let era = h / aether_state::mmr::ERA_LEN;
            crate::era_net::fetch_into(&chain, up, era).await.map_err(|e| (-32000, e))?;
            read = tokio::task::spawn_blocking(move || chain.old_block(h)).await.map_err(|e| (-32000, e.to_string()))?;
        }
    }
    let b = read.map_err(|e| (-32001, e))?;
    let payload = b.payload().ok_or((-32000, "block payload".to_string()))?;
    let state_root = {
        let g = st.chain.lock();
        g.blocks.get(&(h + 1)).map(|n| n.parent_state_root)
    };
    let state_root = match state_root {
        Some(r) => Some(r),
        None => {
            let c = st.chain.clone();
            tokio::task::spawn_blocking(move || c.old_block(h + 1)).await.ok().and_then(Result::ok).and_then(|n| n.payload()).map(|p| p.parent_state_root)
        }
    };
    Ok(json!({
        "height": h,
        "hash": format!("{}", b.digest()),
        "parent": format!("{}", b.parent),
        "timestamp_ms": b.timestamp,
        "proposer": crate::chain::leader_address(&b.context.leader),
        "state_root": state_root,
        "parent_state_root": payload.parent_state_root,
        "txs": payload.txs.iter().map(aether_execution::tx_hash).collect::<Vec<_>>(),
        "gas_used": payload.gas.exec,
        "prove_gas": payload.gas.prove,
        "pruned": true,
        "block": aether_light::to_hex(&b.encode()),
    }))
}

/// `[device_token (base64), operator, validator_key (hex 32), node_id (hex 32), beaconer, ownership (hex)]`
/// (`ownership`: the voting key's signature, `aether candidate-info --operator`)
/// → the registrar's attestation (r, s) to submit to the registry contract.
async fn register_device(st: &RpcState, p: &Value) -> RpcResult {
    let r = st.registrar.as_ref().ok_or((-32601, "this node does not register devices".to_string()))?;
    // The registry's registrar key decides: if the committee rotated or stopped
    // it, this node must not sign attestations that are already dead (G11).
    crate::devicecheck::registrar_key_check(&st.chain.lock().finalized.state, &r.signer.public_hex()).map_err(|e| (-32000, e))?;
    let token: String = param(p, 0)?;
    let operator: Address = param(p, 1)?;
    let hex32 = |i: usize| -> Result<[u8; 32], (i64, String)> {
        let s: String = param(p, i)?;
        hex::decode(s.trim_start_matches("0x")).ok().and_then(|b| b.try_into().ok()).ok_or((-32602, format!("param {i}: 32-byte hex")))
    };
    let (key, node) = (hex32(2)?, hex32(3)?);
    let beaconer: Address = param(p, 4)?;
    let ownership: String = param(p, 5)?;
    let ownership = hex::decode(ownership.trim_start_matches("0x")).map_err(|_| (-32602, "param 5: ownership signature hex".to_string()))?;
    let a = r.register(&token, operator, key, node, beaconer, &ownership).await.map_err(|e| (-32000, e.to_string()))?;
    Ok(json!({ "r": hex::encode(a.r), "s": hex::encode(a.s), "registered_at": a.registered_at }))
}

/// `[answer]`: a registered Mac's beacon answer (no fee, no transaction).
/// Checked against the finalized state, pooled for the next proposals; a
/// validator sends it on to the others, a follower to its upstream.
async fn send_beacon(st: &RpcState, p: &Value) -> RpcResult {
    let a: aether_light::block::BeaconAnswer = param(p, 0)?;
    let new = st.chain.submit_beacon(a.clone()).map_err(|e| (-32000, format!("rejected: {e}")))?;
    if let Some(up) = st.upstream.clone().filter(|_| new) {
        tokio::spawn(async move {
            if let Err(e) = up.call("aether_sendBeacon", json!([a])).await {
                tracing::warn!(%e, "beacon answer not forwarded upstream");
            }
        });
    }
    Ok(json!({ "accepted": new }))
}

/// `[registration]`: a voting-node registration for the block's free lane (no
/// fee, no transaction; docs/design/22-gas-pool.md 2층). Checked against the
/// finalized state and pooled for the next proposals; a validator sends it on
/// to the others, a follower to its upstream. Returns the item's id, which
/// gets a pseudo-receipt in the block that carries it.
async fn send_registration(st: &RpcState, p: &Value) -> RpcResult {
    let r: aether_light::block::NodeRegistration = param(p, 0)?;
    let id = crate::registrations::id(&r);
    let new = st.chain.submit_registration(r.clone()).map_err(|e| (-32000, format!("rejected: {e}")))?;
    if let Some(up) = st.upstream.clone().filter(|_| new) {
        tokio::spawn(async move {
            if let Err(e) = up.call("aether_sendRegistration", json!([r])).await {
                tracing::warn!(%e, "registration not forwarded upstream");
            }
        });
    }
    Ok(json!({ "hash": id, "accepted": new }))
}

/// `[device_token (base64), validator_key (hex 32), period, ownership (hex)]`
/// → the registrar's re-attestation `{period, r, s}` for a beacon answer.
/// Only for the current re-attestation period (or the next, near its start).
async fn reattest(st: &RpcState, p: &Value) -> RpcResult {
    let r = st.registrar.as_ref().ok_or((-32601, "this node does not re-attest devices".to_string()))?;
    let token: String = param(p, 0)?;
    let key: String = param(p, 1)?;
    let key: [u8; 32] = hex::decode(key.trim_start_matches("0x")).ok().and_then(|b| b.try_into().ok()).ok_or((-32602, "param 1: 32-byte hex".to_string()))?;
    let period: u64 = param(p, 2)?;
    let ownership: String = param(p, 3)?;
    let ownership = hex::decode(ownership.trim_start_matches("0x")).map_err(|_| (-32602, "param 3: signature hex".to_string()))?;
    let (low, high) = {
        let g = st.chain.lock();
        let f = &g.finalized;
        // A stopped or rotated registrar key signs re-attestations nobody takes.
        crate::devicecheck::registrar_key_check(&f.state, &r.signer.public_hex()).map_err(|e| (-32000, e))?;
        let epoch = (f.height + 1) / aether_execution::registry::epoch_blocks(&f.state);
        let period = |slot| aether_rewards::beacons::period(&f.state, epoch, slot);
        (period(0), period(aether_rewards::SLOTS - 1) + 1)
    };
    if period < low || period > high {
        return Err((-32000, format!("period {period} is not current ({low}..={high})")));
    }
    let (rr, ss) = r.reattest(&token, key, period, &ownership).await.map_err(|e| (-32000, e.to_string()))?;
    Ok(json!({ "period": period, "r": hex::encode(rr), "s": hex::encode(ss) }))
}

fn param<T: serde::de::DeserializeOwned>(p: &Value, i: usize) -> Result<T, (i64, String)> {
    let v = p.get(i).cloned().ok_or((-32602, format!("missing param {i}")))?;
    serde_json::from_value(v).map_err(|e| (-32602, format!("param {i}: {e}")))
}

fn dispatch(st: &RpcState, method: &str, p: &Value) -> RpcResult {
    let chain = &st.chain;
    match method {
        "aether_status" => {
            let resources = crate::resources::monitor().map(|m| m.status_value()).unwrap_or(Value::Null);
            let g = chain.lock();
            let f = &g.finalized;
            let base = Chain::next_base_fee(&g.cfg, f);
            let mut base_fee = json!({ "exec": base.exec.to_string(), "prove": base.prove.to_string() });
            if base.state != 0 {
                base_fee["state"] = json!(base.state.to_string());
            }
            // Catching up (a node that slept, or one still starting): the app
            // shows this instead of a height that looks stale.
            let behind = g.net_height.map_or(0, |n| n.saturating_sub(f.height));
            Ok(json!({
                // Base fees (wei per unit) for the next block: the exec base is burned, prove goes to the prover escrow.
                "base_fee": base_fee,
                "prover_escrow": f.state.balance(&aether_execution::PROVER_ESCROW),
                "chain_id": g.cfg.chain_id,
                "height": f.height,
                "hash": format!("{}", f.digest),
                "state_root": f.state.root(),
                "timestamp_ms": f.timestamp,
                "mempool": g.mempool.len(),
                "hash_function": "blake3",
                // How many blocks the network is ahead of this node (0 when
                // caught up, or when nothing told it a height), and whether it
                // is still catching up.
                "catching_up": behind > 0,
                "behind": behind,
                // Stage-wise progress (red team #2): a frozen height with a
                // rising `activity` is a node busy on a snapshot, a store
                // recovery or a replay — not a stuck one. `stage` names the
                // long stage when there is one.
                "activity": crate::chain::activity(),
                "stage": crate::chain::stage(),
                // Protocol upgrades on chain: an app whose node runs an older
                // protocol than one scheduled looks for its update right away.
                "protocol": f.next_protocol(),
                "node_protocol": crate::upgrade::implements(),
                // Informational only: followers compare this verifier's guest
                // against their prover before submitting proofs.
                "prover_program": g.verifier.as_ref().and_then(|v| v.program_id()),
                "newest_scheduled": f.schedule.iter().map(|a| a.protocol).max().unwrap_or(1),
                // Activations on chain as (protocol, at-height) pairs: a genesis
                // above protocol 1 carries its own at height 0, so `[3, 0]` here
                // is how a rehearsal knows the rules were on from the start.
                "schedule": f.schedule.iter().map(|a| json!([a.protocol, a.at])).collect::<Vec<_>>(),
                "upcoming_upgrades": g.upgrade_notices,
                // The free registration lane (G2): wallets see it and register
                // without needing a balance for a paid contract call.
                "free_registration": aether_rewards::enabled(&f.state),
                // Keep the status at top level for wallets that only inspect
                // the standard status fields, with measurements under resources.
                "disk_almost_full": resources["disk_almost_full"].as_bool().unwrap_or(false),
                "disk_status": resources["disk_status"].as_str().unwrap_or("unknown"),
                "resources": resources,
                // The faucet this node runs, when it runs one: wallets label
                // grants from this address as "faucet" in the balance breakdown.
                "faucet": st.faucet.as_ref().map(|f| json!(f.address)).unwrap_or(Value::Null),
            }))
        }
        // The next relay nonce a free-lane registration of `operator` must
        // carry (`[operator]`): the count the chain has spent of its items.
        "aether_registrationNonce" => {
            let a: Address = param(p, 0)?;
            let g = chain.lock();
            Ok(json!(aether_execution::registry::lane_nonce(&g.finalized.state, &a)))
        }
        // The voting set proposed for this ceremony window (while no handoff is
        // pending): `aether run` on old and new members reshares to it in the background.
        "aether_rotation" => {
            let g = chain.lock();
            let f = g.finalized.clone();
            let params = aether_execution::registry::params(&f.state);
            let Some((draw, members)) = g.proposal.clone() else { return Ok(Value::Null) };
            let pending = f.handoff.as_ref().is_some_and(|p| f.height < p.switch);
            if (draw != f.height / (params.epoch_blocks * params.draw_epochs)
                && !crate::chain::proposal_in_window(&g, f.height, &f.state))
                || pending
            {
                return Ok(Value::Null);
            }
            let next: Vec<Value> = members.iter().map(|(k, n)| json!({ "key": k, "node": n })).collect();
            if g.cfg.chain_id == 7_780 {
                Ok(json!({ "epoch": draw, "next": next, "network": st.network }))
            } else {
                Ok(json!({ "epoch": draw, "height": f.height, "next": next, "network": st.network }))
            }
        }
        // The latest committee handoff on this node's finalized chain (verified here).
        "aether_handoff" => {
            let f = chain.lock().finalized.clone();
            Ok(f.handoff.as_ref().map_or(Value::Null, |p| {
                json!({ "at": p.at, "switch": p.switch, "round": p.handoff.round, "output": p.handoff.output,
                        "members": p.handoff.members.iter().map(|(k, n)| json!({ "key": k, "node": n })).collect::<Vec<_>>(),
                        "finalized": f.height })
            }))
        }
        // Sign the handoff of this validator's own staged reshare (see handoff::Service).
        "aether_signHandoff" => {
            let svc = st.handoff.as_ref().ok_or((-32601, "this node does not sign handoffs".to_string()))?;
            svc.sign_staged().map(|h| json!({ "round": h.round })).map_err(|e| (-32000, e))
        }
        // The finalized state as a checkpoint snapshot (legacy postcard or
        // new-genesis notice envelope, `snapshot::Snapshot`):
        // a new Mac checks it against the next certified block instead of replaying history.
        // One chunk of the cached snapshot at `height` (hex); the whole is checked by its BLAKE3.
        "aether_snapshotChunk" => {
            let height: u64 = param(p, 0)?;
            let index: usize = param(p, 1)?;
            // Chunk requests never initiate a rebuild. An old manifest stays
            // downloadable while the next manifest is being prepared.
            let (h, bytes) = st.snapshot.lock().expect("snapshot cache").clone().ok_or((-32000, "snapshot not available; request aether_snapshot first".to_string()))?;
            if h != height {
                return Err((-32000, format!("snapshot moved on to height {h}")));
            }
            let start = index.saturating_mul(SNAPSHOT_CHUNK).min(bytes.len());
            let end = (start + SNAPSHOT_CHUNK).min(bytes.len());
            Ok(json!({ "data": hex::encode(&bytes[start..end]) }))
        }
        // Inclusion of block `height` in the history under block `anchor`'s
        // history root (`aether_light::verify_history`).
        "aether_historyProof" => {
            let height: u64 = param(p, 0)?;
            let anchor: u64 = param(p, 1)?;
            let code = |e: &String| if e.starts_with("need") { -32602 } else { -32000 };
            let (proof, hash) = chain.history_proof(height, anchor).map_err(|e| (code(&e), e))?;
            Ok(json!({ "height": height, "hash": hash, "anchor": anchor, "proof": proof }))
        }
        // What history this node keeps (roadmap B4).
        "aether_history" => {
            let g = chain.lock();
            let eras = g.history_index.as_ref().map(|i| i.eras.len());
            Ok(json!({ "pruned_below": g.pruned_below, "head": g.finalized.height, "complete_eras": eras, "era_len": aether_state::mmr::ERA_LEN }))
        }
        // Era files, served to peers that pruned them or never had them (`era_net`).
        "aether_eraInfo" => {
            let era: u64 = param(p, 0)?;
            Ok(chain.store().map(|s| crate::era_net::info(&s, era)).unwrap_or(Value::Null))
        }
        "aether_eraChunk" => {
            let era: u64 = param(p, 0)?;
            let index: usize = param(p, 1)?;
            let s = chain.store().ok_or((-32000, "no store".to_string()))?;
            crate::era_net::chunk(&s, era, index).map_err(|e| (-32000, e))
        }
        // The era's root under block `anchor`'s history root (`aether_light::verify_era_root`).
        "aether_eraProof" => {
            let era: u64 = param(p, 0)?;
            let anchor: u64 = param(p, 1)?;
            let proof = chain.era_proof(era, anchor).map_err(|e| (if e.starts_with("need") { -32602 } else { -32000 }, e))?;
            Ok(json!({ "era": era, "anchor": anchor, "proof": proof }))
        }
        // This node's shard of an era (roadmap B5 phase 1): the shard bytes,
        // their commitment and the candidate answering. A peer checks the
        // Merkle path against the commitment it holds for the era.
        "aether_shard" => {
            let era: u64 = param(p, 0)?;
            let index: u16 = param(p, 1)?;
            st.shards
                .as_ref()
                .ok_or((-32601, "this network keeps no era shards (history v2 only)".to_string()))?
                .serve(era, index)
                .map(|held| held.unwrap_or(Value::Null))
                .map_err(|e| (-32000, e))
        }
        // Era shard holding and challenge results (last 7 days), per candidate:
        // the public statistic of phase 1. No reward weight anywhere in it.
        "aether_shardStats" => st
            .shards
            .as_ref()
            .map(|s| s.stats(chain))
            .ok_or((-32601, "this network keeps no era shards (history v2 only)".to_string())),
        // Voting-node candidates (the registry) and the current epoch.
        "aether_candidates" => {
            let g = chain.lock();
            let state = &g.finalized.state;
            let epoch = g.finalized.height / aether_execution::registry::epoch_blocks(state);
            let list: Vec<Value> = aether_execution::registry::candidates(state)
                .into_iter()
                .map(|c| {
                    json!({
                        "index": c.index,
                        "operator": c.operator,
                        "validator_key": hex::encode(c.validator_key),
                        "node_id": hex::encode(c.node_id),
                        "beaconer": c.beaconer,
                        "registered_epoch": c.registered_epoch,
                        "last_epoch": c.last_epoch,
                        "streak": c.streak,
                    })
                })
                .collect();
            Ok(json!({ "epoch": epoch, "candidates": list, "max_per_epoch": aether_execution::registry::max_per_epoch(state) }))
        }
        "aether_faucet" => {
            let to: Address = param(p, 0)?;
            let f = st.faucet.as_ref().ok_or((-32601, "this node does not run the faucet".to_string()))?;
            let tx = f.grant(chain, to, std::time::Instant::now()).map_err(|e| (-32000, e.to_string()))?;
            let hash = aether_execution::tx_hash(&tx);
            match chain.add_to_mempool(tx.clone()) {
                Ok(true) => {
                    let _ = st.gossip.send(tx);
                }
                Ok(false) => {}
                Err(e) => {
                    f.cancel(to, &tx);
                    return Err((-32000, e));
                }
            }
            Ok(json!({ "hash": hash, "amount_wei": crate::faucet::GRANT.to_string() }))
        }
        "aether_proverStatus" => Ok(match &st.prover {
            Some(s) => {
                let status = s.lock().map_err(|_| (-32000, "status lock".to_string()))?.clone();
                let mut v = serde_json::to_value(&status).unwrap_or_default();
                // How far proving trails the chain, and the last reward received.
                let head = chain.finalized_height();
                v["lag"] = json!(status.last_height.map(|h| head.saturating_sub(h)));
                v["last_reward"] = status.payout
                    .map(|a| last_proof_reward(chain.rewards(&a)))
                    .unwrap_or(Value::Null);
                v
            }
            None => json!({ "running": false }),
        }),
        "aether_proverProgram" => Ok(chain.lock().verifier.as_ref()
            .and_then(|v| v.program_id()).map_or(Value::Null, Value::String)),
        // Node rewards at a glance (docs/design/15-node-rewards.md): how many
        // operators shared the last epoch's pool, and with `[operator]` that
        // operator's Macs, expected share, and what the last distribution paid.
        "aether_rewardStatus" => {
            let operator: Option<Address> =
                p.get(0).map(|v| serde_json::from_value(v.clone())).transpose().map_err(|e| (-32602, format!("param 0: {e}")))?;
            let (chain_id, f, dist, root, epoch_blocks) = {
                let g = chain.lock();
                let f = g.finalized.clone();
                let epoch_blocks = aether_execution::registry::epoch_blocks(&f.state);
                // The first block of this epoch distributed the last one's pool.
                let dist = f.height / epoch_blocks * epoch_blocks;
                (g.cfg.chain_id, f, dist, g.blocks.get(&dist).map(|b| b.state_root), epoch_blocks)
            };
            // What that distribution actually paid the operator: its node record
            // is within this epoch's worth of newest rewards (every block since
            // distributed at most one more). Skipped whole on networks without
            // node rewards, which answer `{"enabled": false}` and nothing else.
            let received = operator
                .filter(|_| aether_rewards::enabled(&f.state))
                .and_then(|op| {
                    let records = chain.recent_rewards(&op, epoch_blocks.min(10_000) as usize);
                    crate::rewards_view::received_from_records(&records, dist)
                });
            Ok(crate::rewards_view::status(chain_id, &f.state, f.height, root, operator, received))
        }
        // The newest rewards (at most 10,000 per call; the app asks for all of them for tax records).
        "aether_rewards" => {
            let a: Address = param(p, 0)?;
            let cap = if st.public_read_only { PUBLIC_REWARDS_LIMIT } else { 10_000 };
            let limit = p.get(1).and_then(Value::as_u64).unwrap_or(1_000).min(cap) as usize;
            Ok(json!(chain.recent_rewards(&a, limit)))
        }
        // Rewards, paged: `aether_rewards` answers one newest-first array whose
        // default limit (1,000) once hid the rest of a long history behind a
        // silent cut. This one hands out pages with a cursor and the total
        // count, so a wallet can load everything and say "N of M".
        "aether_rewardsPage" => {
            let a: Address = param(p, 0)?;
            let cursor = p.get(1).filter(|v| !v.is_null()).map(|v| v.as_str().ok_or((-32602, "cursor must be a string".to_string()))).transpose()?;
            let limit = p.get(2).and_then(Value::as_u64).unwrap_or(1_000).min(10_000) as usize;
            Ok(chain.rewards_page(&a, cursor.as_deref(), limit).map_err(|e| (-32000, e))?)
        }
        "aether_accountHistory" => {
            let address: Address = param(p, 0)?;
            let cursor = p.get(1).filter(|v| !v.is_null()).map(|v| v.as_str().ok_or((-32602, "cursor must be a string".to_string()))).transpose()?;
            let limit = p.get(2).filter(|v| !v.is_null()).map(|v| v.as_u64().ok_or((-32602, "limit must be a positive integer".to_string()))).transpose()?.unwrap_or(50);
            let cap = if st.public_read_only { PUBLIC_HISTORY_LIMIT } else { 200 };
            if !(1..=cap).contains(&limit) { return Err((-32602, format!("limit must be 1..{cap}"))); }
            Ok(json!(chain.account_history(&address, cursor, limit as usize).map_err(|e| (-32000, e))?))
        }
        "aether_sendTransaction" => {
            let tx: TxEnvelope = param(p, 0)?;
            let cfg = chain.cfg();
            validate_stateless(&tx, cfg.chain_id).map_err(|e| (-32000, format!("invalid transaction: {e:?}")))?;
            let hash = aether_execution::tx_hash(&tx);
            if chain.add_to_mempool(tx.clone()).map_err(|e| (-32000, format!("rejected: {e}")))? {
                let _ = st.gossip.send(tx);
            }
            Ok(json!({ "hash": hash }))
        }
        "aether_getAccount" => {
            let a: Address = param(p, 0)?;
            let g = chain.lock();
            let f = &g.finalized;
            let repo = f.state.repo();
            let proof = repo.prove(&[basic_data_key(repo.hasher(), &a)]).remove(0);
            Ok(json!({
                "address": a,
                "balance": f.state.balance(&a),
                "nonce": f.state.nonce(&a),
                "code_size": f.state.code(&a).len(),
                "height": f.height,
                "state_root": f.state.root(),
                "proof": proof,
            }))
        }
        "aether_getStorage" => {
            let a: Address = param(p, 0)?;
            let slot: U256 = param(p, 1)?;
            let g = chain.lock();
            let f = &g.finalized;
            let repo = f.state.repo();
            let proof = repo.prove(&[storage_slot_key(repo.hasher(), &a, slot)]).remove(0);
            Ok(json!({ "value": f.state.storage(&a, slot), "height": f.height, "state_root": f.state.root(), "proof": proof }))
        }
        "aether_getCodeHash" => {
            let a: Address = param(p, 0)?;
            let g = chain.lock();
            let f = &g.finalized;
            let repo = f.state.repo();
            let proof = repo.prove(&[code_hash_key(repo.hasher(), &a)]).remove(0);
            Ok(json!({ "value": f.state.code_hash(&a), "height": f.height, "proof": proof }))
        }
        "aether_releaseEntries" => {
            // Discovery only. Wallets verify every selected entry with
            // `aether_getStorage` proofs and the pinned ReleaseLog code hash.
            let a: Address = param(p, 0)?;
            let start: u64 = param(p, 1).unwrap_or(0);
            let limit: u64 = param(p, 2).unwrap_or(32).min(64);
            let g = chain.lock();
            let f = &g.finalized;
            Ok(release_entries(&f.state, a, start, limit, f.height))
        }
        "aether_getReceipt" => {
            let h: TxHash = param(p, 0)?;
            let g = chain.lock();
            match g.receipts.get(&h) {
                Some((height, r)) => Ok(json!({ "height": height, "receipt": r })),
                None if g.mempool.contains_key(&h) => Ok(json!({ "pending": true })),
                None => Ok(Value::Null),
            }
        }
        "aether_getBlock" => {
            let height: u64 = param(p, 0)?;
            let g = chain.lock();
            Ok(g.blocks.get(&height).map(|b| json!(b)).unwrap_or(Value::Null))
        }
        "aether_recentBlocks" => {
            let n: usize = param(p, 0).unwrap_or(10).min(100);
            let g = chain.lock();
            Ok(json!(g.blocks.values().rev().take(n).collect::<Vec<_>>()))
        }
        // Minimal Ethereum-compatible reads.
        "eth_chainId" => Ok(json!(format!("0x{:x}", chain.cfg().chain_id))),
        "eth_getLogs" => eth_get_logs(chain, p),
        "eth_blockNumber" => Ok(json!(format!("0x{:x}", chain.lock().finalized.height))),
        "eth_getBalance" => {
            let a: Address = param(p, 0)?;
            Ok(json!(format!("0x{:x}", chain.lock().finalized.state.balance(&a))))
        }
        "eth_getTransactionCount" => {
            let a: Address = param(p, 0)?;
            Ok(json!(format!("0x{:x}", chain.lock().finalized.state.nonce(&a))))
        }
        "eth_getCode" => {
            let a: Address = param(p, 0)?;
            Ok(json!(format!("0x{}", hex::encode(chain.lock().finalized.state.code(&a)))))
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

fn last_proof_reward(rows: Vec<Value>) -> Value {
    rows.into_iter().rev()
        .find(|r| r["kind"] == "proof")
        .and_then(|r| r.get("amount").cloned())
        .unwrap_or(Value::Null)
}

fn release_entries(state: &aether_execution::WorldState, address: Address, start: u64, limit: u64, height: u64) -> Value {
    let count = state.storage(&address, U256::ZERO).min(U256::from(u64::MAX)).to::<u64>();
    let base = U256::from_be_bytes(alloy_primitives::keccak256([0u8; 32]).0);
    let entries: Vec<_> = (start..count.min(start.saturating_add(limit.min(64))))
        .map(|index| {
            let slot = base + U256::from(index) * U256::from(4);
            let metadata = state.storage(&address, slot + U256::from(3));
            json!({
                "index": index,
                "manifest_sha256": format!("{:064x}", state.storage(&address, slot)),
                "archive_sha256": format!("{:064x}", state.storage(&address, slot + U256::from(1))),
                "signatures_sha256": format!("{:064x}", state.storage(&address, slot + U256::from(2))),
                "published_block": (metadata & U256::from(u64::MAX)).to::<u64>(),
                "published_at": ((metadata >> 64usize) & U256::from(u64::MAX)).to::<u64>(),
                "emergency": ((metadata >> 128usize) & U256::from(1)).to::<u8>() == 1,
            })
        })
        .collect();
    json!({ "count": count, "height": height, "entries": entries })
}

/// A chain at genesis in an `RpcState` with nothing attached — enough to
/// check routing, gates and errors (shared by the test modules below).
#[cfg(test)]
fn bare_state() -> RpcState {
    let (chain, _) = Chain::new(crate::chain::ChainConfig {
        chain_id: 7781,
        limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
        min_streak: None, draw_epochs: None, history_v2: false, protocol: 1,
        node_rewards: false, committee: vec![], reserve: None, group: 0,
        max_committee: crate::rotation::GROW_UNTIL,
    });
    let (gossip, _) = mpsc::unbounded_channel();
    RpcState {
        chain,
        finality: Finality::Archive(Arc::new(crate::follow::FinalityArchive::default())),
        gossip,
        faucet: None,
        registrar: None,
        network: None,
        upstream: None,
        handoff: None,
        snapshot: Default::default(),
        prover: None,
        shards: None,
        public_read_only: false,
    }
}

#[cfg(test)]
async fn call(st: &RpcState, method: &str, params: Value) -> Value {
    handle_value(st, json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })).await
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn receipt_proof_includes_ordered_receipts_and_the_height_certificate() {
        use commonware_codec::Encode;
        let st = bare_state();
        let receipts: Vec<aether_execution::Receipt> = (1u8..=3).map(|byte| aether_execution::Receipt {
            tx_hash: aether_types::B256::repeat_byte(byte), success: true,
            gas_used: 21_000, prove_gas: 0, state_gas: 0, state_fee: U256::ZERO,
            contract_address: None, logs: 0, output: Default::default(), events: vec![],
        }).collect();
        let height = 1;
        {
            let mut g = st.chain.lock();
            g.blocks.insert(height, crate::chain::BlockSummary {
                height, hash: "block".into(), parent: "parent".into(), timestamp_ms: 0,
                proposer: Address::ZERO, state_root: Default::default(), parent_state_root: Default::default(),
                txs: receipts.iter().map(|r| r.tx_hash).collect(), gas_used: 63_000, prove_gas: 0,
                base_fee: Default::default(), excess: Default::default(),
                archive_excess: 0,
            });
            // Insertion order deliberately differs from block execution order.
            for receipt in receipts.iter().rev() {
                g.receipts.insert(receipt.tx_hash, (height, receipt.clone()));
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let params = json!([receipts[1].tx_hash]);
        assert_eq!(rt.block_on(call(&st, "aether_getReceiptProof", params.clone()))["error"]["code"], -32000);
        let genesis = crate::block::Block::genesis(7781, Default::default());
        let payload = crate::block::Payload {
            receipts_root: Some(aether_execution::receipt::receipt_root(&receipts)),
            ..Default::default()
        };
        let block = crate::block::Block::new(genesis.context, genesis.parent,
            commonware_consensus::types::Height::new(height), 1_000, payload.to_bytes());
        let certificate = json!({ "height": height, "block": aether_light::to_hex(&block.encode()), "finalization": "0x02", "links": [] });
        if let Finality::Archive(archive) = &st.finality { archive.insert(height, certificate.clone()); }
        let answer = rt.block_on(call(&st, "aether_getReceiptProof", params.clone()));
        let result = &answer["result"];
        assert_eq!(result["height"], height);
        assert_eq!(result["index"], 1);
        assert_eq!(result["receipt"], json!(receipts[1]));
        assert_eq!(result["proof"], json!(aether_execution::receipt::receipt_proof(&receipts, 1).unwrap()));
        assert_eq!(result["certified_block"], certificate);
        assert_eq!(answer, rt.block_on(call(&st, "eastsea_getReceiptProof", params.clone())));
        assert_eq!(rt.block_on(call(&st, "aether_getReceipt", params))["result"], json!({ "height": height, "receipt": receipts[1] }));
        assert_eq!(rt.block_on(call(&st, "aether_getReceiptProof", json!([aether_types::B256::ZERO])))["result"], Value::Null);
        st.chain.lock().receipts.get_mut(&receipts[1].tx_hash).unwrap().1.success = false;
        let corrupt = rt.block_on(call(&st, "aether_getReceiptProof", json!([receipts[1].tx_hash])));
        assert_eq!(corrupt["error"]["message"], "stored receipts do not match certified block");
        st.chain.lock().receipts.get_mut(&receipts[1].tx_hash).unwrap().1.success = true;
        if let Finality::Archive(archive) = &st.finality {
            let mut legacy = certificate.clone();
            legacy["block"] = json!(aether_light::to_hex(&crate::block::Block::genesis(7781, Default::default()).encode()));
            archive.insert(height, legacy);
        }
        let legacy = rt.block_on(call(&st, "aether_getReceiptProof", json!([receipts[1].tx_hash])));
        assert_eq!(legacy["error"]["message"], "receipt proofs unavailable for legacy blocks");
        st.chain.lock().receipts.remove(&receipts[0].tx_hash);
        assert_eq!(rt.block_on(call(&st, "aether_getReceiptProof", json!([receipts[1].tx_hash])))["error"]["code"], -32000);
    }

    #[test]
    fn status_advertises_the_verifier_program_for_follower_compatibility() {
        struct Pinned;
        impl crate::chain::ProofVerifier for Pinned {
            fn verify(&self, _: &[u8], _: [u8; 32]) -> bool { true }
            fn program_id(&self) -> Option<String> { Some("validator-program".into()) }
        }
        let st = bare_state();
        st.chain.lock().verifier = Some(Arc::new(Pinned));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let answer = rt.block_on(call(&st, "aether_status", json!([])));
        assert_eq!(answer["result"]["prover_program"], "validator-program");
        let program = rt.block_on(call(&st, "aether_proverProgram", json!([])));
        assert_eq!(program["result"], "validator-program");
    }

    #[test]
    fn last_prover_reward_ignores_newer_node_rewards() {
        let rows = vec![
            json!({"kind": "proof", "amount": "0x10"}),
            json!({"kind": "node", "amount": "0x20"}),
        ];
        assert_eq!(last_proof_reward(rows), "0x10");
    }

    #[test]
    fn eastsea_methods_alias_aether_ones() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = bare_state();
        // The same call under both spellings must answer identically.
        for (method, params) in [
            ("aether_status", json!([])),
            ("aether_network", json!([])),
            ("aether_recentBlocks", json!([])),
            ("aether_history", json!([])),
            ("aether_getAccount", json!(["0x0000000000000000000000000000000000000001"])),
        ] {
            let old = rt.block_on(call(&st, method, params.clone()));
            assert!(
                old.get("error").and_then(|e| e["message"].as_str()).is_none_or(|m| !m.contains("method not found")),
                "{method} itself must keep working"
            );
            let alias = format!("eastsea_{}", &method["aether_".len()..]);
            let new = rt.block_on(call(&st, &alias, params));
            assert_eq!(old, new, "{method} and its eastsea_ spelling must answer identically");
        }
        // An unknown method stays unknown under the new prefix, and methods
        // outside the renamed family are untouched.
        for missing in ["eastsea_nope", "eastsea_"] {
            let r = rt.block_on(call(&st, missing, json!([])));
            assert_eq!(r["error"]["code"], -32601, "{missing} must be method-not-found");
            assert!(r["error"]["message"].as_str().unwrap().contains("method not found"));
        }
        assert_eq!(rt.block_on(call(&st, "eth_chainId", json!([])))["result"], "0x1e65");
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;

    #[test]
    fn release_entries_are_paginated_from_state() {
        let address: Address = "0x0000000000000000000000000000000000007704".parse().unwrap();
        let mut state = aether_execution::WorldState::default();
        state.set_storage(address, U256::ZERO, U256::from(2));
        let base = U256::from_be_bytes(alloy_primitives::keccak256([0u8; 32]).0);
        state.set_storage(address, base + U256::from(4), U256::from(42));
        state.set_storage(address, base + U256::from(7), U256::from(100) | (U256::from(1_000) << 64usize) | (U256::from(1) << 128usize));
        let page = release_entries(&state, address, 1, 1, 200);
        assert_eq!(page["count"], 2);
        assert_eq!(page["height"], 200);
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["entries"][0]["manifest_sha256"], format!("{:064x}", U256::from(42)));
        assert_eq!(page["entries"][0]["published_block"], 100);
        assert_eq!(page["entries"][0]["published_at"], 1_000);
        assert_eq!(page["entries"][0]["emergency"], true);
        assert!(release_entries(&state, address, 2, 1, 200)["entries"].as_array().unwrap().is_empty());
    }

    #[test]
    fn snapshot_rebuild_is_single_flight_and_rate_limited() {
        // This test must not depend on the machine it runs on: a busy Mac sits
        // at WARN most of the day. Pin normal pressure and roomy memory (the
        // seam exists only in test builds — resources.rs).
        let _seam = crate::resources::SEAM.lock().unwrap_or_else(|e| e.into_inner());
        crate::resources::set_test_readings(Some(crate::resources::PRESSURE_NORMAL), Some(64 * crate::resources::GB));
        let (chain, _) = Chain::new(crate::chain::ChainConfig {
            chain_id: 7781,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: false, protocol: 1,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        });
        let (gossip, _) = mpsc::unbounded_channel();
        let st = RpcState {
            chain,
            finality: Finality::Archive(Arc::new(crate::follow::FinalityArchive::default())),
            gossip,
            faucet: None,
            registrar: None,
            network: None,
            upstream: None,
            handoff: None,
            snapshot: Default::default(),
            prover: None,
            shards: None,
            public_read_only: false,
        };
        st.snapshot.0.building.store(true, Ordering::Release);
        assert!(cached_snapshot(&st).unwrap_err().1.contains("already running"));
        st.snapshot.0.building.store(false, Ordering::Release);
        st.snapshot.0.queued.store(true, Ordering::Release);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let busy = rt.block_on(handle_value(&st, json!({ "id": 1, "method": "aether_snapshot", "params": [] })));
        assert!(busy["error"]["message"].as_str().unwrap().contains("already queued"));
        st.snapshot.0.queued.store(false, Ordering::Release);
        *st.snapshot.0.last_attempt.lock().unwrap() = Some(Instant::now());
        assert!(cached_snapshot(&st).unwrap_err().1.contains("rate limited"));
        *st.snapshot.0.last_attempt.lock().unwrap() = None;
        let manifest = rt.block_on(handle_value(&st, json!({ "id": 2, "method": "aether_snapshot", "params": [] })));
        assert!(manifest["error"].is_null(), "the blocking worker built the manifest: {manifest}");
        let first = cached_snapshot(&st).unwrap();
        let second = cached_snapshot(&st).unwrap();
        assert!(Arc::ptr_eq(&first.1, &second.1), "one build per cache window");
        assert_eq!(manifest["result"]["blake3"], blake3_hex(&first.1));
        let chunk = rt.block_on(handle_value(&st, json!({ "id": 3, "method": "aether_snapshotChunk", "params": [first.0, 0] })));
        assert_eq!(hex::decode(chunk["result"]["data"].as_str().unwrap()).unwrap(), *first.1);
    }
}

/// The public read-only gateway (docs/research/public-read-access-2026-10-05.md
/// §8a): allowlist, caps, aliasing and the loopback-only bind.
#[cfg(test)]
mod public_read_tests {
    use super::*;

    fn public_state() -> RpcState {
        let mut st = bare_state();
        st.public_read_only = true;
        st
    }

    fn gate_error(v: &Value) -> bool {
        v.get("error").and_then(|e| e["message"].as_str()).is_some_and(|m| m.contains("public read-only gateway"))
    }

    /// Tests that fire a real eth_call share PUBLIC_CALLS — a process-global
    /// budget — with the test that fills it on purpose. Claim this lock for
    /// the whole test so the two cannot flake on each other.
    fn claim_call_budget() -> std::sync::MutexGuard<'static, ()> {
        static CLAIM: Mutex<()> = Mutex::new(());
        CLAIM.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every write, node-local and heavy method is refused before its handler
    /// runs — the gateway can neither relay a transaction nor trigger node-side
    /// work. Aliased spellings are refused too (the gate runs after normalize).
    #[test]
    fn every_write_and_node_local_method_is_refused() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = public_state();
        let refused = [
            // The research doc's refusal list, plus every other non-read method.
            ("aether_sendTransaction", json!(["00"])),
            ("aether_faucet", json!(["0x0000000000000000000000000000000000000001"])),
            ("aether_registerDevice", json!(["t", "0x0000000000000000000000000000000000000001", "00", "00", "0x0000000000000000000000000000000000000001", "00"])),
            ("aether_sendBeacon", json!([{}])),
            ("aether_sendRegistration", json!([{}])),
            ("aether_reattest", json!(["t", "00", 0, "00"])),
            ("aether_signHandoff", json!([])),
            ("aether_submitProof", json!([{}])),
            ("aether_handoff", json!([])),
            ("aether_snapshot", json!([])),
            ("aether_snapshotChunk", json!([0, 0])),
            ("aether_rotation", json!([])),
            ("aether_network", json!([])),
            ("aether_proverProgram", json!([])),
            ("aether_eraChunk", json!([0, 0])),
            ("aether_shard", json!([0, 0])),
            ("aether_shardStats", json!([])),
            ("aether_registrationNonce", json!(["0x0000000000000000000000000000000000000001"])),
            ("aether_rewardStatus", json!([])),
            ("aether_rewardsPage", json!(["0x0000000000000000000000000000000000000001"])),
            ("aether_getReceiptProof", json!(["0x0000000000000000000000000000000000000000000000000000000000000001"])),
            ("aether_getStorage", json!(["0x0000000000000000000000000000000000000001", "0x0"])),
            ("aether_getCodeHash", json!(["0x0000000000000000000000000000000000000001"])),
            ("aether_releaseEntries", json!(["0x0000000000000000000000000000000000000001"])),
            ("eth_chainId", json!([])),
            ("eth_getBalance", json!(["0x0000000000000000000000000000000000000001"])),
            ("eth_getTransactionCount", json!(["0x0000000000000000000000000000000000000001"])),
            ("eth_getCode", json!(["0x0000000000000000000000000000000000000001"])),
            ("eastsea_faucet", json!(["0x0000000000000000000000000000000000000001"])),
            ("eastsea_sendTransaction", json!(["00"])),
            ("aether_nope", json!([])),
        ];
        for (method, params) in refused {
            let answer = rt.block_on(call(&st, method, params));
            assert_eq!(answer["error"]["code"], -32601, "{method} must be refused by name");
            assert!(gate_error(&answer), "{method} must say it was the public gateway that refused");
        }
    }

    /// Every allowlisted read reaches its handler: whatever it answers on a
    /// genesis chain, the public gateway itself never stands in the way.
    #[test]
    fn allowlisted_reads_pass_the_gate() {
        let _calls = claim_call_budget();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = public_state();
        let reads = [
            ("aether_status", json!([])),
            ("aether_recentBlocks", json!([])),
            ("aether_candidates", json!([])),
            ("aether_proverStatus", json!([])),
            ("aether_getBlock", json!([0])),
            ("aether_getReceipt", json!(["0x0000000000000000000000000000000000000000000000000000000000000001"])),
            ("aether_getAccount", json!(["0x0000000000000000000000000000000000000001"])),
            ("aether_getFinalized", json!([0])),
            ("aether_history", json!([])),
            ("aether_historyProof", json!([0, 0])),
            ("aether_eraInfo", json!([0])),
            ("aether_eraProof", json!([0, 0])),
            ("aether_rewards", json!(["0x0000000000000000000000000000000000000001"])),
            ("aether_accountHistory", json!(["0x0000000000000000000000000000000000000001"])),
            ("eth_blockNumber", json!([])),
            ("eth_call", json!([{ "to": "0x0000000000000000000000000000000000000001", "data": "0x" }])),
            ("eth_getLogs", json!([{}])),
            ("eastsea_status", json!([])),
        ];
        for (method, params) in reads {
            let answer = rt.block_on(call(&st, method, params));
            assert!(!gate_error(&answer), "{method} is an allowlisted read, the gateway must let it through: {answer}");
        }
        // And the reads that must actually answer something on a genesis chain.
        assert_eq!(rt.block_on(call(&st, "aether_status", json!([])))["result"]["chain_id"], 7781);
        assert_eq!(rt.block_on(call(&st, "eth_blockNumber", json!([])))["result"], "0x0");
        assert!(rt.block_on(call(&st, "eth_getLogs", json!([{}])))["result"].is_array());
        // Off the gateway the same writes keep working (aether_status is not
        // the check; a refused method must still be reachable privately).
        let private = bare_state();
        let answer = rt.block_on(call(&private, "aether_network", json!([])));
        assert!(answer["error"].is_null(), "privately the gate stays out of the way: {answer}");
    }

    /// The caps the request states about itself are enforced at the gate, not
    /// silently clamped: the asker learns the number.
    #[test]
    fn caps_are_enforced() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = public_state();
        let addr = json!(["0x0000000000000000000000000000000000000001"]);
        let over = [
            // A 8,192-block eth_getLogs window (the cap is 2,000).
            ("eth_getLogs", json!([{ "fromBlock": "0x0", "toBlock": "0x2000" }])),
            ("eth_call", json!([{ "to": "0x0000000000000000000000000000000000000001", "gas": format!("0x{:x}", PUBLIC_CALL_GAS + 1) }])),
            ("aether_rewards", json!([addr, PUBLIC_REWARDS_LIMIT + 1])),
            ("aether_accountHistory", json!([addr, null, PUBLIC_HISTORY_LIMIT + 1])),
        ];
        for (method, params) in over {
            let answer = rt.block_on(call(&st, method, params));
            assert_eq!(answer["error"]["code"], -32002, "{method} cap must be a clear error: {answer}");
            assert!(gate_error(&answer), "{method} cap error must name the gateway: {answer}");
        }
        // The explorer's own shape passes: a 1,999-block window under the cap.
        let answer = rt.block_on(call(&st, "eth_getLogs", json!([{ "fromBlock": "0x0", "toBlock": "0x7cf" }])));
        assert!(!gate_error(&answer), "a 1,999-block window is the cap the node scans: {answer}");
        // Batches: capped at PUBLIC_MAX_BATCH calls, and shaped like JSON-RPC.
        let one = json!({ "jsonrpc": "2.0", "id": 1, "method": "aether_status", "params": [] });
        let answer = rt.block_on(handle_value(&st, Value::Array(vec![one.clone(); PUBLIC_MAX_BATCH])));
        assert!(answer.as_array().is_some_and(|a| a.len() == PUBLIC_MAX_BATCH), "8 calls answer one by one: {answer}");
        let answer = rt.block_on(handle_value(&st, Value::Array(vec![one; PUBLIC_MAX_BATCH + 1])));
        assert_eq!(answer["error"]["code"], -32002);
        let answer = rt.block_on(handle_value(&st, json!([])));
        assert_eq!(answer["error"]["code"], -32600, "an empty batch is invalid JSON-RPC");
        // Privately a large batch is nobody's business but the caller's.
        let answer = rt.block_on(handle_value(&bare_state(), Value::Array(vec![json!({ "id": 1, "method": "aether_status", "params": [] }); 20])));
        assert!(answer.as_array().is_some_and(|a| a.len() == 20), "the batch cap is a public-gateway cap");
    }

    /// An inverted or wholly-future eth_getLogs range is answered, never fed
    /// to the block range scan: BTreeMap::range with from > to aborts the
    /// process, which the fatal-panic watch turns into a node exit (pre-audit
    /// 7 PA7-02). The old code panicked on both shapes here.
    #[test]
    fn reversed_and_future_getlogs_ranges_error_or_empty_never_panic() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        // A genesis chain: head is 0, so any height above it is the future.
        let st = bare_state();
        let inverted = rt.block_on(call(&st, "eth_getLogs", json!([{ "fromBlock": "0x64", "toBlock": "0x32" }])));
        assert_eq!(inverted["error"]["code"], -32602, "an inverted range is a clear error: {inverted}");
        let future = rt.block_on(call(&st, "eth_getLogs", json!([{ "fromBlock": "0x3e8", "toBlock": "0x7d0" }])));
        assert!(future["result"].as_array().is_some_and(|a| a.is_empty()), "a wholly-future range is an empty answer, not a panic: {future}");
        // fromBlock above head with toBlock defaulted to head: still an error,
        // never a silent clamp-to-empty.
        let ahead = rt.block_on(call(&st, "eth_getLogs", json!([{ "fromBlock": "0x3e8" }])));
        assert_eq!(ahead["error"]["code"], -32602, "{ahead}");
        // The public gateway refuses the inverted ask at the gate too, with
        // the same shape it demands of every other request.
        let pub_st = public_state();
        let gate = rt.block_on(call(&pub_st, "eth_getLogs", json!([{ "fromBlock": "0x64", "toBlock": "0x32" }])));
        assert_eq!(gate["error"]["code"], -32602, "{gate}");
        assert!(gate_error(&gate), "{gate}");
    }

    /// A public history read never fetches an era from upstream (pre-audit 7
    /// PA7-04): old_block's self-healing fetch calls fetch_into, which SAVES
    /// the whole era file into this node's store — a stranger asking for
    /// pruned heights would undo the pruning one era at a time. On the
    /// gateway the answer is the local-only pruned error. The old code
    /// fetched (here: connection refused to a dead upstream → -32000).
    #[test]
    fn public_reads_never_fetch_history_from_upstream() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut st = public_state();
        // A pruned height whose era this node does not hold, and an upstream
        // that cannot answer (port 9 discards): if the fetch ran at all, the
        // call would fail with the upstream error instead of "pruned".
        st.upstream = Some(Arc::new(crate::follow::Upstream::Http(vec!["http://127.0.0.1:9".into()])));
        st.chain.lock().pruned_below = aether_state::mmr::ERA_LEN * 3;
        let answer = rt.block_on(call(&st, "aether_getBlock", json!([100])));
        // The code is the claim: -32001 is old_block's local refusal ("no
        // store" on this bare chain, "pruned" on a real one); a fetch that ran
        // against the dead upstream would surface -32000 instead.
        assert_eq!(answer["error"]["code"], -32001, "the public answer is the local refusal, not an upstream fetch error: {answer}");
        // Privately the same ask still self-heals (the fetch runs; a dead
        // upstream surfaces its own error, never a fake "pruned" answer).
        let mut private = bare_state();
        private.upstream = Some(Arc::new(crate::follow::Upstream::Http(vec!["http://127.0.0.1:9".into()])));
        private.chain.lock().pruned_below = aether_state::mmr::ERA_LEN * 3;
        let answer = rt.block_on(call(&private, "aether_getBlock", json!([100])));
        assert_eq!(answer["error"]["code"], -32000, "privately the fetch runs and its failure is the upstream's, not a pruned refusal: {answer}");
    }

    /// Simultaneous public eth_calls are bounded (pre-audit 7 PA7-05): the
    /// gas cap bounds one call's EVM work; the execution budget bounds how
    /// many run at once. With the budget deliberately full, a public call is
    /// refused with a busy error — the old code had no such bound: every
    /// overlapping call deep-copied the whole WorldState under the chain
    /// mutex, and only their individual gas said anything.
    #[test]
    fn public_eth_call_execution_is_capped() {
        let _calls = claim_call_budget();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = public_state();
        let ask = json!([{ "to": "0x0000000000000000000000000000000000000001", "data": "0x" }]);
        // The budget already exhausted by MAX_PUBLIC_CALLS in-flight calls.
        PUBLIC_CALLS.store(MAX_PUBLIC_CALLS, Ordering::Release);
        let answer = rt.block_on(call(&st, "eth_call", ask.clone()));
        assert_eq!(answer["error"]["code"], -32002, "{answer}");
        assert!(gate_error(&answer), "the busy refusal names the gateway: {answer}");
        // Privately the same ask never waits on the public budget.
        let private = bare_state();
        let answer = rt.block_on(call(&private, "eth_call", ask.clone()));
        assert!(answer["error"].is_null(), "private calls are not budgeted: {answer}");
        // With the budget free again the public call goes through.
        PUBLIC_CALLS.store(0, Ordering::Release);
        let answer = rt.block_on(call(&st, "eth_call", ask));
        assert!(answer["error"].is_null(), "a budgeted public call answers once a slot is free: {answer}");
    }

    /// A large finalized state answers public calls from the SHARED snapshot
    /// (pre-audit 7 PA7-05): the call path takes an Arc clone under the
    /// mutex, so preparation cost no longer grows with the state — and the
    /// answer is still correct against the state the calls share.
    #[test]
    fn a_large_state_answers_calls_from_the_shared_finalized_snapshot() {
        let _calls = claim_call_budget();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = public_state();
        const ACCOUNTS: u32 = 20_000;
        {
            let mut g = st.chain.lock();
            let exec = std::sync::Arc::make_mut(&mut g.finalized);
            for i in 0..ACCOUNTS {
                let mut b = [0u8; 20];
                b[0..4].copy_from_slice(&i.to_be_bytes());
                exec.state.set_balance(aether_types::Address::new(b), U256::from(i)).unwrap();
            }
        }
        // A call whose answer depends on the big state: sending value from a
        // funded account reads its balance out of the shared snapshot.
        let mut b = [0u8; 20];
        b[0..4].copy_from_slice(&7_777u32.to_be_bytes());
        let funded_addr = aether_types::Address::new(b);
        let funded = format!("{funded_addr:#x}");
        let ask = json!([{ "from": funded, "to": "0x0000000000000000000000000000000000000001", "value": "0x1" }]);
        let answer = rt.block_on(call(&st, "eth_call", ask));
        assert!(answer["error"].is_null(), "a large shared state still answers: {answer}");
        // And a read of the same shared snapshot the call executed against:
        // eth_getBalance is not on the public allowlist, so the identical
        // chain is read through a private-mode clone of the same RpcState.
        let mut private = st.clone();
        private.public_read_only = false;
        assert_eq!(
            rt.block_on(call(&private, "eth_getBalance", json!([funded])))["result"],
            json!(format!("0x{:x}", 7_777)),
        );
    }

    /// The gateway binds loopback only; exposure is a tunnel's job. This is
    /// checked at bind time, not documented and hoped for.
    #[test]
    fn public_serve_refuses_non_loopback_binds() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = public_state();
        for addr in ["0.0.0.0:18545", "192.0.2.10:18545", "[::]:18545"] {
            let err = rt.block_on(serve(addr.parse().unwrap(), st.clone())).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{addr}: {err}");
        }
        // Loopback is exactly what the flag serves.
        let loopback = rt.spawn(serve("127.0.0.1:0".parse().unwrap(), st));
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(50)).await });
        assert!(!loopback.is_finished(), "loopback bind must be allowed");
        loopback.abort();
    }

    /// One public request may make this node parse at most PUBLIC_MAX_BODY
    /// (axum's default is 2 MiB; the gateway is tighter).
    #[test]
    fn public_serve_caps_the_request_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let st = public_state();
        let addr = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let a = probe.local_addr().unwrap();
            drop(probe);
            a
        };
        rt.spawn(serve(addr, st));
        rt.block_on(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let sock = tokio::net::TcpStream::connect(addr).await.expect("gateway is listening");
            let (mut reader, mut writer) = sock.into_split();
            let body = format!("{{\"padding\":\"{}\"}}", "a".repeat(PUBLIC_MAX_BODY));
            let head = format!(
                "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            writer.write_all(head.as_bytes()).await.unwrap();
            // Chunked writes so the server gets to answer while we are sending.
            let sending = tokio::spawn(async move {
                for chunk in body.as_bytes().chunks(64 * 1024) {
                    if writer.write_all(chunk).await.is_err() {
                        break;
                    }
                }
            });
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while std::time::Instant::now() < deadline && !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match tokio::time::timeout(Duration::from_secs(5), reader.read(&mut buf)).await {
                    Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                    Ok(Ok(n)) => seen.extend_from_slice(&buf[..n]),
                }
            }
            sending.abort();
            let status = String::from_utf8_lossy(&seen);
            let first_line = status.lines().next().unwrap_or_default();
            assert!(first_line.contains("413") || first_line.contains("400"), "a body over PUBLIC_MAX_BODY must be refused, got: {first_line}");
        });
    }

    /// The public gateway never serves era files (pre-audit 7 PA7-03): `/era/`
    /// is a webseed's route, for the node's own private listener. On the
    /// gateway it is not registered at all, so no stranger's GET can make
    /// this node buffer and ship a whole era file; privately the same GET
    /// still serves the bytes, proving the refusal is the gateway's, not the
    /// store being empty. The old code registered the route in public mode
    /// and served the bytes.
    #[test]
    fn public_gateway_serves_no_era_files() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let dir = std::env::temp_dir().join(format!("aether-rpc-era-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = crate::store::Store::open(&dir.join("db.redb")).unwrap();
        std::fs::create_dir_all(store.era_dir()).unwrap();
        let era: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();
        std::fs::write(store.era_dir().join("era-00000003.aera"), &era).unwrap();
        let (chain, _) = crate::chain::Chain::open(crate::chain::ChainConfig {
            chain_id: 7781,
            limits: aether_types::GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc: vec![], fees: false, registrar: None, epoch_blocks: 0,
            min_streak: None, draw_epochs: None, history_v2: false, protocol: 1,
            node_rewards: false, committee: vec![], reserve: None, group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        }, store).unwrap();
        let state = |public: bool| {
            let (gossip, _) = mpsc::unbounded_channel();
            RpcState {
                chain: chain.clone(),
                finality: Finality::Archive(Arc::new(crate::follow::FinalityArchive::default())),
                gossip,
                faucet: None, registrar: None, network: None, upstream: None,
                handoff: None, snapshot: Default::default(), prover: None,
                shards: None, public_read_only: public,
            }
        };
        let free = || {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let a = probe.local_addr().unwrap();
            drop(probe);
            a
        };
        let (pub_addr, priv_addr) = (free(), free());
        rt.spawn(serve(pub_addr, state(true)));
        rt.spawn(serve(priv_addr, state(false)));
        let ask = |addr: std::net::SocketAddr| async move {
            let mut sock = tokio::net::TcpStream::connect(addr).await.expect("listener is up");
            sock.write_all(b"GET /era/era-00000003.aera HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.unwrap();
            let mut buf = Vec::new();
            sock.read_to_end(&mut buf).await.unwrap();
            buf
        };
        rt.block_on(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let public = ask(pub_addr).await;
            let status = String::from_utf8_lossy(&public).lines().next().unwrap_or_default().to_string();
            assert!(status.contains("404"), "the public gateway must not serve era files: {status}");
            assert!(!public.windows(64).any(|w| w == &era[..64]), "no era bytes in the public answer");
            let private = ask(priv_addr).await;
            let text = String::from_utf8_lossy(&private);
            assert!(text.lines().next().unwrap_or_default().contains("200"), "the private webseed still serves it: {text}");
            assert_eq!(&private[private.len() - era.len()..], &era[..], "the private answer carries the exact era bytes");
        });
    }
}

/// A block proof: validators verify it and keep it for their proposals;
/// followers pass it on to a validator.
async fn submit_proof(st: &RpcState, p: &Value) -> Result<Value, (i64, String)> {
    // Size first, before copying anything.
    if p.pointer("/0/proof").and_then(Value::as_str).is_none_or(|s| s.len() > 2 * crate::chain::MAX_PROOF_BYTES) {
        return Err((-32602, "proof missing or too large".into()));
    }
    let claim: aether_light::block::ProofClaim = serde_json::from_value(p.get(0).cloned().unwrap_or_default()).map_err(|e| (-32602, format!("proof: {e}")))?;
    if let Some(up) = &st.upstream {
        return up.first("aether_submitProof", json!([claim])).await.map_err(|e| (-32000, e));
    }
    let chain = st.chain.clone();
    let height = claim.height;
    tokio::task::spawn_blocking(move || chain.add_proof(claim)).await.map_err(|e| (-32000, e.to_string()))?.map_err(|e| (-32000, e))?;
    Ok(json!({ "accepted": height }))
}

fn hex_arg(v: &Value, k: &str) -> Result<Option<Vec<u8>>, (i64, String)> {
    match v.get(k).and_then(Value::as_str) {
        None => Ok(None),
        Some(s) => hex::decode(s.trim_start_matches("0x")).map(Some).map_err(|_| (-32602, format!("{k} is not hex"))),
    }
}

/// Gas a private `eth_call` may run: the block gas limit.
const PRIVATE_CALL_GAS: u64 = 1 << 24;

/// Concurrent public eth_call executions (pre-audit 7 PA7-05): the gas cap
/// bounds one call's EVM work; this bounds how many strangers' calls execute
/// at once, so overlapping calls cannot stack revm instances on the machine
/// whatever each one's gas says.
const MAX_PUBLIC_CALLS: usize = 4;
static PUBLIC_CALLS: AtomicUsize = AtomicUsize::new(0);

/// eth_call on the finalized state (no fees, nothing committed).
/// The state is the SHARED immutable snapshot (an Arc clone under the
/// mutex, never a deep copy of the tree — a call does not mutate it), and
/// public execution is both budgeted and moved off the async runtime
/// (pre-audit 7 PA7-05: the old path copied the whole WorldState under the
/// chain mutex for every call, cheap EVM or not).
async fn eth_call(st: &RpcState, p: &Value, gas: u64) -> RpcResult {
    let c = p.get(0).ok_or((-32602, "missing call object".to_string()))?;
    let addr = |k: &str| -> Result<Option<Address>, (i64, String)> {
        c.get(k).and_then(Value::as_str).map(|s| s.parse().map_err(|_| (-32602, format!("{k} is not an address")))).transpose()
    };
    let to = addr("to")?;
    let from = addr("from")?.unwrap_or(Address::ZERO);
    let data = hex_arg(c, "data")?.or(hex_arg(c, "input")?).unwrap_or_default();
    let value = match c.get("value").and_then(Value::as_str) {
        Some(v) => U256::from_str_radix(v.trim_start_matches("0x"), 16).map_err(|_| (-32602, "value".to_string()))?,
        None => U256::ZERO,
    };
    // The finalized snapshot by Arc, and the small config — the lock is held
    // for two pointer-ish clones, not a walk of the state tree.
    let (exec, cfg) = {
        let g = st.chain.lock();
        (g.finalized.clone(), g.cfg.clone())
    };
    let ctx = aether_execution::BlockContext {
        chain_id: cfg.chain_id,
        number: exec.height + 1,
        timestamp: exec.timestamp / 1000,
        beneficiary: Address::ZERO,
        limits: cfg.limits,
        fees: None,
    };
    // The budget is held for the whole execution (released on drop with the
    // blocking task); a stranger arriving while it is full is told to retry.
    let _budget = if st.public_read_only {
        BudgetSlot::acquire(&PUBLIC_CALLS, MAX_PUBLIC_CALLS)
            .ok_or((-32002, "public read-only gateway: too many concurrent eth_call executions; retry shortly".to_string()))?
    } else {
        BudgetSlot::acquire(&PUBLIC_CALLS, usize::MAX).expect("usize::MAX budget never refuses")
    };
    let r = tokio::task::spawn_blocking(move || {
        aether_execution::call(&exec.state, &ctx, from, to, data.into(), value, gas)
    })
    .await
    .map_err(|e| (-32000, e.to_string()))?
    .map_err(|e| (-32000, e))?;
    if r.success {
        Ok(json!(format!("0x{}", hex::encode(&r.output))))
    } else {
        Err((3, format!("execution reverted: 0x{}", hex::encode(&r.output))))
    }
}

/// An `eth_getLogs` block parameter: "latest" or absent → `default`,
/// "earliest" → 0, otherwise a hex quantity.
fn block_param(f: &Value, k: &str, default: u64) -> u64 {
    match f.get(k).and_then(Value::as_str) {
        Some("latest") | None => default,
        Some("earliest") => 0,
        Some(s) => u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(default),
    }
}

/// eth_getLogs over at most 2,000 finalized blocks (address and topic filters).
fn eth_get_logs(chain: &Chain, p: &Value) -> RpcResult {
    let f = p.get(0).cloned().unwrap_or_default();
    let g = chain.lock();
    let head = g.finalized.height;
    // The range as the request states it, before any clamping. An inverted ask
    // is a malformed request answered with an error — never handed to the
    // BTreeMap range below, whose inverted bounds abort the process (the
    // fatal-panic supervisor path turns that into a node restart, pre-audit 7
    // PA7-02).
    let asked_to = block_param(&f, "toBlock", head);
    let asked_from = block_param(&f, "fromBlock", head);
    if asked_from > asked_to {
        return Err((-32602, format!("eth_getLogs: fromBlock {asked_from} is above toBlock {asked_to}")));
    }
    let to = asked_to.min(head);
    let from = asked_from.max(to.saturating_sub(1999));
    if from > to {
        // asked_from <= asked_to and `to` clamped below asked_from: the whole
        // ask sits above this node's head. No block exists there yet, so the
        // complete answer is empty — not an error, and never a panic.
        return Ok(json!([]));
    }
    let addrs: Vec<String> = match f.get("address") {
        Some(Value::String(a)) => vec![a.to_lowercase()],
        Some(Value::Array(v)) => v.iter().filter_map(Value::as_str).map(str::to_lowercase).collect(),
        _ => vec![],
    };
    let topics: Vec<Vec<String>> = f
        .get("topics")
        .and_then(Value::as_array)
        .map(|t| {
            t.iter()
                .map(|x| match x {
                    Value::String(s) => vec![s.to_lowercase()],
                    Value::Array(v) => v.iter().filter_map(Value::as_str).map(str::to_lowercase).collect(),
                    _ => vec![],
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for (height, b) in g.blocks.range(from..=to) {
        let mut index = 0u64;
        for (ti, h) in b.txs.iter().enumerate() {
            let Some((_, r)) = g.receipts.get(h) else { continue };
            for e in &r.events {
                let log_index = index;
                index += 1;
                let address = format!("{:#x}", e.address);
                if !addrs.is_empty() && !addrs.contains(&address) {
                    continue;
                }
                let topic_ok = topics.iter().enumerate().all(|(i, want)| want.is_empty() || e.topics.get(i).is_some_and(|t| want.contains(&format!("{t:#x}"))));
                if !topic_ok {
                    continue;
                }
                out.push(json!({
                    "address": address,
                    "topics": e.topics.iter().map(|t| format!("{t:#x}")).collect::<Vec<_>>(),
                    "data": format!("0x{}", hex::encode(&e.data)),
                    "blockNumber": format!("0x{height:x}"),
                    "blockHash": format!("0x{}", b.hash),
                    "transactionHash": format!("{h:#x}"),
                    "transactionIndex": format!("0x{ti:x}"),
                    "logIndex": format!("0x{log_index:x}"),
                    "removed": false,
                }));
            }
        }
    }
    Ok(json!(out))
}
