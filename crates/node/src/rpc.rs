//! JSON-RPC 2.0 over HTTP (POST /). Serves the finalized state.
//!
//! `aether_getAccount` returns an EIP-7864 Merkle proof so clients can verify
//! balances against the state root instead of trusting this server.

use crate::chain::Chain;
use aether_execution::validate_stateless;
use aether_state::layout::{basic_data_key, code_hash_key, storage_slot_key};
use aether_state::StateRepository;
use aether_types::{Address, TxEnvelope, TxHash, U256};
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use std::net::SocketAddr;
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
}

/// The snapshot being served: (height, serialized bytes).
pub type SnapshotCache = std::sync::Arc<std::sync::Mutex<Option<(u64, std::sync::Arc<Vec<u8>>)>>>;

/// Bytes per snapshot chunk (well under transport message limits).
pub const SNAPSHOT_CHUNK: usize = 1 << 20;

/// The snapshot of the finalized state, rebuilt at most once per height.
fn cached_snapshot(st: &RpcState) -> (u64, std::sync::Arc<Vec<u8>>) {
    let height = st.chain.finalized_height();
    let mut cache = st.snapshot.lock().expect("snapshot cache");
    if let Some((h, b)) = cache.as_ref() {
        // Keep serving one snapshot for a while so a download can finish.
        if *h + 120 >= height {
            return (*h, b.clone());
        }
    }
    let s = crate::snapshot::Snapshot::of(&st.chain);
    let fresh = (s.summary.height, std::sync::Arc::new(s.to_bytes()));
    *cache = Some(fresh.clone());
    fresh
}

pub fn blake3_hex(b: &[u8]) -> String {
    blake3::hash(b).to_hex().to_string()
}

pub async fn serve(addr: SocketAddr, state: RpcState) -> std::io::Result<()> {
    // Loopback only; any origin may ask (web pages and dApps read through this
    // node; every write still needs the user's signature in the wallet).
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([axum::http::Method::POST, axum::http::Method::OPTIONS])
        .allow_headers([axum::http::header::CONTENT_TYPE]);
    let app = Router::new().route("/", post(handle)).layer(cors).with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await
}

async fn handle(State(st): State<RpcState>, Json(req): Json<Value>) -> Json<Value> {
    Json(handle_value(&st, req).await)
}

