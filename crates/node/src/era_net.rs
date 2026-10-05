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
//!
//! Serving (audit §5) never reads a whole era file per call: `aether_eraChunk`
//! seeks to the asked range and reads exactly that, and `aether_eraInfo`'s
//! BLAKE3 is streamed once per file and then cached (era files are sealed once
//! and never change; the cache follows the file's length and mtime).

use crate::era::{self, Era};
use crate::follow::Upstream;
use aether_state::mmr::MmrProof;
use aether_types::B256;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Bytes per era chunk (hex doubles it on the wire; well under message limits).
pub const ERA_CHUNK: usize = 1 << 20;
/// Largest era file a node downloads (8192 full blocks compress far below it).
pub const MAX_ERA_FILE: usize = 256 << 20;
/// Era files whose (size, BLAKE3) is cached; a peer asks about a few eras.
const ERA_CACHE: usize = 16;
/// The (size, BLAKE3) of one era file, remembered while the file stands still.
struct CachedEra {
    len: u64,
    modified: SystemTime,
    blake3: String,
}
static ERA_INFO: Mutex<Vec<(PathBuf, CachedEra)>> = Mutex::new(Vec::new());

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

/// The era file this node keeps: its path, length and mtime, if it is a file
/// within the size a peer may download.
fn kept_file(store: &crate::store::Store, era: u64) -> Option<(PathBuf, u64, SystemTime)> {
    let path = store.era_dir().join(era::file_name(era));
    let m = std::fs::metadata(&path).ok()?;
    if !m.is_file() || m.len() > MAX_ERA_FILE as u64 {
        return None;
    }
    Some((path, m.len(), m.modified().ok()?))
}

/// The era file's BLAKE3, hashed in bounded windows (never the whole file in
/// memory) and cached per path while its length and mtime hold.
fn blake3_of(path: &Path, len: u64, modified: SystemTime) -> Option<String> {
    {
        let mut cache = ERA_INFO.lock().expect("era info cache");
        if let Some(i) = cache.iter().position(|(p, _)| p == path) {
            if cache[i].1.len == len && cache[i].1.modified == modified {
                let hash = cache[i].1.blake3.clone();
                let entry = cache.remove(i); // most recently used last
                cache.push(entry);
                return Some(hash);
            }
            cache.remove(i); // the file moved on: recompute
        }
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; ERA_CHUNK];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hash = hasher.finalize().to_hex().to_string();
    let mut cache = ERA_INFO.lock().expect("era info cache");
    if cache.len() >= ERA_CACHE {
        cache.remove(0);
    }
    cache.push((path.to_path_buf(), CachedEra { len, modified, blake3: hash.clone() }));
    Some(hash)
}

/// The whole era file this node keeps, if any (shards cut it into pieces;
/// peers get ranges through `chunk`). Files above `MAX_ERA_FILE` are not read.
pub fn kept(store: &crate::store::Store, era: u64) -> Option<Vec<u8>> {
    let path = store.era_dir().join(era::file_name(era));
    let len = std::fs::metadata(&path).ok()?.len();
    if len > MAX_ERA_FILE as u64 {
        return None;
    }
    std::fs::read(path).ok()
}

/// `aether_eraInfo`: what a peer needs to download era `era` from this node.
pub fn info(store: &crate::store::Store, era: u64) -> Value {
    let Some((path, len, modified)) = kept_file(store, era) else {
        return Value::Null;
    };
    match blake3_of(&path, len, modified) {
        Some(blake3) => json!({ "era": era, "size": len, "blake3": blake3, "chunk": ERA_CHUNK }),
        None => Value::Null, // sealed but unreadable: as if not kept here
    }
}

/// `aether_eraChunk`: chunk `index` of era `era` (hex). Only the asked range
/// is read: seek plus `read_exact`, at most `ERA_CHUNK` bytes, whatever the
/// index says.
pub fn chunk(store: &crate::store::Store, era: u64, index: usize) -> Result<Value, String> {
    let (path, len, _) = kept_file(store, era).ok_or(format!("era {era} is not kept here"))?;
    let start = index.saturating_mul(ERA_CHUNK).min(len as usize);
    let end = (start + ERA_CHUNK).min(len as usize);
    let mut buf = vec![0u8; end - start];
    if !buf.is_empty() {
        let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::Start(start as u64)).map_err(|e| e.to_string())?;
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    }
    Ok(json!({ "data": hex::encode(buf) }))
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
        receipts_root: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> crate::store::Store {
        let d = std::env::temp_dir().join(format!("aether-era-net-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        crate::store::Store::open(&d.join("db.redb")).unwrap()
    }

    /// A deterministic pattern of `len` bytes written as the node's era `era`.
    fn file(store: &crate::store::Store, era: u64, len: usize) -> Vec<u8> {
        std::fs::create_dir_all(store.era_dir()).unwrap();
        let bytes: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        std::fs::write(store.era_dir().join(era::file_name(era)), &bytes).unwrap();
        bytes
    }

    #[test]
    fn info_and_chunks_answer_from_the_file_without_reading_it_all() {
        let s = store("serve");
        let len = 2 * ERA_CHUNK + 543_210;
        let bytes = file(&s, 7, len);

        let v = info(&s, 7);
        assert_eq!(v["era"], json!(7));
        assert_eq!(v["size"], json!(len));
        assert_eq!(v["chunk"], json!(ERA_CHUNK));
        assert_eq!(v["blake3"], json!(crate::rpc::blake3_hex(&bytes)));

        assert_eq!(chunk(&s, 7, 0).unwrap()["data"], json!(hex::encode(&bytes[..ERA_CHUNK])));
        assert_eq!(chunk(&s, 7, 1).unwrap()["data"], json!(hex::encode(&bytes[ERA_CHUNK..2 * ERA_CHUNK])));
        assert_eq!(chunk(&s, 7, 2).unwrap()["data"], json!(hex::encode(&bytes[2 * ERA_CHUNK..])));
        assert_eq!(chunk(&s, 7, 3).unwrap()["data"], json!(""), "just past the end: empty, not an error");
        assert_eq!(chunk(&s, 7, usize::MAX / 2).unwrap()["data"], json!(""), "a wild index is clamped, not a panic");
    }

    #[test]
    fn a_missing_era_answers_null_and_an_error() {
        let s = store("missing");
        assert_eq!(info(&s, 3), Value::Null);
        assert!(chunk(&s, 3, 0).unwrap_err().contains("not kept"));
    }

    #[test]
    fn info_follows_the_file_and_the_cache_never_goes_stale() {
        let s = store("cache");
        file(&s, 9, ERA_CHUNK);
        let first = info(&s, 9);
        assert_eq!(first, info(&s, 9), "the second answer comes from the cache");
        // The file is rewritten longer: length and mtime moved, so the hash is recomputed.
        let longer = vec![7u8; 3 * ERA_CHUNK];
        std::fs::write(s.era_dir().join(era::file_name(9)), &longer).unwrap();
        let second = info(&s, 9);
        assert_eq!(second["size"], json!(longer.len()));
        assert_ne!(second["blake3"], first["blake3"]);
        assert_eq!(second["blake3"], json!(crate::rpc::blake3_hex(&longer)));
    }
}
