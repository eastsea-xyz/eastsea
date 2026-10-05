//! Era files (docs/research/history-compression-2026.md, roadmap B3).
//!
//! An era is 8192 consecutive finalized blocks starting at a multiple of 8192
//! (`aether_state::mmr::ERA_LEN`). Its identity is its MMR sub-root: the root of
//! the perfect tree over leaves H('L' ‖ height ‖ block hash), which is a node of
//! the history MMR. So one certificate on any later block, plus one
//! `MmrProof::verify_node` path, authenticates the whole era, and every block in
//! it is re-hashed from the file (nothing in the file is trusted).
//!
//! Layout (all integers little-endian or LEB128 varints):
//!
//! ```text
//! "AETHERA" 0x01                      magic + format version
//! era index (u64), block count (u32)
//! MMR before the era: leaves (u64), peak count (u8), peaks (height u8, digest 32)*
//! parent of the first block (32)
//! leaders: count (varint), ed25519 keys (32 each)
//! columns: zstd length (u32) + zstd bytes
//! bodies:  zstd length (u32) + zstd bytes
//! era root (32)                      checked against the recomputed root
//! ```
//!
//! Columns (one stream each, written back to back, so equal values sit
//! together): flags, epoch deltas, view deltas, parent-view gaps, leader
//! indices, timestamp deltas, version deltas, gas triples, body lengths
//! (the index into the bodies stream), and the 32-byte values that are not
//! recomputable: parent state roots and parent metadata hashes only when they
//! change, history roots and context parents only when they differ from the
//! recomputed ones. A receipts root, when present, follows that block's explicit
//! history root in the history column (flag bit 6); absent roots add no bytes.
//! Heights are implicit, parent hashes are the previous
//! block's hash, history roots follow from the MMR. Bodies (transactions,
//! access list, rare extras) are JSON, compressed together.
//!
//! Pruning and serving eras: `crate::prune`, `crate::era_net` (B4); shards:
//! `crate::shards` (B5). Not yet: the era's aggregated finality signature (one
//! BLS signature for all 8192 certificates); the file keeps room for it in a
//! later format version.

use crate::block::{Block, Context, Payload, PublicKey};
use aether_hash::{ChainHasher, Digest as H32};
use aether_state::mmr::{self, Mmr, MmrProof, ERA_BITS, ERA_LEN};
use aether_types::{BlockAccessList, GasVector, TxEnvelope, B256};
use commonware_codec::DecodeExt;
use commonware_consensus::types::{Epoch, Height, Round, View};
use commonware_cryptography::{sha256::Digest, Digestible};
use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 8] = b"AETHERA\x01";
/// zstd level: eras are written once and read rarely.
const ZSTD_LEVEL: i32 = 19;
/// Upper bound on one decompressed stream (8192 full blocks fit well under it).
const MAX_STREAM: usize = 1 << 30;

// Per-block flags.
const STATE_ROOT_CHANGED: u8 = 1;
const META_CHANGED: u8 = 1 << 1;
const HISTORY_EXPLICIT: u8 = 1 << 2;
const HAS_BODY: u8 = 1 << 3;
const RAW_PAYLOAD: u8 = 1 << 4;
const CONTEXT_PARENT_EXPLICIT: u8 = 1 << 5;
const HAS_RECEIPTS: u8 = 1 << 6;

#[derive(Debug, PartialEq, Eq)]
pub enum EraError {
    /// Not 8192 consecutive blocks from a multiple of 8192, or not a chain.
    NotAnEra(String),
    Corrupt(&'static str),
    /// The blocks hash to another era root than the file (or the caller) says.
    RootMismatch,
    /// The era root is not in the certified history.
    NotInHistory,
}

impl std::fmt::Display for EraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for EraError {}

/// A decoded, internally consistent era.
#[derive(Debug)]
pub struct Era {
    pub index: u64,
    /// History MMR over every block before the era.
    pub start: Mmr,
    pub blocks: Vec<Block>,
    /// The era's identity: the MMR node over its blocks.
    pub root: H32,
}

impl Era {
    pub fn first_height(&self) -> u64 {
        self.index * ERA_LEN
    }