/// Transport-independent JSON-RPC handling (HTTP on loopback, iroh QUIC publicly).
pub async fn handle_value(st: &RpcState, req: Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = req.get("params").cloned().unwrap_or(Value::Array(vec![]));
    let result = match method.as_str() {
        "aether_getFinalized" => finalized(st, &params).await,
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
        let era = h / aether_state::mmr::ERA_LEN;
        crate::era_net::fetch_into(&chain, up, era).await.map_err(|e| (-32000, e))?;
        read = tokio::task::spawn_blocking(move || chain.old_block(h)).await.map_err(|e| (-32000, e.to_string()))?;
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
            let g = chain.lock();
            let f = &g.finalized;
            let base = Chain::next_base_fee(&g.cfg, f);
            // Catching up (a node that slept, or one still starting): the app
            // shows this instead of a height that looks stale.
            let behind = g.net_height.map_or(0, |n| n.saturating_sub(f.height));
            Ok(json!({
                // Base fees (wei per unit) for the next block: the exec base is burned, prove goes to the prover escrow.
                "base_fee": { "exec": base.exec.to_string(), "prove": base.prove.to_string() },
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
                "node_protocol": crate::upgrade::PROTOCOL,
                "newest_scheduled": f.schedule.iter().map(|a| a.protocol).max().unwrap_or(1),
                // Activations on chain as (protocol, at-height) pairs: a genesis
                // above protocol 1 carries its own at height 0, so `[3, 0]` here
                // is how a rehearsal knows the rules were on from the start.
                "schedule": f.schedule.iter().map(|a| json!([a.protocol, a.at])).collect::<Vec<_>>(),
                "upcoming_upgrades": g.upgrade_notices,
                // The free registration lane (G2): wallets see it and register
                // without needing a balance for a paid contract call.
                "free_registration": aether_rewards::enabled(&f.state),
                // This node's resource state (docs/ops/resource-limits.md):
                // the disk guard the app shows as "디스크 공간 부족".
                "resources": crate::resources::monitor().map(|m| m.status_value()).unwrap_or(Value::Null),
            }))
        }
        // The next relay nonce a free-lane registration of `operator` must
        // carry (`[operator]`): the count the chain has spent of its items.
        "aether_registrationNonce" => {
            let a: Address = param(p, 0)?;
            let g = chain.lock();
            Ok(json!(aether_execution::registry::lane_nonce(&g.finalized.state, &a)))
        }
        // The voting set proposed for this registry epoch (while no handoff is
        // pending): `aether run` on old and new members reshares to it in the background.
        "aether_rotation" => {
            let g = chain.lock();
            let f = g.finalized.clone();
            let params = aether_execution::registry::params(&f.state);
            let Some((draw, members)) = g.proposal.clone() else { return Ok(Value::Null) };
            let pending = f.handoff.as_ref().is_some_and(|p| f.height < p.switch);
            if draw != f.height / (params.epoch_blocks * params.draw_epochs) || pending {
                return Ok(Value::Null);
            }
            let next: Vec<Value> = members.iter().map(|(k, n)| json!({ "key": k, "node": n })).collect();
            Ok(json!({ "epoch": draw, "next": next, "network": st.network }))
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
        "aether_snapshot" => {
            let (height, bytes) = cached_snapshot(st);
            Ok(json!({ "height": height, "size": bytes.len(), "blake3": blake3_hex(&bytes), "chunk": SNAPSHOT_CHUNK }))
        }
        // One chunk of the cached snapshot at `height` (hex); the whole is checked by its BLAKE3.
        "aether_snapshotChunk" => {
            let height: u64 = param(p, 0)?;
            let index: usize = param(p, 1)?;
            let (h, bytes) = cached_snapshot(st);
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
                v["last_reward"] = status.payout.and_then(|a| chain.rewards(&a).last().and_then(|r| r.get("amount").cloned())).unwrap_or(Value::Null);
                v
            }
            None => json!({ "running": false }),
        }),
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
            let limit = p.get(1).and_then(Value::as_u64).unwrap_or(1_000).min(10_000) as usize;
            Ok(json!(chain.recent_rewards(&a, limit)))
        }
        "aether_accountHistory" => {
            let address: Address = param(p, 0)?;
            let cursor = p.get(1).filter(|v| !v.is_null()).map(|v| v.as_str().ok_or((-32602, "cursor must be a string".to_string()))).transpose()?;
            let limit = p.get(2).filter(|v| !v.is_null()).map(|v| v.as_u64().ok_or((-32602, "limit must be a positive integer".to_string()))).transpose()?.unwrap_or(50);
            if !(1..=200).contains(&limit) { return Err((-32602, "limit must be 1..200".into())); }
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
        "eth_call" => eth_call(chain, p),
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

/// eth_call on the finalized state (no fees, nothing committed).
fn eth_call(chain: &Chain, p: &Value) -> RpcResult {
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
    let (state, ctx) = {
        let g = chain.lock();
        let f = &g.finalized;
        let ctx = aether_execution::BlockContext {
            chain_id: g.cfg.chain_id,
            number: f.height + 1,
            timestamp: f.timestamp / 1000,
            beneficiary: Address::ZERO,
            limits: g.cfg.limits,
            fees: None,
        };
        (f.state.clone(), ctx)
    };
    let r = aether_execution::call(&state, &ctx, from, to, data.into(), value, 1 << 24).map_err(|e| (-32000, e))?;
    if r.success {
        Ok(json!(format!("0x{}", hex::encode(&r.output))))
    } else {
        Err((3, format!("execution reverted: 0x{}", hex::encode(&r.output))))
    }
}

/// eth_getLogs over at most 2,000 finalized blocks (address and topic filters).
fn eth_get_logs(chain: &Chain, p: &Value) -> RpcResult {
    let f = p.get(0).cloned().unwrap_or_default();
    let g = chain.lock();
    let head = g.finalized.height;
    let num = |k: &str, d: u64| -> u64 {
        match f.get(k).and_then(Value::as_str) {
            Some("latest") | None => d,
            Some("earliest") => 0,
            Some(s) => u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(d),
        }
    };
    let to = num("toBlock", head).min(head);
    let from = num("fromBlock", head).max(to.saturating_sub(1999));
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
