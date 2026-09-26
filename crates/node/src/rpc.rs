//! JSON-RPC 2.0 over HTTP (POST /). Serves the finalized state.
//!
//! `aether_getAccount` returns an EIP-7864 Merkle proof so clients can verify
//! balances against the state root instead of trusting this server.

use crate::chain::Chain;
use aether_execution::validate_stateless;
use aether_state::layout::{basic_data_key, storage_slot_key};
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
}

pub async fn serve(addr: SocketAddr, state: RpcState) -> std::io::Result<()> {
    let app = Router::new().route("/", post(handle)).with_state(state);
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
    let result = if method == "aether_getFinalized" { finalized(st, &params).await } else { dispatch(st, &method, &params) };
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
    let marshal = match &st.finality {
        Finality::Marshal(m) => m,
        Finality::Archive(a) => {
            return Ok(a.get(h).map_or(Value::Null, |(block, fin)| json!({ "height": h, "block": block, "finalization": fin })));
        }
    };
    let (Some(block), Some(fin)) = (marshal.get_block(Height::new(h)).await, marshal.get_finalization(Height::new(h)).await) else {
        return Ok(Value::Null);
    };
    Ok(json!({
        "height": h,
        "block": aether_light::to_hex(&block.encode()),
        "finalization": aether_light::to_hex(&fin.encode()),
    }))
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
                "hash_function": "poseidon2-koalabear-16",
            }))
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