    /// Check the era against a certified history root (`anchor_history_root`
    /// of a finalized block after it) with `proof` of its root (`prove_era`).
    pub fn verify_in_history(&self, proof: &MmrProof, anchor_history_root: &B256) -> Result<(), EraError> {
        if proof.index != self.first_height() || !proof.verify_node(&ChainHasher::new(), &self.root, ERA_BITS, &anchor_history_root.0) {
            return Err(EraError::NotInHistory);
        }
        Ok(())
    }
}

/// Transactions, access list and rare extras: what a block carries besides its header fields.
#[derive(Serialize, Deserialize)]
struct Body {
    txs: Vec<TxEnvelope>,
    bal: BlockAccessList,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handoff: Option<aether_light::block::Handoff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed: Option<aether_light::block::Seed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upgrade: Option<aether_light::block::SignedUpgrade>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    proofs: Vec<aether_light::block::ProofClaim>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    beacons: Vec<aether_light::block::BeaconAnswer>,
    /// The consensus group the block belongs to (0 = the only group today, and
    /// omitted — group-0 bodies keep their exact bytes).
    #[serde(default, skip_serializing_if = "is_zero_group")]
    group: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    registrations: Vec<aether_light::block::NodeRegistration>,
}

fn is_zero_group(group: &u16) -> bool {
    *group == 0
}

fn digest_of(d: &Digest) -> H32 {
    d.as_ref().try_into().expect("sha256 digest is 32 bytes")
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(v: i128) -> u64 {
    let v = v.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    ((v << 1) ^ (v >> 63)) as u64
}

fn unzigzag(v: u64) -> i64 {
    (v >> 1) as i64 ^ -((v & 1) as i64)
}

struct Cursor<'a> {
    b: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], EraError> {
        if self.b.len() < n {
            return Err(EraError::Corrupt("truncated"));
        }
        let (h, t) = self.b.split_at(n);
        self.b = t;
        Ok(h)
    }
    fn u8(&mut self) -> Result<u8, EraError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, EraError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> Result<u64, EraError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn h32(&mut self) -> Result<H32, EraError> {
        Ok(self.take(32)?.try_into().expect("32"))
    }
    fn varint(&mut self) -> Result<u64, EraError> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(EraError::Corrupt("varint"))
    }
}

/// The column streams, in file order.
#[derive(Default)]
struct Columns {
    flags: Vec<u8>,
    epochs: Vec<u8>,
    views: Vec<u8>,
    parent_views: Vec<u8>,
    leaders: Vec<u8>,
    timestamps: Vec<u8>,
    versions: Vec<u8>,
    gas: Vec<u8>,
    body_lens: Vec<u8>,
    state_roots: Vec<u8>,
    metas: Vec<u8>,
    histories: Vec<u8>,
    context_parents: Vec<u8>,
}

impl Columns {
    fn streams(&self) -> [&Vec<u8>; 13] {
        [
            &self.flags,
            &self.epochs,
            &self.views,
            &self.parent_views,
            &self.leaders,
            &self.timestamps,
            &self.versions,
            &self.gas,
            &self.body_lens,
            &self.state_roots,
            &self.metas,
            &self.histories,
            &self.context_parents,
        ]
    }
}

