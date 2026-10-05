//! Re-execute a finalized chain in an isolated store before upgrading a node.
//! Source data is read from an archive peer or a stopped history-v2 data dir.

use crate::block::Block;
use crate::chain::{Chain, ChainConfig};
use crate::store::{Checkpoint, Store};
use aether_execution::Receipt;
use aether_state::mmr::ERA_LEN;
use aether_types::{B256, TxHash};
use commonware_codec::Decode;
use commonware_cryptography::Digestible;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub enum Source {
    Peer {
        url: String,
        client: reqwest::blocking::Client,
    },
    Local {
        dir: PathBuf,
        store: Store,
        checkpoint: Checkpoint,
    },
}

impl Source {
    /// Copy the checkpoint before opening redb: opening it must never write to
    /// the real node's file or contend with its database lock.
    pub fn open(from: &str, scratch: &Path) -> Result<Self, String> {
        if from.starts_with("http://") || from.starts_with("https://") {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?;
            return Ok(Self::Peer {
                url: from.to_owned(),
                client,
            });
        }
        let dir = PathBuf::from(from);
        let source = dir.join("state.redb");
        if !source.is_file() {
            return Err(format!("no state.redb in {}", dir.display()));
        }
        std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
        let copy = scratch.join("source.redb");
        std::fs::copy(source, &copy).map_err(|e| e.to_string())?;
        let store = Store::open(&copy).map_err(|e| e.to_string())?;
        let checkpoint = store
            .load()
            .map_err(|e| e.to_string())?
            .ok_or("source has no finalized checkpoint")?;
        Ok(Self::Local {
            dir,
            store,
            checkpoint,
        })
    }

    pub fn head(&self) -> Result<u64, String> {
        match self {
            Self::Local { checkpoint, .. } => Ok(checkpoint.height),
            Self::Peer { url, client } => rpc(client, url, "aether_status", json!([]))?["height"]
                .as_u64()
                .ok_or("peer has no finalized height".into()),
        }
    }

    fn genesis_hash(&self) -> Result<String, String> {
        match self {
            Self::Local { checkpoint, .. } => checkpoint.blocks.get(&0).map(|b| b.hash.clone())
                .ok_or("source has no genesis summary".into()),
            Self::Peer { url, client } => rpc(client, url, "aether_getBlock", json!([0]))?["hash"]
                .as_str().map(str::to_owned).ok_or("peer has no genesis summary".into()),
        }
    }

    fn block(&self, height: u64) -> Result<Block, String> {
        match self {
            Self::Local { dir, store, .. } => {
                let era = height / ERA_LEN;
                let file = dir.join("eras").join(crate::era::file_name(era));
                if file.is_file() {
                    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
                    let decoded = crate::era::read(&bytes, None).map_err(|e| e.to_string())?;
                    return decoded
                        .blocks
                        .get((height % ERA_LEN) as usize)
                        .cloned()
                        .ok_or(format!("era lacks block {height}"));
                }
                let (rows, _) = store.staged(era).map_err(|e| e.to_string())?;
                let bytes = rows
                    .into_iter()
                    .find(|(h, _)| *h == height)
                    .map(|(_, b)| b)
                    .ok_or(format!(
                        "source has no block {height}; use a history-v2 archive or an archive peer"
                    ))?;
                decode(&bytes)
            }
            Self::Peer { url, client } => {
                let certified = rpc(client, url, "aether_getFinalized", json!([height]))?;
                let block = if certified["block"].is_string() {
                    certified
                } else {
                    rpc(client, url, "aether_getBlock", json!([height]))?
                };
                let bytes = aether_light::from_hex(block["block"].as_str().ok_or(format!(
                    "peer has no finalized block {height}; use an archive peer"
                ))?)
                .map_err(|e| e.to_string())?;
                decode(&bytes)
            }
        }
    }

    fn state_root(&self, height: u64) -> Result<B256, String> {
        match self {
            Self::Local { checkpoint, .. } if height == checkpoint.height => Ok(checkpoint.state.root()),
            Self::Local { .. } => self.block(height + 1)?
                .payload()
                .map(|p| p.parent_state_root)
                .ok_or(format!("source block {} has no parent state root", height + 1)),
            Self::Peer { url, client } => {
                let row = rpc(client, url, "aether_getBlock", json!([height]))?;
                serde_json::from_value(row["state_root"].clone())
                    .map_err(|e| format!("peer state root at {height}: {e}"))
            }
        }
    }

