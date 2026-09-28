//! Durable finalized state (docs/design/05-state.md).
//!
//! Every finalized block commits, in one redb transaction: its EIP-7864 tree
//! writes, new bytecode, block summary, receipts, and the checkpoint
//! `(height, digest, state root)`. On restart the tree is rebuilt from the
//! stored entries and its root must equal the checkpoint's; only blocks after
//! the checkpoint are re-executed.
//!
//! NOMT (design D5) is not used as the engine: it tags leaf/internal hashes in
//! the MSB, so its roots cannot equal EIP-7864 roots (see 05-state.md).

use crate::chain::BlockSummary;
use aether_execution::{Journal, Receipt, WorldState};
use aether_types::{Bytes, TxHash, B256};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const STATE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("state");
const CODE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("code");
const BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const RECEIPTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("receipts");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
/// History v2: finalized blocks (codec bytes) of eras not yet sealed into a file.
const ERA_BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("era_blocks");
/// Pruning (roadmap B4): roots of the eras whose per-block rows are gone
/// (era -> 32 bytes). Restarts and history proofs rebuild the history index
/// from them instead of from every block summary.
const ERA_ROOTS: TableDefinition<u64, &[u8]> = TableDefinition::new("era_roots");
/// First height whose summary, receipts and finality proof are still kept.
const PRUNED_BELOW: &str = "pruned_below";

fn era_start_key(era: u64) -> String {
    format!("era_start/{era}")
}
/// The latest committee handoff (JSON), restored with the checkpoint.
const HANDOFF: &str = "handoff";
/// The latest draw seed (JSON: height and seed), committed with its block.
const SEED: &str = "seed";
/// MMR peaks of the history (JSON), committed with each block.
const HISTORY: &str = "history";
/// Protocol activation schedule (JSON), committed with each block.
const SCHEDULE: &str = "schedule";
/// The head block's statement commitment and escrow share (JSON).
const STATEMENT: &str = "statement";
/// Finality proofs a follower verified (`aether_getFinalized` JSON by height):
/// history it keeps serving, also after it becomes a voting node.
const PROOFS: TableDefinition<u64, &[u8]> = TableDefinition::new("proofs");
/// Rewards paid to provers, kept by this node for tax records: (prover ‖ paid-in
/// height) -> JSON. Not consensus data; nothing is ever dropped.
const REWARDS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rewards");

#[derive(Debug)]
pub enum StoreError {
    Db(String),
    Corrupt(&'static str),
    /// The rebuilt tree's root differs from the checkpoint.
    RootMismatch {
        height: u64,
        stored: B256,
        rebuilt: B256,
    },
    /// The data was written for another genesis (another network or an older
    /// genesis): it is never mixed with this one.
    OtherGenesis,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for StoreError {}

fn dberr<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Db(e.to_string())
}

/// The finalized head restored from disk.
pub struct Checkpoint {
    pub height: u64,
    pub digest: [u8; 32],
    pub state: WorldState,
    pub blocks: BTreeMap<u64, BlockSummary>,
    pub receipts: HashMap<TxHash, (u64, Receipt)>,
    pub handoff: Option<crate::handoff::Pending>,
    pub seed: Option<(u64, aether_light::block::Seed)>,
    pub history: aether_state::mmr::Mmr,
    pub schedule: crate::upgrade::Schedule,
    pub statement: crate::chain::Statement,
    /// First height with a kept summary (0: nothing pruned).
    pub pruned_below: u64,
    /// Roots of the pruned eras, from era 0 (`ERA_ROOTS`).
    pub era_roots: Vec<[u8; 32]>,
}

/// What one pruning pass removed.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct PruneReport {
    pub summaries: u64,
    pub receipts: u64,
    pub proofs: u64,
}

/// One finalized block's data to persist.
pub struct Commit<'a> {
    pub height: u64,
    pub digest: [u8; 32],
    pub root: B256,
    pub diff: &'a Journal,
    pub summary: &'a BlockSummary,
    pub receipts: Vec<(TxHash, &'a Receipt)>,
    /// Set when this block carries a committee handoff.
    pub handoff: Option<&'a crate::handoff::Pending>,
    /// Set when this block carries a draw seed (with its height).
    pub seed: Option<&'a (u64, aether_light::block::Seed)>,
    /// MMR peaks over blocks 0..=height.
    pub history: &'a aether_state::mmr::Mmr,
    /// Protocol activations on chain up to this block.
    pub schedule: &'a crate::upgrade::Schedule,
    /// Its statement commitment and escrow share.
    pub statement: &'a crate::chain::Statement,
    /// History v2: the block's codec bytes, kept until its era is sealed.
    pub staged: Option<Staged<'a>>,
}