/// Encode era `index` from its 8192 finalized blocks and the history MMR before them.
pub fn write(start: &Mmr, blocks: &[Block]) -> Result<Vec<u8>, EraError> {
    let h = ChainHasher::new();
    let first = blocks.first().ok_or_else(|| EraError::NotAnEra("no blocks".into()))?;
    let first_height = first.height.get();
    if blocks.len() as u64 != ERA_LEN || !first_height.is_multiple_of(ERA_LEN) || start.leaves != first_height {
        return Err(EraError::NotAnEra(format!("{} blocks from {first_height}, history of {}", blocks.len(), start.leaves)));
    }
    let mut leaders: Vec<PublicKey> = Vec::new();
    let mut cols = Columns::default();
    let mut bodies = Vec::new();
    let (mut mmr, mut leaves) = (start.clone(), Vec::with_capacity(blocks.len()));
    let (mut epoch, mut view, mut ts, mut version) = (0u64, 0u64, 0u64, 0u32);
    let (mut state_root, mut meta) = (B256::ZERO, B256::ZERO);
    let mut parent = first.parent;
    for (i, b) in blocks.iter().enumerate() {
        let height = first_height + i as u64;
        if b.height.get() != height || b.parent != parent {
            return Err(EraError::NotAnEra(format!("block {i} does not follow the previous one")));
        }
        let mut flags = 0u8;
        let (e, v) = (b.context.round.epoch().get(), b.context.round.view().get());
        put_varint(&mut cols.epochs, zigzag(e as i128 - epoch as i128));
        put_varint(&mut cols.views, zigzag(v as i128 - view as i128));
        put_varint(&mut cols.parent_views, zigzag(v as i128 - b.context.parent.0.get() as i128));
        (epoch, view) = (e, v);
        if b.context.parent.1 != b.parent {
            flags |= CONTEXT_PARENT_EXPLICIT;
            cols.context_parents.extend_from_slice(b.context.parent.1.as_ref());
        }
        let leader = match leaders.iter().position(|l| *l == b.context.leader) {
            Some(k) => k,
            None => {
                leaders.push(b.context.leader.clone());
                leaders.len() - 1
            }
        };
        put_varint(&mut cols.leaders, leader as u64);
        put_varint(&mut cols.timestamps, zigzag(b.timestamp as i128 - ts as i128));
        ts = b.timestamp;
        let history = B256::from(mmr.root(&h));
        match Payload::from_bytes(&b.data).filter(|p| p.to_bytes() == b.data) {
            Some(p) => {
                put_varint(&mut cols.versions, zigzag(p.version as i128 - version as i128));
                version = p.version;
                if p.parent_state_root != state_root {
                    flags |= STATE_ROOT_CHANGED;
                    cols.state_roots.extend_from_slice(p.parent_state_root.as_slice());
                    state_root = p.parent_state_root;
                }
                if p.parent_meta != meta {
                    flags |= META_CHANGED;
                    cols.metas.extend_from_slice(p.parent_meta.as_slice());
                    meta = p.parent_meta;
                }
                if p.history_root != history {
                    flags |= HISTORY_EXPLICIT;
                    cols.histories.extend_from_slice(p.history_root.as_slice());
                }
                if let Some(root) = p.receipts_root {
                    flags |= HAS_RECEIPTS;
                    cols.histories.extend_from_slice(root.as_slice());
                }
                for g in [p.gas.exec, p.gas.state, p.gas.prove] {
                    put_varint(&mut cols.gas, g);
                }
                let body = Body { txs: p.txs, bal: p.bal, handoff: p.handoff, seed: p.seed, upgrade: p.upgrade, proofs: p.proofs, beacons: p.beacons, group: p.group, registrations: p.registrations };
                let empty = body.txs.is_empty()
                    && body.bal == BlockAccessList::default()
                    && body.handoff.is_none()
                    && body.seed.is_none()
                    && body.upgrade.is_none()
                    && body.proofs.is_empty()
                    && body.beacons.is_empty()
                    && body.group == 0
                    && body.registrations.is_empty();
                if !empty {
                    flags |= HAS_BODY;
                    let bytes = serde_json::to_vec(&body).map_err(|_| EraError::Corrupt("body"))?;
                    put_varint(&mut cols.body_lens, bytes.len() as u64);
                    bodies.extend_from_slice(&bytes);
                }
            }
            None => {
                // Not a canonical payload (never accepted by a node, but kept byte for byte).
                flags |= RAW_PAYLOAD | HAS_BODY;
                put_varint(&mut cols.body_lens, b.data.len() as u64);
                bodies.extend_from_slice(&b.data);
            }
        }
        cols.flags.push(flags);
        let d = digest_of(&b.digest());
        mmr = mmr.append(&h, height, &d);
        leaves.push(mmr::leaf(&h, height, &d));
        parent = b.digest();
    }

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(first_height / ERA_LEN).to_le_bytes());
    out.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.leaves.to_le_bytes());
    out.push(start.peaks.len() as u8);
    for (ht, d) in &start.peaks {
        out.push(*ht);
        out.extend_from_slice(d);
    }
    out.extend_from_slice(first.parent.as_ref());
    put_varint(&mut out, leaders.len() as u64);
    for l in &leaders {
        out.extend_from_slice(l.as_ref());
    }
    let mut columns = Vec::new();
    for s in cols.streams() {
        put_varint(&mut columns, s.len() as u64);
        columns.extend_from_slice(s);
    }
    for stream in [columns, bodies] {
        let z = zstd::bulk::compress(&stream, ZSTD_LEVEL).map_err(|_| EraError::Corrupt("zstd"))?;
        out.extend_from_slice(&(z.len() as u32).to_le_bytes());
        out.extend_from_slice(&(stream.len() as u32).to_le_bytes());
        out.extend_from_slice(&z);
    }
    out.extend_from_slice(&mmr::subtree_root(&h, &leaves));
    Ok(out)
}

