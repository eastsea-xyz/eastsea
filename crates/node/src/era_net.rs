//! Old eras between nodes (roadmap B4).
//!
//! A node serves the era files it keeps over its JSON-RPC (loopback HTTP and
//! the public iroh endpoint, the same path as state snapshots):
//! `aether_eraInfo [era]` (size, BLAKE3, chunk size), `aether_eraChunk [era,
//! index]` (hex), and `aether_eraProof [era, anchor]` (the era root's path
//! under block `anchor`'s history root). A node that needs an era it pruned or
//! never had fetches it from any peer and trusts nothing it receives: the file
//! is re-hashed block by block (`era::read`), its root must be in a certified
//! history root it already holds (`aether_light::verify_era_root`), and must
//! equal the root its own history index keeps when it has one.
//!
//! iroh-blobs is not a dependency (only iroh itself is), so eras travel over
//! this RPC instead of BLAKE3-verified blob streams; the checks above make the
//! transport irrelevant to safety. See docs/design/13-roadmap.md B4.

use crate::era::{self, Era};
use crate::follow::Upstream;
use aether_state::mmr::MmrProof;
use aether_types::B256;
use serde_json::{json, Value};

/// Bytes per era chunk (hex doubles it on the wire; well under message limits).
pub const ERA_CHUNK: usize = 1 << 20;
/// Largest era file a node downloads (8192 full blocks compress far below it).
pub const MAX_ERA_FILE: usize = 256 << 20;

/// A history root this node trusts: the MMR root over the first `leaves`
/// blocks, as committed in block `leaves`'s header (its own finalized chain,
/// or a block whose certificate it checked).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryAnchor {
    pub leaves: u64,
    pub root: B256,
}

impl HistoryAnchor {
    /// The history of this node's own finalized chain (blocks 0..=head).
    pub fn of_chain(chain: &crate::chain::Chain) -> HistoryAnchor {
        let f = chain.lock().finalized.clone();
        HistoryAnchor { leaves: f.history.leaves, root: B256::from(f.history.root(&aether_hash::ChainHasher::new())) }
    }
}

/// The era file this node keeps, if any.
pub fn kept(store: &crate::store::Store, era: u64) -> Option<Vec<u8>> {
    std::fs::read(store.era_dir().join(era::file_name(era))).ok()
}

/// `aether_eraInfo`: what a peer needs to download era `era` from this node.
pub fn info(store: &crate::store::Store, era: u64) -> Value {
    match kept(store, era) {
        Some(b) => json!({ "era": era, "size": b.len(), "blake3": crate::rpc::blake3_hex(&b), "chunk": ERA_CHUNK }),
        None => Value::Null,
    }
}

/// `aether_eraChunk`: chunk `index` of era `era` (hex).
pub fn chunk(store: &crate::store::Store, era: u64, index: usize) -> Result<Value, String> {
    let b = kept(store, era).ok_or(format!("era {era} is not kept here"))?;
    let start = index.saturating_mul(ERA_CHUNK).min(b.len());
    let end = (start + ERA_CHUNK).min(b.len());
    Ok(json!({ "data": hex::encode(&b[start..end]) }))
}

/// Check era bytes from anywhere: they decode to 8192 blocks hashing to their
/// root, the root is in `anchor`'s history (`proof`), and it equals `known`
/// (the root this node's history index keeps) when given.
pub fn verify(bytes: &[u8], era: u64, anchor: &HistoryAnchor, proof: &MmrProof, known: Option<&[u8; 32]>) -> Result<Era, String> {
    let e = era::read(bytes, known).map_err(|e| format!("era {era}: {e}"))?;
    if e.index != era {
        return Err(format!("asked for era {era}, got era {}", e.index));
    }
    let v = aether_light::VerifiedBlock {
        height: anchor.leaves,
        digest: String::new(),
        timestamp_ms: 0,
        parent_state_root: B256::ZERO,
        history_root: anchor.root,
    };
    aether_light::verify_era_root(&v, era, &e.root, proof).map_err(|e| format!("era {era} is not in the certified history: {e:?}"))?;
    Ok(e)
}

/// Download era `era` from `upstream` and verify it against `anchor` (and
/// `known`). Returns the file bytes and the decoded era.
pub async fn fetch(upstream: &Upstream, era: u64, anchor: &HistoryAnchor, known: Option<&[u8; 32]>) -> Result<(Vec<u8>, Era), String> {
    let v = upstream.first("aether_eraInfo", json!([era])).await?;
    if v.is_null() {
        return Err(format!("no source keeps era {era}"));
    }
    let size = v["size"].as_u64().ok_or("no era size")? as usize;
    let chunk = v["chunk"].as_u64().ok_or("no era chunk size")? as usize;
    let want = v["blake3"].as_str().unwrap_or_default().to_string();
    if size > MAX_ERA_FILE || !(64 << 10..=4 << 20).contains(&chunk) {
        return Err(format!("era of {size} bytes in chunks of {chunk} is outside the limits"));
    }
    let mut bytes = Vec::with_capacity(size);
    for index in 0..size.div_ceil(chunk) {
        let c = upstream.first("aether_eraChunk", json!([era, index])).await?;
        let data = hex::decode(c["data"].as_str().ok_or("no chunk data")?).map_err(|e| e.to_string())?;
        if data.len() != chunk.min(size - bytes.len()) {
            return Err("era chunk of the wrong size".into());
        }
        bytes.extend(data);
    }
    // Integrity of the download; authenticity comes from the history root below.
    if crate::rpc::blake3_hex(&bytes) != want {
        return Err("era download does not match its BLAKE3".into());
    }
    let p = upstream.first("aether_eraProof", json!([era, anchor.leaves])).await?;
    let proof: MmrProof = serde_json::from_value(p["proof"].clone()).map_err(|e| format!("era proof: {e}"))?;
    let e = verify(&bytes, era, anchor, &proof, known)?;
    Ok((bytes, e))
}

/// Keep a verified era file in the store's era folder (temporary name, then renamed).
pub fn save(store: &crate::store::Store, era: u64, bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    let dir = store.era_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(era::file_name(era));
    let tmp = dir.join(format!("{}.part", era::file_name(era)));
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Fetch, verify and keep era `era` for `chain` (anchored on its own finalized
/// history; its history index root must match when it has one).
pub async fn fetch_into(chain: &crate::chain::Chain, upstream: &Upstream, era: u64) -> Result<std::path::PathBuf, String> {
    let store = chain.store().ok_or("no store")?;
    let anchor = HistoryAnchor::of_chain(chain);
    let known = chain.lock().history_index.as_ref().and_then(|i| i.eras.get(era as usize).copied());
    let (bytes, _) = fetch(upstream, era, &anchor, known.as_ref()).await?;
    save(&store, era, &bytes)
}