/// A finalized block kept for its era file (`crate::era`).
pub struct Staged<'a> {
    pub block: &'a [u8],
    /// On an era's first block: the history MMR before it (the era file's start).
    pub era_start: Option<&'a aether_state::mmr::Mmr>,
}

/// First byte of a packed block summary. Older stores hold JSON rows (they
/// start with `{`); both are read, new rows are packed (roadmap B1: JSON
/// summaries were ~590 of the ~680 bytes each empty block added).
const PACKED: u8 = 0xa1;
/// Files below this are not worth compacting at start-up.
const COMPACT_MIN_FILE: u64 = 32 << 20;

/// A block summary in postcard form, links to the previous height elided.
#[derive(Serialize, serde::Deserialize)]
struct Packed {
    hash: [u8; 32],
    /// None: the previous height's hash.
    parent: Option<[u8; 32]>,
    timestamp_ms: u64,
    proposer: [u8; 20],
    state_root: [u8; 32],
    /// None: the previous height's state root.
    parent_state_root: Option<[u8; 32]>,
    txs: Vec<[u8; 32]>,
    gas_used: u64,
    prove_gas: u64,
    base_fee: (u128, u128, u128),
    excess: (u64, u64, u64),
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    let b: [u8; 32] = hex::decode(s).ok()?.try_into().ok()?;
    // Only a string that comes back byte for byte is packed.
    (hex::encode(b) == s).then_some(b)
}

/// The hash and state root a summary row links its successor to.
fn summary_links(v: &[u8]) -> Option<(String, B256)> {
    if v.first() == Some(&PACKED) {
        let p: Packed = postcard::from_bytes(&v[1..]).ok()?;
        return Some((hex::encode(p.hash), B256::from(p.state_root)));
    }
    let s: BlockSummary = serde_json::from_slice(v).ok()?;
    Some((s.hash, s.state_root))
}

fn encode_summary(s: &BlockSummary, previous: Option<&(String, B256)>) -> Result<Vec<u8>, StoreError> {
    let (Some(hash), Some(parent)) = (hex32(&s.hash), hex32(&s.parent)) else {
        return serde_json::to_vec(s).map_err(dberr);
    };
    let p = Packed {
        hash,
        parent: (previous.map(|p| p.0.as_str()) != Some(s.parent.as_str())).then_some(parent),
        timestamp_ms: s.timestamp_ms,
        proposer: s.proposer.0 .0,
        state_root: s.state_root.0,
        parent_state_root: (previous.map(|p| p.1) != Some(s.parent_state_root)).then_some(s.parent_state_root.0),
        txs: s.txs.iter().map(|t| t.0).collect(),
        gas_used: s.gas_used,
        prove_gas: s.prove_gas,
        base_fee: (s.base_fee.exec, s.base_fee.state, s.base_fee.prove),
        excess: (s.excess.exec, s.excess.state, s.excess.prove),
    };
    let mut out = vec![PACKED];
    out.extend(postcard::to_allocvec(&p).map_err(dberr)?);
    Ok(out)
}

/// Decode a summary row (packed or JSON); `previous` is the row one height below.
fn decode_summary(v: &[u8], height: u64, previous: Option<&BlockSummary>) -> Option<BlockSummary> {
    if v.first() != Some(&PACKED) {
        return serde_json::from_slice(v).ok();
    }
    let p: Packed = postcard::from_bytes(&v[1..]).ok()?;
    let parent = match p.parent {
        Some(d) => hex::encode(d),
        None => previous?.hash.clone(),
    };
    let parent_state_root = match p.parent_state_root {
        Some(r) => B256::from(r),
        None => previous?.state_root,
    };
    Some(BlockSummary {
        height,
        hash: hex::encode(p.hash),
        parent,
        timestamp_ms: p.timestamp_ms,
        proposer: aether_types::Address::from(p.proposer),
        state_root: B256::from(p.state_root),
        parent_state_root,
        txs: p.txs.into_iter().map(B256::from).collect(),
        gas_used: p.gas_used,
        prove_gas: p.prove_gas,
        base_fee: aether_types::FeeVector { exec: p.base_fee.0, state: p.base_fee.1, prove: p.base_fee.2 },
        excess: aether_types::GasVector { exec: p.excess.0, state: p.excess.1, prove: p.excess.2 },
    })
}