fn stream(c: &mut Cursor<'_>) -> Result<Vec<u8>, EraError> {
    let z = c.u32()? as usize;
    let len = c.u32()? as usize;
    if len > MAX_STREAM {
        return Err(EraError::Corrupt("stream size"));
    }
    let bytes = c.take(z)?;
    let out = zstd::bulk::decompress(bytes, len).map_err(|_| EraError::Corrupt("zstd"))?;
    if out.len() != len {
        return Err(EraError::Corrupt("stream size"));
    }
    Ok(out)
}

/// Decode an era file, re-hash every block, and check it against the root the
/// file names (and, when given, the root the caller expects).
pub fn read(bytes: &[u8], expected_root: Option<&H32>) -> Result<Era, EraError> {
    let h = ChainHasher::new();
    let mut c = Cursor { b: bytes };
    if c.take(8)? != MAGIC {
        return Err(EraError::Corrupt("magic"));
    }
    let index = c.u64()?;
    let count = c.u32()? as u64;
    let first_height = index.checked_mul(ERA_LEN).ok_or(EraError::Corrupt("era index"))?;
    if count != ERA_LEN {
        return Err(EraError::Corrupt("block count"));
    }
    let leaves_before = c.u64()?;
    let npeaks = c.u8()? as usize;
    let mut peaks = Vec::with_capacity(npeaks);
    for _ in 0..npeaks {
        let ht = c.u8()?;
        peaks.push((ht, c.h32()?));
    }
    let start = Mmr { leaves: leaves_before, peaks };
    // The peaks' heights are the binary digits of the leaf count.
    let heights: Vec<u8> = (0..64u8).rev().filter(|b| leaves_before >> b & 1 == 1).collect();
    if leaves_before != first_height || start.peaks.iter().map(|p| p.0).collect::<Vec<_>>() != heights {
        return Err(EraError::Corrupt("history before the era"));
    }
    let first_parent = Digest::decode(c.take(32)?).map_err(|_| EraError::Corrupt("parent"))?;
    let nleaders = c.varint()?;
    if nleaders > count {
        return Err(EraError::Corrupt("leaders"));
    }
    let mut leaders = Vec::new();
    for _ in 0..nleaders {
        leaders.push(PublicKey::decode(c.take(32)?).map_err(|_| EraError::Corrupt("leader"))?);
    }
    let columns = stream(&mut c)?;
    let bodies = stream(&mut c)?;
    let file_root = c.h32()?;
    if !c.b.is_empty() {
        return Err(EraError::Corrupt("trailing bytes"));
    }

    let mut cc = Cursor { b: &columns };
    let mut col = || -> Result<Vec<u8>, EraError> {
        let n = cc.varint()? as usize;
        Ok(cc.take(n)?.to_vec())
    };
    let flags = col()?;
    let (epochs, views, parent_views, leader_ix, timestamps, versions, gas, body_lens) = (col()?, col()?, col()?, col()?, col()?, col()?, col()?, col()?);
    let (state_roots, metas, histories, context_parents) = (col()?, col()?, col()?, col()?);
    if !cc.b.is_empty() {
        return Err(EraError::Corrupt("trailing columns"));
    }
    if flags.len() as u64 != count {
        return Err(EraError::Corrupt("flags"));
    }
    let mut cur = [&epochs, &views, &parent_views, &leader_ix, &timestamps, &versions, &gas, &body_lens].map(|v| Cursor { b: v });
    let [epochs, views, parent_views, leader_ix, timestamps, versions, gas, body_lens] = &mut cur;
    let (mut state_roots, mut metas, mut histories, mut context_parents, mut bodies) =
        (Cursor { b: &state_roots }, Cursor { b: &metas }, Cursor { b: &histories }, Cursor { b: &context_parents }, Cursor { b: &bodies });

    let mut blocks = Vec::with_capacity(count as usize);
    let (mut mmr, mut leaves) = (start.clone(), Vec::with_capacity(count as usize));
    let (mut epoch, mut view, mut ts, mut version) = (0i64, 0i64, 0i64, 0i64);
    let (mut state_root, mut meta) = (B256::ZERO, B256::ZERO);
    let mut parent = first_parent;
    let add = |base: i64, d: u64| base.checked_add(unzigzag(d)).filter(|v| *v >= 0).ok_or(EraError::Corrupt("delta"));
    for (i, &f) in flags.iter().enumerate() {
        if f & !(STATE_ROOT_CHANGED | META_CHANGED | HISTORY_EXPLICIT | HAS_BODY | RAW_PAYLOAD | CONTEXT_PARENT_EXPLICIT | HAS_RECEIPTS) != 0
            || f & RAW_PAYLOAD != 0 && (f & HAS_BODY == 0 || f & (STATE_ROOT_CHANGED | META_CHANGED | HISTORY_EXPLICIT | HAS_RECEIPTS) != 0)
        {
            return Err(EraError::Corrupt("flags"));
        }
        let height = first_height + i as u64;
        epoch = add(epoch, epochs.varint()?)?;
        view = add(view, views.varint()?)?;
        let parent_view = view.checked_sub(unzigzag(parent_views.varint()?)).filter(|v| *v >= 0).ok_or(EraError::Corrupt("parent view"))?;
        let leader = leaders.get(leader_ix.varint()? as usize).ok_or(EraError::Corrupt("leader index"))?.clone();
        ts = add(ts, timestamps.varint()?)?;
        let context_parent =
            if f & CONTEXT_PARENT_EXPLICIT != 0 { Digest::decode(context_parents.take(32)?).map_err(|_| EraError::Corrupt("context parent"))? } else { parent };
        let data = if f & RAW_PAYLOAD != 0 {
            let n = body_lens.varint()? as usize;
            bytes::Bytes::copy_from_slice(bodies.take(n)?)
        } else {
            version = add(version, versions.varint()?)?;
            if f & STATE_ROOT_CHANGED != 0 {
                state_root = B256::from(state_roots.h32()?);
            }
            if f & META_CHANGED != 0 {
                meta = B256::from(metas.h32()?);
            }
            let history_root = if f & HISTORY_EXPLICIT != 0 { B256::from(histories.h32()?) } else { B256::from(mmr.root(&h)) };
            let receipts_root = if f & HAS_RECEIPTS != 0 { Some(B256::from(histories.h32()?)) } else { None };
            let g = GasVector { exec: gas.varint()?, state: gas.varint()?, prove: gas.varint()? };
            let body = if f & HAS_BODY != 0 {
                let n = body_lens.varint()? as usize;
                serde_json::from_slice(bodies.take(n)?).map_err(|_| EraError::Corrupt("body"))?
            } else {
                Body { txs: vec![], bal: BlockAccessList::default(), handoff: None, seed: None, upgrade: None, proofs: vec![], beacons: vec![], group: 0, registrations: vec![] }
            };
            let p = Payload {
                version: u32::try_from(version).map_err(|_| EraError::Corrupt("version"))?,
                parent_state_root: state_root,
                history_root,
                receipts_root,
                parent_meta: meta,
                txs: body.txs,
                bal: body.bal,
                gas: g,
                handoff: body.handoff,
                seed: body.seed,
                upgrade: body.upgrade,
                proofs: body.proofs,
                beacons: body.beacons,
                group: body.group,
                registrations: body.registrations,
            };
            p.to_bytes()
        };
        let context = Context {
            round: Round::new(Epoch::new(epoch as u64), View::new(view as u64)),
            leader,
            parent: (View::new(parent_view as u64), context_parent),
        };
        let b = Block::new(context, parent, Height::new(height), ts as u64, data);
        let d = digest_of(&b.digest());
        mmr = mmr.append(&h, height, &d);
        leaves.push(mmr::leaf(&h, height, &d));
        parent = b.digest();
        blocks.push(b);
    }
    let rest = cur.iter().map(|c| c.b.len()).sum::<usize>()
        + state_roots.b.len()
        + metas.b.len()
        + histories.b.len()
        + context_parents.b.len()
        + bodies.b.len();
    if rest != 0 {
        return Err(EraError::Corrupt("unread column bytes"));
    }
    let root = mmr::subtree_root(&h, &leaves);
    if root != file_root || expected_root.is_some_and(|e| *e != root) {
        return Err(EraError::RootMismatch);
    }
    Ok(Era { index, start, blocks, root })
}

