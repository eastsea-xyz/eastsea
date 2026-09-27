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
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const STATE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("state");
const CODE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("code");
const BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const RECEIPTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("receipts");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
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
}

pub struct Store {
    db: Database,
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
        tx.commit().map_err(dberr)?;
        Ok(Store { db })
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

    pub fn put_proof(&self, height: u64, proof: &[u8]) -> Result<(), StoreError> {
        let tx = self.db.begin_write().map_err(dberr)?;
        tx.open_table(PROOFS).map_err(dberr)?.insert(height, proof).map_err(dberr)?;
        tx.commit().map_err(dberr)
    }

    pub fn proof(&self, height: u64) -> Result<Option<Vec<u8>>, StoreError> {
        let tx = self.db.begin_read().map_err(dberr)?;
        let t = tx.open_table(PROOFS).map_err(dberr)?;
        Ok(t.get(height).map_err(dberr)?.map(|v| v.value().to_vec()))
    }

    /// Persist one finalized block atomically.
    pub fn commit(&self, c: Commit<'_>) -> Result<(), StoreError> {
        let tx = self.db.begin_write().map_err(dberr)?;
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
            blocks.insert(c.height, serde_json::to_vec(c.summary).map_err(dberr)?.as_slice()).map_err(dberr)?;
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
        let statement = match meta.get(STATEMENT).map_err(dberr)? {
            Some(v) => serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("statement"))?,
            None => return Err(StoreError::Corrupt("statement")),
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
            blocks.insert(h.value(), serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("block summary"))?);
        }
        let mut receipts = HashMap::new();
        for row in tx.open_table(RECEIPTS).map_err(dberr)?.iter().map_err(dberr)? {
            let (k, v) = row.map_err(dberr)?;
            let k: [u8; 32] = k.value().try_into().map_err(|_| StoreError::Corrupt("receipt key"))?;
            receipts.insert(B256::from(k), serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("receipt"))?);
        }
        Ok(Some(Checkpoint { height, digest, state, blocks, receipts, handoff, seed, history, schedule, statement }))
    }
}