    fn receipt(&self, height: u64, hash: TxHash) -> Result<Receipt, String> {
        match self {
            Self::Local { checkpoint, .. } => checkpoint
                .receipts
                .get(&hash)
                .and_then(|(h, r)| (*h == height).then_some(r.clone()))
                .ok_or(format!(
                    "source has no receipt {hash} at {height}; use an archive node"
                )),
            Self::Peer { url, client } => {
                let row = rpc(client, url, "aether_getReceipt", json!([hash]))?;
                if row["height"].as_u64() != Some(height) {
                    return Err(format!(
                        "peer has no receipt {hash} at {height}; use an archive peer"
                    ));
                }
                serde_json::from_value(row["receipt"].clone()).map_err(|e| e.to_string())
            }
        }
    }
}

fn decode(bytes: &[u8]) -> Result<Block, String> {
    Block::decode_cfg(bytes, &Block::codec_config(aether_light::MAX_BLOCK_BYTES))
        .map_err(|e| e.to_string())
}

fn rpc(
    client: &reqwest::blocking::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let response: Value = client
        .post(url)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    if let Some(error) = response.get("error") {
        return Err(format!("{method}: {error}"));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

/// A replay diagnostic digest, including receipt order and lengths. New-genesis
/// block validity separately uses the canonical `receipt::receipt_root`.
pub fn receipts_digest(receipts: &[Receipt]) -> String {
    let mut hash = blake3::Hasher::new();
    for receipt in receipts {
        let bytes = serde_json::to_vec(receipt).expect("receipt serializes");
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    hash.finalize().to_hex().to_string()
}

pub fn replay(cfg: ChainConfig, source: &Source, to: u64, scratch: &Path) -> Result<(), String> {
    let head = source.head()?;
    if to > head {
        return Err(format!(
            "source is finalized only through {head}, requested {to}"
        ));
    }
    let (chain, genesis) = Chain::open(
        cfg,
        Store::open(&scratch.join("replay.redb")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let first = source.genesis_hash()?;
    if first != format!("{}", genesis.digest()) {
        return Err(format!(
            "SHADOW MISMATCH height 0 block/genesis: source {} replay {}",
            first,
            genesis.digest()
        ));
    }
    chain
        .finalize(&genesis)
        .map_err(|e| format!("height 0: {e:?}"))?;
    let genesis_root = chain.lock().finalized.state.root();
    let expected_genesis_root = source.state_root(0)?;
    if genesis_root != expected_genesis_root {
        return Err(format!(
            "SHADOW MISMATCH height 0 state root: source {expected_genesis_root} replay {genesis_root}"
        ));
    }
    for height in 1..=to {
        let block = source.block(height)?;
        if block.height.get() != height {
            return Err(format!(
                "SHADOW MISMATCH height {height}: source returned block {}",
                block.height.get()
            ));
        }
        chain
            .finalize(&block)
            .map_err(|e| format!("SHADOW MISMATCH height {height} execution: {e:?}"))?;
        let actual = chain.lock().finalized.clone();
        let expected_root = source.state_root(height)?;
        if actual.state.root() != expected_root {
            return Err(format!(
                "SHADOW MISMATCH height {height} state root: source {expected_root} replay {}",
                actual.state.root()
            ));
        }
        let hashes = actual.tx_hashes.clone();
        let observed = source_receipts(source, height, &hashes)?;
        let replayed = hashes
            .iter()
            .map(|h| {
                chain
                    .lock()
                    .receipts
                    .get(h)
                    .map(|(_, r)| r.clone())
                    .ok_or(format!("replay has no receipt {h} at {height}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (expected, got) = (receipts_digest(&observed), receipts_digest(&replayed));
        if expected != got {
            return Err(format!(
                "SHADOW MISMATCH height {height} receipts digest: source {expected} replay {got}"
            ));
        }
        if height == to || height.is_multiple_of(1000) {
            println!(
                "shadow {height}/{to} state={} receipts={got}",
                actual.state.root()
            );
        }
    }
    Ok(())
}

fn source_receipts(
    source: &Source,
    height: u64,
    hashes: &[TxHash],
) -> Result<Vec<Receipt>, String> {
    hashes.iter().map(|h| source.receipt(height, *h)).collect()
}
