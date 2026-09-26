//! Follower node (docs/design/12-launch-plan.md P1): any Mac runs the full
//! chain without being a validator.
//!
//! It pulls each finalized block and its finality certificate from validators,
//! checks the certificate against the committee key, re-executes the block
//! (state root, block access list and gas must match, as a validator checks
//! them), and persists the result. Wallets on this Mac then ask it instead of a
//! remote node; transactions they submit are forwarded upstream. Nothing a
//! validator sends is trusted beyond the certificate.

use crate::block::Block;
use crate::chain::Chain;
use aether_light::{from_hex, verify_finalized, ValidatorSet, MAX_BLOCK_BYTES};
use commonware_codec::Decode as _;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, warn};

/// Certified blocks this follower verified, served to wallets as `aether_getFinalized`.
#[derive(Default)]
pub struct FinalityArchive {
    inner: Mutex<BTreeMap<u64, (String, String)>>,
}

/// Recent heights kept (wallets anchor on the latest ones).
const KEEP: usize = 8_192;

impl FinalityArchive {
    pub fn insert(&self, height: u64, block_hex: String, finalization_hex: String) {
        let mut g = self.inner.lock().expect("archive lock");
        g.insert(height, (block_hex, finalization_hex));
        while g.len() > KEEP {
            g.pop_first();
        }
    }

    pub fn get(&self, height: u64) -> Option<(String, String)> {
        self.inner.lock().expect("archive lock").get(&height).cloned()
    }
}

/// Where certified blocks come from: validators' RPC over HTTP (local networks)
/// or over iroh (found by node id on the Mainline DHT).
pub enum Upstream {
    Http(Vec<String>),
    Iroh(aether_net::RpcClient),
}

impl Upstream {
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Upstream::Iroh(c) => c.call(method, params).await.map_err(|e| e.to_string()),
            Upstream::Http(urls) => {
                let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
                let client = reqwest::Client::new();
                let mut last = String::from("no upstream");
                for url in urls {
                    match client.post(url).json(&body).timeout(Duration::from_secs(5)).send().await {
                        Ok(r) => match r.json::<Value>().await {
                            Ok(v) if v.get("error").is_some() => last = v["error"].to_string(),
                            Ok(v) => return Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                            Err(e) => last = e.to_string(),
                        },
                        Err(e) => last = e.to_string(),
                    }
                }
                Err(last)
            }
        }
    }
}

/// Follow the chain forever: verify, execute and persist each next block.
pub async fn run(chain: Chain, upstream: std::sync::Arc<Upstream>, set: ValidatorSet, archive: std::sync::Arc<FinalityArchive>) {
    let mut last_log = 0;
    loop {
        let next = chain.finalized_height() + 1;
        match fetch(&upstream, &set, next).await {
            Ok(Some((block, block_hex, fin_hex))) => match chain.finalize(&block) {
                Ok(()) => {
                    archive.insert(next, block_hex, fin_hex);
                    if next - last_log >= 100 || next % 10 == 0 {
                        info!(height = next, root = %chain.lock().finalized.state.root(), "followed");
                        last_log = next;
                    }
                    continue;
                }
                Err(e) => warn!(height = next, ?e, "certified block did not execute to the same result; not adopting it"),
            },
            Ok(None) => {}
            Err(e) => warn!(height = next, %e, "upstream"),
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

/// Block `h` and its certificate, verified; `None` if not finalized yet.
async fn fetch(upstream: &Upstream, set: &ValidatorSet, h: u64) -> Result<Option<(Block, String, String)>, String> {
    let v = upstream.call("aether_getFinalized", json!([h])).await?;
    if v.is_null() {
        return Ok(None);
    }
    let (bh, fh) = (v["block"].as_str().unwrap_or_default().to_string(), v["finalization"].as_str().unwrap_or_default().to_string());
    let (bb, fb) = (from_hex(&bh).map_err(|e| e.to_string())?, from_hex(&fh).map_err(|e| e.to_string())?);
    let verified = verify_finalized(set, &bb, &fb).map_err(|e| format!("certificate: {e:?}"))?;
    if verified.height != h {
        return Err(format!("asked for block {h}, got a certificate for {}", verified.height));
    }
    let block = Block::decode_cfg(bb.as_slice(), &Block::codec_config(MAX_BLOCK_BYTES)).map_err(|e| format!("block: {e}"))?;
    Ok(Some((block, bh, fh)))
}

/// Forward txs submitted to this follower to the validators.
pub async fn forward(upstream: std::sync::Arc<Upstream>, mut rx: tokio::sync::mpsc::UnboundedReceiver<aether_types::TxEnvelope>) {
    while let Some(tx) = rx.recv().await {
        if let Err(e) = upstream.call("aether_sendTransaction", json!([tx])).await {
            warn!(%e, "could not forward a transaction upstream");
        }
    }
}
