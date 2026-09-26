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

/// Largest upstream response accepted (a block and its certificate, hex encoded).
const MAX_RESPONSE: usize = 4 * MAX_BLOCK_BYTES as usize + (1 << 20);

/// Where certified blocks come from: validators' RPC over HTTP (local networks)
/// or over iroh (found by node id on the Mainline DHT).
pub enum Upstream {
    Http(Vec<String>),
    /// The client, and how many answers in a row gave nothing new.
    Iroh(aether_net::RpcClient, std::sync::atomic::AtomicU32),
}

impl Upstream {
    /// Ask each source in turn until `accept` takes an answer.
    async fn ask<T>(&self, method: &str, params: Value, accept: impl Fn(Value) -> Result<Option<T>, String>) -> Result<Option<T>, String> {
        match self {
            Upstream::Iroh(c, misses) => {
                use std::sync::atomic::Ordering::Relaxed;
                let answer = c.call(method, params).await.map_err(|e| e.to_string()).and_then(&accept);
                // Switch validators when one serves data that does not verify, or has
                // had nothing new for a while (it may be lagging behind the others).
                let stale = match &answer {
                    Ok(Some(_)) => {
                        misses.store(0, Relaxed);
                        false
                    }
                    Ok(None) => misses.fetch_add(1, Relaxed) + 1 >= 10,
                    Err(_) => true,
                };
                if stale {
                    misses.store(0, Relaxed);
                    c.rotate().await;
                }
                answer
            }
            Upstream::Http(urls) => {
                let mut last = Err(String::from("no upstream"));
                for url in urls {
                    match http_call(url, method, &params).await.and_then(&accept) {
                        Ok(Some(v)) => return Ok(Some(v)),
                        other => last = other,
                    }
                }
                last
            }
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.ask(method, params, |v| Ok(Some(v))).await.map(|v| v.unwrap_or(Value::Null))
    }
}

async fn http_call(url: &str, method: &str, params: &Value) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let mut r = reqwest::Client::new().post(url).json(&body).timeout(Duration::from_secs(10)).send().await.map_err(|e| e.to_string())?;
    if r.content_length().is_some_and(|n| n as usize > MAX_RESPONSE) {
        return Err("response too large".into());
    }
    let mut buf = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
        if buf.len() + chunk.len() > MAX_RESPONSE {
            return Err("response too large".into());
        }
        buf.extend_from_slice(&chunk);
    }
    let v: Value = serde_json::from_slice(&buf).map_err(|e| e.to_string())?;
    match v.get("error") {
        Some(e) => Err(e.to_string()),
        None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
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
                    if next - last_log >= 100 || next.is_multiple_of(10) {
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

/// Block `h` and its certificate, verified; `None` if no source has it yet.
/// A source that answers with nothing or with a bad certificate is skipped.
async fn fetch(upstream: &Upstream, set: &ValidatorSet, h: u64) -> Result<Option<(Block, String, String)>, String> {
    upstream.ask("aether_getFinalized", json!([h]), |v| check(set, h, v)).await
}

fn check(set: &ValidatorSet, h: u64, v: Value) -> Result<Option<(Block, String, String)>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let (bh, fh) = (v["block"].as_str().unwrap_or_default().to_string(), v["finalization"].as_str().unwrap_or_default().to_string());
    let (bb, fb) = (from_hex(&bh).map_err(|e| format!("{e:?}"))?, from_hex(&fh).map_err(|e| format!("{e:?}"))?);
    let verified = verify_finalized(set, &bb, &fb).map_err(|e| format!("certificate: {e:?}"))?;
    if verified.height != h {
        return Err(format!("asked for block {h}, got a certificate for {}", verified.height));
    }
    let block = Block::decode_cfg(bb.as_slice(), &Block::codec_config(MAX_BLOCK_BYTES)).map_err(|e| format!("block: {e}"))?;
    // Keep the canonical encoding, never the upstream's text (which may be padded).
    Ok(Some((block, aether_light::to_hex(&bb), aether_light::to_hex(&fb))))
}

/// Forward txs submitted to this follower to the validators, retrying for a
/// while so a brief outage does not lose them.
pub async fn forward(upstream: std::sync::Arc<Upstream>, mut rx: tokio::sync::mpsc::UnboundedReceiver<aether_types::TxEnvelope>) {
    while let Some(tx) = rx.recv().await {
        let up = upstream.clone();
        tokio::spawn(async move {
            let mut wait = Duration::from_secs(1);
            for attempt in 1..=6 {
                match up.call("aether_sendTransaction", json!([tx])).await {
                    Ok(_) => return,
                    Err(e) if attempt == 6 => warn!(%e, "gave up forwarding a transaction upstream"),
                    Err(_) => {}
                }
                tokio::time::sleep(wait).await;
                wait *= 2;
            }
        });
    }
}