/// Transaction hashes a summary row names (their receipts go with the row).
fn summary_txs(v: &[u8]) -> Option<Vec<[u8; 32]>> {
    if v.first() == Some(&PACKED) {
        let p: Packed = postcard::from_bytes(&v[1..]).ok()?;
        return Some(p.txs);
    }
    let s: BlockSummary = serde_json::from_slice(v).ok()?;
    Some(s.txs.iter().map(|t| t.0).collect())
}

/// A packed row with the links to its predecessor written out, so it decodes
/// on its own once the rows below it are pruned. JSON rows never elide links.
fn unelide(v: &[u8], previous: &(String, B256)) -> Option<Vec<u8>> {
    if v.first() != Some(&PACKED) {
        return Some(v.to_vec());
    }
    let mut p: Packed = postcard::from_bytes(&v[1..]).ok()?;
    if p.parent.is_none() {
        p.parent = Some(hex32(&previous.0)?);
    }
    if p.parent_state_root.is_none() {
        p.parent_state_root = Some(previous.1 .0);
    }
    let mut out = vec![PACKED];
    out.extend(postcard::to_allocvec(&p).ok()?);
    Some(out)
}

/// One table's share of the file.
#[derive(Debug, Clone, Serialize)]
pub struct TableUse {
    pub name: &'static str,
    pub entries: u64,
    /// Key and value bytes.
    pub stored: u64,
    /// Branch keys and other b-tree metadata.
    pub metadata: u64,
    /// Unused space inside the table's pages.
    pub fragmented: u64,
    pub pages: u64,
}

/// Where a store's bytes go (B1 of docs/design/13-roadmap.md).
#[derive(Debug, Clone, Serialize)]
pub struct StoreStats {
    pub tables: Vec<TableUse>,
    pub allocated_pages: u64,
    pub page_size: u64,
    pub stored: u64,
    pub metadata: u64,
    pub fragmented: u64,
}