/// File name of era `era` in a store's era folder.
pub fn file_name(era: u64) -> String {
    format!("era-{era:08}.aera")
}

/// Seal era `era` from the blocks the store kept (history v2): write its file
/// (to a temporary name, then renamed), read it back and check it, then drop
/// the kept blocks. An era this node can never complete (it started from a
/// checkpoint inside it) is dropped without a file. Returns the file written.
pub fn seal(store: &crate::store::Store, era: u64) -> Result<Option<std::path::PathBuf>, String> {
    let (rows, start) = store.staged(era).map_err(|e| e.to_string())?;
    let complete = rows.len() as u64 == ERA_LEN && start.as_ref().is_some_and(|s| s.leaves == era * ERA_LEN);
    if !complete {
        store.drop_staged(era).map_err(|e| e.to_string())?;
        return Ok(None);
    }
    let start = start.expect("checked");
    let cfg = Block::codec_config(aether_light::MAX_BLOCK_BYTES);
    let blocks = rows
        .iter()
        .map(|(_, b)| <Block as commonware_codec::Decode>::decode_cfg(b.as_slice(), &cfg).map_err(|e| format!("kept block: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let bytes = write(&start, &blocks).map_err(|e| e.to_string())?;
    read(&bytes, None).map_err(|e| format!("era {era} does not read back: {e}"))?;
    let dir = store.era_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(file_name(era));
    let tmp = dir.join(format!("{}.tmp", file_name(era)));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&bytes).and_then(|_| f.sync_all()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    store.drop_staged(era).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

pub fn seal_logged(store: &crate::store::Store, era: u64) {
    match seal(store, era) {
        Ok(Some(p)) => tracing::info!(era, path = %p.display(), "era sealed"),
        Ok(None) => tracing::info!(era, "era incomplete here (started from a checkpoint inside it); not sealed"),
        Err(e) => tracing::warn!(era, %e, "could not seal era; its blocks stay kept"),
    }
}

/// Seal every kept era that ended at or before finalized height `head`.
pub fn seal_pending(store: &crate::store::Store, head: u64) {
    for era in store.staged_eras().unwrap_or_default() {
        if (era + 1) * ERA_LEN - 1 <= head {
            seal_logged(store, era);
        }
    }
}

/// Proof that era `index`'s root is in the history of the first `n` blocks
/// (the history root of block `n`), from the era roots a node keeps.
pub fn prove_era(index: &aether_state::mmr::EraIndex, n: u64, era: u64) -> Option<MmrProof> {
    let open = index.eras.len() as u64;
    mmr::prove_by_eras(&ChainHasher::new(), n, era * ERA_LEN, ERA_BITS, &index.eras, |e| (e == open).then(|| index.open.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_cryptography::{ed25519, Hasher as _, Sha256, Signer as _};

    /// A synthetic chain: 4 rotating leaders, ~1 s blocks with jitter, the history
    /// root each node would put in, state roots that change when `changes(h)`.
    fn chain(eras: u64, changes: impl Fn(u64) -> bool, body: impl Fn(u64) -> Option<Vec<TxEnvelope>>) -> (Vec<Block>, Vec<Mmr>) {
        chain_with_receipts(eras, changes, body, |_| None)
    }

    fn chain_with_receipts(
        eras: u64,
        changes: impl Fn(u64) -> bool,
        body: impl Fn(u64) -> Option<Vec<TxEnvelope>>,
        receipts: impl Fn(u64) -> Option<B256>,
    ) -> (Vec<Block>, Vec<Mmr>) {
        let h = ChainHasher::new();
        let leaders: Vec<PublicKey> = (0..4).map(|i| ed25519::PrivateKey::from_seed(i).public_key()).collect();
        let genesis = Block::genesis(7, B256::repeat_byte(1));
        let (mut blocks, mut mmrs) = (vec![genesis.clone()], vec![Mmr::default()]);
        let mut mmr = Mmr::default().append(&h, 0, &digest_of(&genesis.digest()));
        let mut root = B256::repeat_byte(1);
        for height in 1..eras * ERA_LEN {
            mmrs.push(mmr.clone());
            let prev = blocks.last().unwrap();
            if changes(height) {
                root = B256::from(digest_of(&Sha256::hash(&[height.to_be_bytes().as_slice()])));
            }
            let txs = body(height).unwrap_or_default();
            let payload = Payload {
                version: if height < 100 { 1 } else { 2 },
                parent_state_root: root,
                history_root: B256::from(mmr.root(&h)),
                receipts_root: receipts(height),
                parent_meta: B256::repeat_byte((height / 5000) as u8),
                gas: GasVector { exec: 21_000 * txs.len() as u64, ..Default::default() },
                txs,
                ..Default::default()
            };
            // Skipped views now and then (a nullified leader).
            let view = height + height / 1000;
            let context = Context {
                round: Round::new(Epoch::zero(), View::new(view)),
                leader: leaders[(view % 4) as usize].clone(),
                parent: (View::new(prev.context.round.view().get()), prev.digest()),
            };
            let ts = 1_790_000_000_000 + height * 1000 + u64::from(blake3::hash(&height.to_le_bytes()).as_bytes()[0] % 64);
            let b = Block::new(context, prev.digest(), Height::new(height), ts, payload.to_bytes());
            mmr = mmr.append(&h, height, &digest_of(&b.digest()));
            blocks.push(b);
        }
        mmrs.push(mmr);
        (blocks, mmrs)
    }

    fn era_of(blocks: &[Block], mmrs: &[Mmr], e: u64) -> Vec<u8> {
        let (a, b) = ((e * ERA_LEN) as usize, ((e + 1) * ERA_LEN) as usize);
        write(&mmrs[a], &blocks[a..b]).unwrap()
    }

    /// Rewrite the compressed columns while retaining the bodies and claimed root.
    fn alter_columns(bytes: &[u8], alter: impl FnOnce(&mut Vec<Vec<u8>>)) -> Vec<u8> {
        let mut c = Cursor { b: bytes };
        c.take(8 + 8 + 4 + 8).unwrap();
        let peaks = c.u8().unwrap() as usize;
        c.take(peaks * 33 + 32).unwrap();
        let leaders = c.varint().unwrap() as usize;
        c.take(leaders * 32).unwrap();
        let prefix = bytes.len() - c.b.len();
        let columns = stream(&mut c).unwrap();
        let suffix = c.b;
        let mut cc = Cursor { b: &columns };
        let mut cols = Vec::new();
        while !cc.b.is_empty() {
            let len = cc.varint().unwrap() as usize;
            cols.push(cc.take(len).unwrap().to_vec());
        }
        alter(&mut cols);
        let mut columns = Vec::new();
        for col in cols {
            put_varint(&mut columns, col.len() as u64);
            columns.extend_from_slice(&col);
        }
        let z = zstd::bulk::compress(&columns, ZSTD_LEVEL).unwrap();
        let mut out = bytes[..prefix].to_vec();
        out.extend_from_slice(&(z.len() as u32).to_le_bytes());
        out.extend_from_slice(&(columns.len() as u32).to_le_bytes());
        out.extend_from_slice(&z);
        out.extend_from_slice(suffix);
        out
    }

    #[test]
    fn receipts_roots_round_trip_without_json_bodies() {
        let (blocks, mmrs) = chain_with_receipts(1, |_| false, |_| None, |height| {
            match height % 3 {
                0 => None,
                1 => Some(B256::ZERO),
                _ => Some(B256::repeat_byte(42)),
            }
        });
        let bytes = era_of(&blocks, &mmrs, 0);
        let era = read(&bytes, None).unwrap();
        assert_eq!(era.blocks, blocks);
        assert_eq!(write(&era.start, &era.blocks).unwrap(), bytes);
        alter_columns(&bytes, |cols| {
            assert!(cols[0].iter().all(|flags| flags & HAS_BODY == 0));
            assert!(cols[0].iter().any(|flags| flags & HAS_RECEIPTS != 0));
        });
        let truncated = alter_columns(&bytes, |cols| { cols[11].pop(); });
        assert_eq!(read(&truncated, None).unwrap_err(), EraError::Corrupt("truncated"));
        let unknown_flag = alter_columns(&bytes, |cols| cols[0][1] |= 1 << 7);
        assert_eq!(read(&unknown_flag, None).unwrap_err(), EraError::Corrupt("flags"));
        let raw_with_root = alter_columns(&bytes, |cols| cols[0][1] |= RAW_PAYLOAD | HAS_BODY);
        assert_eq!(read(&raw_with_root, None).unwrap_err(), EraError::Corrupt("flags"));
        let extra_column = alter_columns(&bytes, |cols| cols.push(vec![]));
        assert_eq!(read(&extra_column, None).unwrap_err(), EraError::Corrupt("trailing columns"));
        let (legacy, legacy_mmrs) = chain(1, |_| false, |_| None);
        let legacy_bytes = era_of(&legacy, &legacy_mmrs, 0);
        alter_columns(&legacy_bytes, |cols| {
            assert!(cols[0].iter().all(|flags| flags & HAS_RECEIPTS == 0));
            assert!(cols[11].is_empty());
        });
    }

    #[test]
    fn eras_round_trip_block_for_block_and_prove_against_a_later_history_root() {
        let (blocks, mmrs) = chain(2, |h| h % 3 == 0, |_| None);
        let mut index = aether_state::mmr::EraIndex::default();
        for b in &blocks {
            index.push(&ChainHasher::new(), mmr::leaf(&ChainHasher::new(), b.height.get(), &digest_of(&b.digest())));
        }
        for e in 0..2 {
            let bytes = era_of(&blocks, &mmrs, e);
            let era = read(&bytes, Some(&index.eras[e as usize])).unwrap();
            assert_eq!(era.index, e);
            assert_eq!(era.blocks.as_slice(), &blocks[(e * ERA_LEN) as usize..((e + 1) * ERA_LEN) as usize]);
            // A later block's history root: block 2*ERA_LEN commits both eras.
            let n = 2 * ERA_LEN;
            let anchor = B256::from(mmrs[n as usize].root(&ChainHasher::new()));
            let mut short = index.clone();
            short.open = vec![];
            let full: Vec<H32> = blocks.iter().map(|b| mmr::leaf(&ChainHasher::new(), b.height.get(), &digest_of(&b.digest()))).collect();
            let proof = mmr::prove_by_eras(&ChainHasher::new(), n, e * ERA_LEN, ERA_BITS, &index.eras, |k| Some(full[(k * ERA_LEN) as usize..].to_vec()))
                .unwrap();
            era.verify_in_history(&proof, &anchor).unwrap();
            // The same proof does not vouch for the other era.
            let other = read(&era_of(&blocks, &mmrs, 1 - e), None).unwrap();
            assert_eq!(other.verify_in_history(&proof, &anchor), Err(EraError::NotInHistory));
        }
        // The node-side proof (roots only, open era in memory) is the same kind of proof.
        let proof = prove_era(&index, 2 * ERA_LEN, 0).unwrap();
        let anchor = B256::from(mmrs[(2 * ERA_LEN) as usize].root(&ChainHasher::new()));
        read(&era_of(&blocks, &mmrs, 0), None).unwrap().verify_in_history(&proof, &anchor).unwrap();
    }

    #[test]
    fn any_corruption_is_detected() {
        let (blocks, mmrs) = chain(1, |h| h % 50 == 0, |_| None);
        let bytes = era_of(&blocks, &mmrs, 0);
        let good = read(&bytes, None).unwrap();
        // Flip one bit anywhere: the file never decodes to other blocks under the same root.
        for pos in (0..bytes.len()).step_by(bytes.len() / 23 + 1).chain([bytes.len() - 1, 9, 20]) {
            let mut t = bytes.clone();
            t[pos] ^= 0x10;
            match read(&t, None) {
                Err(_) => {}
                Ok(era) => panic!("bit flip at {pos} decoded (root equal: {})", era.root == good.root),
            }
        }
        // A consistent file for another root is refused when the caller expects this one.
        assert_eq!(read(&bytes, Some(&[0u8; 32])).unwrap_err(), EraError::RootMismatch);
        assert!(read(&bytes[..bytes.len() - 1], None).is_err());
    }

    #[test]
    fn only_whole_aligned_eras_are_written() {
        let (blocks, mmrs) = chain(2, |_| false, |_| None);
        assert!(write(&mmrs[1], &blocks[1..(ERA_LEN + 1) as usize]).is_err(), "not aligned");
        assert!(write(&mmrs[0], &blocks[..10]).is_err(), "not whole");
        assert!(write(&mmrs[1], &blocks[..ERA_LEN as usize]).is_err(), "wrong history before it");
        let mut gap = blocks[..ERA_LEN as usize].to_vec();
        gap.swap(5, 6);
        assert!(write(&mmrs[0], &gap).is_err(), "not a chain");
    }

    #[test]
    fn empty_blocks_cost_a_few_bytes_each() {
        // Nothing changes but time and leaders (a quiet chain under history v2).
        let (blocks, mmrs) = chain(1, |_| false, |_| None);
        let quiet = era_of(&blocks, &mmrs, 0).len() as f64 / ERA_LEN as f64;
        // The state root changes every block (a protocol-2 chain records each block's statement).
        let (blocks, mmrs) = chain(1, |_| true, |_| None);
        let busy = era_of(&blocks, &mmrs, 0).len() as f64 / ERA_LEN as f64;
        println!("era bytes per empty block: {quiet:.2} (state unchanged), {busy:.2} (state root changes)");
        assert!(quiet < 4.0, "{quiet}");
        assert!(busy < 40.0, "{busy}");
    }
}