pub struct Store {
    db: Database,
    /// The directory the store lives in (era files go in its `eras` folder).
    dir: std::path::PathBuf,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(dberr)?;
        }
        let db = Database::create(path).map_err(dberr)?;
        let tx = db.begin_write().map_err(dberr)?;
        for t in [STATE, CODE, RECEIPTS] {
            tx.open_table(t).map_err(dberr)?;
        }
        tx.open_table(BLOCKS).map_err(dberr)?;
        tx.open_table(META).map_err(dberr)?;
        tx.open_table(PROOFS).map_err(dberr)?;
        tx.open_table(REWARDS).map_err(dberr)?;
        tx.commit().map_err(dberr)?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut store = Store { db, dir };
        store.compact_if_sparse(path)?;
        Ok(store)
    }

    /// redb never shrinks its file: pages freed by copy-on-write stay in it
    /// (on the 7780 testnet ~40% of the file). Compact at start-up, when nothing
    /// else holds the store, once a large share of the file is free.
    fn compact_if_sparse(&mut self, path: &Path) -> Result<(), StoreError> {
        let file = std::fs::metadata(path).map_err(dberr)?.len();
        if file < COMPACT_MIN_FILE {
            return Ok(());
        }
        let used = {
            let tx = self.db.begin_write().map_err(dberr)?;
            let s = tx.stats().map_err(dberr)?;
            tx.abort().map_err(dberr)?;
            s.allocated_pages() * s.page_size() as u64
        };
        if used * 4 < file * 3 {
            while self.compact()? {}
        }
        Ok(())
    }

    pub fn put_meta(&self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        let tx = self.db.begin_write().map_err(dberr)?;
        tx.open_table(META).map_err(dberr)?.insert(key, value).map_err(dberr)?;
        tx.commit().map_err(dberr)
    }

    pub fn meta(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let t = tx.open_table(META).map_err(dberr)?;
        Ok(t.get(key).map_err(dberr)?.map(|v| v.value().to_vec()))
    }

    /// Record a reward paid to `prover` in block `height` (one per proven block: `proven`).
    pub fn put_reward(&self, prover: &[u8; 20], height: u64, proven: u64, record: &[u8]) -> Result<(), StoreError> {
        let mut key = prover.to_vec();
        key.extend_from_slice(&height.to_be_bytes());
        key.extend_from_slice(&proven.to_be_bytes());
        let tx = self.db.begin_write().map_err(dberr)?;
        tx.open_table(REWARDS).map_err(dberr)?.insert(key.as_slice(), record).map_err(dberr)?;
        tx.commit().map_err(dberr)
    }

    /// Every reward recorded for `prover`, oldest first.
    /// The newest `limit` rewards of `prover`, oldest first.
    pub fn rewards(&self, prover: &[u8; 20], limit: usize) -> Result<Vec<Vec<u8>>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let t = tx.open_table(REWARDS).map_err(dberr)?;
        let mut end = prover.to_vec();
        end.extend_from_slice(&[0xff; 16]);
        let mut out = Vec::new();
        for row in t.range(prover.as_slice()..=end.as_slice()).map_err(dberr)?.rev().take(limit) {
            out.push(row.map_err(dberr)?.1.value().to_vec());
        }
        out.reverse();
        Ok(out)
    }

    /// Certificates this node verified, served to wallets and other followers.
    /// Written without the fsync: they are re-fetchable certified data, and
    /// redb persists them with the next durable block commit.
    pub fn put_proof(&self, height: u64, proof: &[u8]) -> Result<(), StoreError> {
        let mut tx = self.db.begin_write().map_err(dberr)?;
        tx.set_durability(redb::Durability::None);
        tx.open_table(PROOFS).map_err(dberr)?.insert(height, proof).map_err(dberr)?;
        tx.commit().map_err(dberr)
    }

    pub fn proof(&self, height: u64) -> Result<Option<Vec<u8>>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let t = tx.open_table(PROOFS).map_err(dberr)?;
        Ok(t.get(height).map_err(dberr)?.map(|v| v.value().to_vec()))
    }

    /// Where sealed era files go.
    pub fn era_dir(&self) -> std::path::PathBuf {
        self.dir.join("eras")
    }

    /// Eras with blocks kept for sealing, oldest first.
    pub fn staged_eras(&self) -> Result<Vec<u64>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let t = match tx.open_table(ERA_BLOCKS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
            Err(e) => return Err(dberr(e)),
        };
        let mut eras: Vec<u64> = Vec::new();
        for row in t.iter().map_err(dberr)? {
            let e = row.map_err(dberr)?.0.value() / aether_state::mmr::ERA_LEN;
            if eras.last() != Some(&e) {
                eras.push(e);
            }
        }
        Ok(eras)
    }

    /// Era `era`'s kept blocks (codec bytes, by height) and the history before it.
    #[allow(clippy::type_complexity)]
    pub fn staged(&self, era: u64) -> Result<(Vec<(u64, Vec<u8>)>, Option<aether_state::mmr::Mmr>), StoreError> {
        let len = aether_state::mmr::ERA_LEN;
        let tx = self.db.begin_read().map_err(dberr)?;
        let mut rows = Vec::new();
        if let Ok(t) = tx.open_table(ERA_BLOCKS) {
            for row in t.range(era * len..(era + 1) * len).map_err(dberr)? {
                let (h, v) = row.map_err(dberr)?;
                rows.push((h.value(), v.value().to_vec()));
            }
        }
        let meta = tx.open_table(META).map_err(dberr)?;
        let start = match meta.get(era_start_key(era).as_str()).map_err(dberr)? {
            Some(v) => Some(serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("era start"))?),
            None => None,
        };
        Ok((rows, start))
    }

    /// Forget era `era`'s kept blocks (its file is written, or it can never be complete).
    pub fn drop_staged(&self, era: u64) -> Result<(), StoreError> {
        let len = aether_state::mmr::ERA_LEN;
        let tx = self.db.begin_write().map_err(dberr)?;
        {
            let mut t = tx.open_table(ERA_BLOCKS).map_err(dberr)?;
            t.retain_in(era * len..(era + 1) * len, |_, _| false).map_err(dberr)?;
            tx.open_table(META).map_err(dberr)?.remove(era_start_key(era).as_str()).map_err(dberr)?;
        }
        tx.commit().map_err(dberr)
    }

    /// First height whose summary is still kept (0: nothing pruned).
    pub fn pruned_below(&self) -> Result<u64, StoreError> {
        Ok(match self.meta(PRUNED_BELOW)? {
            Some(v) => u64::from_be_bytes(v.as_slice().try_into().map_err(|_| StoreError::Corrupt("pruned_below"))?),
            None => 0,
        })
    }

    /// The history MMR of the last persisted block (cheap; no state rebuild).
    pub fn history(&self) -> Result<Option<aether_state::mmr::Mmr>, StoreError> {
        match self.meta(HISTORY)? {
            Some(v) => serde_json::from_slice(&v).map(Some).map_err(|_| StoreError::Corrupt("history")),
            None => Ok(None),
        }
    }

    /// Roots of the pruned eras (era 0 first); empty when nothing was pruned.
    pub fn era_roots(&self) -> Result<Vec<[u8; 32]>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let t = match tx.open_table(ERA_ROOTS) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
            Err(e) => return Err(dberr(e)),
        };
        let mut out = Vec::new();
        for (i, row) in t.iter().map_err(dberr)?.enumerate() {
            let (k, v) = row.map_err(dberr)?;
            if k.value() != i as u64 {
                return Err(StoreError::Corrupt("era roots not contiguous"));
            }
            out.push(v.value().try_into().map_err(|_| StoreError::Corrupt("era root"))?);
        }
        Ok(out)
    }

    /// Pruning (roadmap B4): drop what the store keeps per block below `cutoff`
    /// (block summaries, their receipts, a follower's finality proofs) and
    /// record `roots` (era, root) of the eras below it. One era per
    /// transaction, so a crash leaves a consistent prefix pruned; the first
    /// kept summary is rewritten with its links so it loads on its own.
    pub fn prune_below(&self, cutoff: u64, roots: &[(u64, [u8; 32])]) -> Result<PruneReport, StoreError> {
        let len = aether_state::mmr::ERA_LEN;
        let mut report = PruneReport::default();
        let mut lo = self.pruned_below()?;
        while lo < cutoff {
            let hi = ((lo / len + 1) * len).min(cutoff);
            let tx = self.db.begin_write().map_err(dberr)?;
            {
                let mut blocks = tx.open_table(BLOCKS).map_err(dberr)?;
                let mut receipts = tx.open_table(RECEIPTS).map_err(dberr)?;
                let links = blocks.get(hi - 1).map_err(dberr)?.and_then(|v| summary_links(v.value()));
                let mut txs = Vec::new();
                for row in blocks.range(lo..hi).map_err(dberr)? {
                    let (_, v) = row.map_err(dberr)?;
                    txs.extend(summary_txs(v.value()).ok_or(StoreError::Corrupt("block summary"))?);
                    report.summaries += 1;
                }
                for t in &txs {
                    if receipts.remove(t.as_slice()).map_err(dberr)?.is_some() {
                        report.receipts += 1;
                    }
                }
                blocks.retain_in(lo..hi, |_, _| false).map_err(dberr)?;
                // The first kept row may elide its links to the row just removed.
                let first = match (&links, blocks.get(hi).map_err(dberr)?) {
                    (Some(l), Some(v)) => Some(unelide(v.value(), l).ok_or(StoreError::Corrupt("block summary"))?),
                    _ => None,
                };
                if let Some(v) = first {
                    blocks.insert(hi, v.as_slice()).map_err(dberr)?;
                }
                let mut proofs = tx.open_table(PROOFS).map_err(dberr)?;
                let before = proofs.len().map_err(dberr)?;
                proofs.retain_in(lo..hi, |_, _| false).map_err(dberr)?;
                report.proofs += before - proofs.len().map_err(dberr)?;
                let mut era_roots = tx.open_table(ERA_ROOTS).map_err(dberr)?;
                for (e, r) in roots.iter().filter(|(e, _)| (lo / len..hi / len).contains(e)) {
                    era_roots.insert(*e, r.as_slice()).map_err(dberr)?;
                }
                tx.open_table(META).map_err(dberr)?.insert(PRUNED_BELOW, hi.to_be_bytes().as_slice()).map_err(dberr)?;
            }
            tx.commit().map_err(dberr)?;
            lo = hi;
        }
        Ok(report)
    }

    /// Storage use per table and for the whole file.
    pub fn stats(&self) -> Result<StoreStats, StoreError> {
        fn one<K: redb::Key + 'static, V: redb::Value + 'static>(
            tx: &redb::WriteTransaction,
            def: TableDefinition<K, V>,
            name: &'static str,
        ) -> Result<TableUse, StoreError> {
            let t = tx.open_table(def).map_err(dberr)?;
            let st = t.stats().map_err(dberr)?;
            Ok(TableUse {
                name,
                entries: t.len().map_err(dberr)?,
                stored: st.stored_bytes(),
                metadata: st.metadata_bytes(),
                fragmented: st.fragmented_bytes(),
                pages: st.leaf_pages() + st.branch_pages(),
            })
        }
        let tx = self.db.begin_write().map_err(dberr)?;
        let tables = vec![
            one(&tx, STATE, "state")?,
            one(&tx, CODE, "code")?,
            one(&tx, BLOCKS, "blocks")?,
            one(&tx, RECEIPTS, "receipts")?,
            one(&tx, META, "meta")?,
            one(&tx, PROOFS, "proofs")?,
            one(&tx, REWARDS, "rewards")?,
            one(&tx, ERA_BLOCKS, "era_blocks")?,
            one(&tx, ERA_ROOTS, "era_roots")?,
        ];
        let db = tx.stats().map_err(dberr)?;
        let out = StoreStats {
            tables,
            allocated_pages: db.allocated_pages(),
            page_size: db.page_size() as u64,
            stored: db.stored_bytes(),
            metadata: db.metadata_bytes(),
            fragmented: db.fragmented_bytes(),
        };
        tx.abort().map_err(dberr)?;
        Ok(out)
    }

    /// Give free pages back to the file system (redb compaction).
    pub fn compact(&mut self) -> Result<bool, StoreError> {
        self.db.compact().map_err(dberr)
    }

    /// Persist one finalized block atomically (durable: the commit fsyncs).
    pub fn commit(&self, c: Commit<'_>) -> Result<(), StoreError> {
        self.write(c, redb::Durability::Immediate)
    }

    /// The same write without the fsync, while a certified backlog is being
    /// replayed (`Chain::relaxed`): redb holds it until the next durable
    /// commit, so a crash loses only the blocks after the last one — every
    /// block is re-fetchable, so they simply replay.
    pub fn commit_relaxed(&self, c: Commit<'_>) -> Result<(), StoreError> {
        self.write(c, redb::Durability::None)
    }

    fn write(&self, c: Commit<'_>, durability: redb::Durability) -> Result<(), StoreError> {
        let mut tx = self.db.begin_write().map_err(dberr)?;
        tx.set_durability(durability);
        {
            let mut state = tx.open_table(STATE).map_err(dberr)?;
            for (k, v) in &c.diff.writes {
                match v {
                    Some(v) => state.insert(k.as_slice(), v.as_slice()).map_err(dberr)?,
                    None => state.remove(k.as_slice()).map_err(dberr)?,
                };
            }
            let mut code = tx.open_table(CODE).map_err(dberr)?;
            for (h, bytes) in &c.diff.codes {
                code.insert(h.as_slice(), bytes.as_ref()).map_err(dberr)?;
            }
            let mut blocks = tx.open_table(BLOCKS).map_err(dberr)?;
            let previous = match c.height.checked_sub(1) {
                Some(p) => blocks.get(p).map_err(dberr)?.and_then(|v| summary_links(v.value())),
                None => None,
            };
            blocks.insert(c.height, encode_summary(c.summary, previous.as_ref())?.as_slice()).map_err(dberr)?;
            let mut receipts = tx.open_table(RECEIPTS).map_err(dberr)?;
            for (h, r) in &c.receipts {
                receipts.insert(h.as_slice(), serde_json::to_vec(&(c.height, r)).map_err(dberr)?.as_slice()).map_err(dberr)?;
            }
            let mut meta = tx.open_table(META).map_err(dberr)?;
            meta.insert("height", c.height.to_be_bytes().as_slice()).map_err(dberr)?;
            meta.insert("digest", c.digest.as_slice()).map_err(dberr)?;
            meta.insert("root", c.root.as_slice()).map_err(dberr)?;
            if let Some(h) = c.handoff {
                meta.insert(HANDOFF, serde_json::to_vec(h).map_err(dberr)?.as_slice()).map_err(dberr)?;
            }
            if let Some(s) = c.seed {
                meta.insert(SEED, serde_json::to_vec(s).map_err(dberr)?.as_slice()).map_err(dberr)?;
            }
            meta.insert(HISTORY, serde_json::to_vec(c.history).map_err(dberr)?.as_slice()).map_err(dberr)?;
            meta.insert(SCHEDULE, serde_json::to_vec(c.schedule).map_err(dberr)?.as_slice()).map_err(dberr)?;
            meta.insert(STATEMENT, serde_json::to_vec(c.statement).map_err(dberr)?.as_slice()).map_err(dberr)?;
            if let Some(s) = &c.staged {
                tx.open_table(ERA_BLOCKS).map_err(dberr)?.insert(c.height, s.block).map_err(dberr)?;
                if let Some(start) = s.era_start {
                    let key = era_start_key(c.height / aether_state::mmr::ERA_LEN);
                    meta.insert(key.as_str(), serde_json::to_vec(start).map_err(dberr)?.as_slice()).map_err(dberr)?;
                }
            }
        }
        tx.commit().map_err(dberr)
    }

    /// Height and block hash of the last persisted finalized block (cheap; no state rebuild).
    pub fn head(&self) -> Result<Option<(u64, [u8; 32])>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let meta = tx.open_table(META).map_err(dberr)?;
        let Some(height) = meta.get("height").map_err(dberr)? else { return Ok(None) };
        let height = u64::from_be_bytes(height.value().try_into().map_err(|_| StoreError::Corrupt("height"))?);
        let digest: [u8; 32] =
            meta.get("digest").map_err(dberr)?.ok_or(StoreError::Corrupt("digest"))?.value().try_into().map_err(|_| StoreError::Corrupt("digest"))?;
        Ok(Some((height, digest)))
    }

    /// The last checkpoint, with its state rebuilt and its root checked.
    pub fn load(&self) -> Result<Option<Checkpoint>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let meta = tx.open_table(META).map_err(dberr)?;
        let Some(height) = meta.get("height").map_err(dberr)? else { return Ok(None) };
        let height = u64::from_be_bytes(height.value().try_into().map_err(|_| StoreError::Corrupt("height"))?);
        let digest: [u8; 32] =
            meta.get("digest").map_err(dberr)?.ok_or(StoreError::Corrupt("digest"))?.value().try_into().map_err(|_| StoreError::Corrupt("digest"))?;
        let stored: [u8; 32] =
            meta.get("root").map_err(dberr)?.ok_or(StoreError::Corrupt("root"))?.value().try_into().map_err(|_| StoreError::Corrupt("root"))?;
        let stored = B256::from(stored);
        let handoff = match meta.get(HANDOFF).map_err(dberr)? {
            Some(v) => Some(serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("handoff"))?),
            None => None,
        };
        let history = match meta.get(HISTORY).map_err(dberr)? {
            Some(v) => serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("history"))?,
            None => return Err(StoreError::Corrupt("history")),
        };
        let schedule = match meta.get(SCHEDULE).map_err(dberr)? {
            Some(v) => serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("schedule"))?,
            None => return Err(StoreError::Corrupt("schedule")),
        };
        // Written from protocol 2 on; a store from before has none (a protocol-1 head).
        let statement = match meta.get(STATEMENT).map_err(dberr)? {
            Some(v) => serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("statement"))?,
            None => Default::default(),
        };
        let seed = match meta.get(SEED).map_err(dberr)? {
            Some(v) => Some(serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("seed"))?),
            None => None,
        };

        let mut entries = Vec::new();
        for row in tx.open_table(STATE).map_err(dberr)?.iter().map_err(dberr)? {
            let (k, v) = row.map_err(dberr)?;
            let k: [u8; 32] = k.value().try_into().map_err(|_| StoreError::Corrupt("state key"))?;
            let v: [u8; 32] = v.value().try_into().map_err(|_| StoreError::Corrupt("state value"))?;
            entries.push((k, v));
        }
        let mut codes = BTreeMap::new();
        for row in tx.open_table(CODE).map_err(dberr)?.iter().map_err(dberr)? {
            let (k, v) = row.map_err(dberr)?;
            let k: [u8; 32] = k.value().try_into().map_err(|_| StoreError::Corrupt("code hash"))?;
            codes.insert(B256::from(k), Bytes::copy_from_slice(v.value()));
        }
        let state = WorldState::from_parts(entries, codes);
        if state.root() != stored {
            return Err(StoreError::RootMismatch { height, stored, rebuilt: state.root() });
        }

        let mut blocks = BTreeMap::new();
        for row in tx.open_table(BLOCKS).map_err(dberr)?.iter().map_err(dberr)? {
            let (h, v) = row.map_err(dberr)?;
            let previous = h.value().checked_sub(1).and_then(|p| blocks.get(&p));
            let s = decode_summary(v.value(), h.value(), previous).ok_or(StoreError::Corrupt("block summary"))?;
            blocks.insert(h.value(), s);
        }
        let mut receipts = HashMap::new();
        for row in tx.open_table(RECEIPTS).map_err(dberr)?.iter().map_err(dberr)? {
            let (k, v) = row.map_err(dberr)?;
            let k: [u8; 32] = k.value().try_into().map_err(|_| StoreError::Corrupt("receipt key"))?;
            receipts.insert(B256::from(k), serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("receipt"))?);
        }
        drop(tx);
        let pruned_below = self.pruned_below()?;
        let era_roots = self.era_roots()?;
        Ok(Some(Checkpoint { height, digest, state, blocks, receipts, handoff, seed, history, schedule, statement, pruned_below, era_roots }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_types::{Address, FeeVector, GasVector};

    fn summary(height: u64, parent: &str, parent_state_root: B256) -> BlockSummary {
        BlockSummary {
            height,
            hash: hex::encode([height as u8 + 1; 32]),
            parent: parent.to_string(),
            timestamp_ms: 1_790_000_000_000 + height * 1000,
            proposer: Address::repeat_byte(9),
            state_root: B256::repeat_byte(height as u8 + 100),
            parent_state_root,
            txs: vec![B256::repeat_byte(3)],
            gas_used: 21_000,
            prove_gas: 5,
            base_fee: FeeVector { exec: 7, state: 8, prove: 1 << 100 },
            excess: GasVector { exec: 1, state: 2, prove: 3 },
        }
    }

    fn same(a: &BlockSummary, b: &BlockSummary) {
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }

    #[test]
    fn packed_summaries_round_trip_and_elide_links() {
        let s0 = summary(0, &hex::encode([0xee; 32]), B256::ZERO);
        let s1 = summary(1, &s0.hash, s0.state_root);
        let p0 = encode_summary(&s0, None).unwrap();
        let p1 = encode_summary(&s1, summary_links(&p0).as_ref()).unwrap();
        let json = serde_json::to_vec(&s1).unwrap().len();
        assert!(p1.len() * 3 < json, "packed {} vs json {json}", p1.len());
        assert!(p1.len() + 60 < p0.len(), "links to the previous row are elided");
        let d0 = decode_summary(&p0, 0, None).unwrap();
        same(&d0, &s0);
        same(&decode_summary(&p1, 1, Some(&d0)).unwrap(), &s1);
        assert!(decode_summary(&p1, 1, None).is_none(), "an elided link needs the previous row");
        // A summary that does not fit the packed form stays JSON.
        let odd = BlockSummary { hash: "not hex".into(), ..s1.clone() };
        let j = encode_summary(&odd, None).unwrap();
        assert_eq!(j[0], b'{');
        same(&decode_summary(&j, 1, None).unwrap(), &odd);
    }

    #[test]
    fn stores_with_json_rows_still_load_and_new_rows_are_packed() {
        let dir = std::env::temp_dir().join(format!("aether-store-packed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        let store = Store::open(&path).unwrap();
        let state = WorldState::default();
        let s0 = BlockSummary { state_root: state.root(), ..summary(0, &hex::encode([0xee; 32]), B256::ZERO) };
        let commit = |store: &Store, s: &BlockSummary| {
            store
                .commit(Commit {
                    height: s.height,
                    digest: [s.height as u8 + 1; 32],
                    root: state.root(),
                    diff: state.journal(),
                    summary: s,
                    receipts: vec![],
                    handoff: None,
                    seed: None,
                    history: &Default::default(),
                    schedule: &Default::default(),
                    statement: &Default::default(),
                    staged: None,
                })
                .unwrap()
        };
        commit(&store, &s0);
        // Block 0 as an older binary wrote it: JSON.
        let tx = store.db.begin_write().unwrap();
        tx.open_table(BLOCKS).unwrap().insert(0, serde_json::to_vec(&s0).unwrap().as_slice()).unwrap();
        tx.commit().unwrap();
        let s1 = BlockSummary { state_root: state.root(), ..summary(1, &s0.hash, s0.state_root) };
        commit(&store, &s1);
        drop(store);
        let cp = Store::open(&path).unwrap().load().unwrap().unwrap();
        same(&cp.blocks[&0], &s0);
        same(&cp.blocks[&1], &s1);
        let store = Store::open(&path).unwrap();
        let tx = store.db.begin_read().unwrap();
        let t = tx.open_table(BLOCKS).unwrap();
        assert_eq!(t.get(0).unwrap().unwrap().value()[0], b'{');
        assert_eq!(t.get(1).unwrap().unwrap().value()[0], PACKED);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
