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
use std::sync::{Arc, Mutex};

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
const UPGRADE_NOTICES: &str = "upgrade_notices";
/// The head block's statement commitment and escrow share (JSON).
const STATEMENT: &str = "statement";
/// Finality proofs a follower verified (`aether_getFinalized` JSON by height):
/// history it keeps serving, also after it becomes a voting node.
const PROOFS: TableDefinition<u64, &[u8]> = TableDefinition::new("proofs");
/// Rewards paid to provers, kept by this node for tax records: (prover ‖ paid-in
/// height) -> JSON. Not consensus data; nothing is ever dropped.
const REWARDS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rewards");
/// Address || height || transaction index -> one finalized activity row.
const ACCOUNT_HISTORY: TableDefinition<&[u8], &[u8]> = TableDefinition::new("account_history");
/// Height -> concatenated account-history keys, so pruning is proportional to
/// the removed blocks rather than every account's lifetime activity.
const ACCOUNT_BLOCK_KEYS: TableDefinition<u64, &[u8]> = TableDefinition::new("account_block_keys");
const ACCOUNT_HISTORY_SINCE: &str = "account_history_since";

/// The schema version of the tables above, as this binary writes them
/// (docs/design/24-self-healing.md, red team #15). Version 1 is the layout as
/// it stands: nothing has migrated yet, so [`migrations`] is empty. The first
/// change to a table's meaning bumps this and registers a migration step.
pub const CURRENT_SCHEMA: u32 = 1;
/// `meta.schema_version` (u32, big-endian): the layout that wrote the data.
/// Absent on a database from before this change: those are version 1, adopted
/// in place — stamped, never rejected, never rebuilt.
const SCHEMA_VERSION: &str = "schema_version";
/// `meta.min_read_version` (u32, big-endian): the oldest schema a binary may
/// run and still read this database. An older binary — the app rolled back to
/// `aether.prev` after crashes — refuses with [`StoreError::TooNew`] instead
/// of misreading rows a newer layout wrote.
const MIN_READ_VERSION: &str = "min_read_version";
/// `meta.migration` (JSON): the migration step marker — the step in flight
/// and whether it finished. Marker and data commit in one redb write
/// transaction, so an interrupted migration leaves the database at a recorded
/// sub-step boundary and the next open resumes exactly there.
const MIGRATION: &str = "migration";

#[derive(Debug)]
pub enum StoreError {
    Db(String),
    /// A disk failure (ENOSPC, EIO…): redb is unusable until the database is
    /// closed and re-opened ([`Store::reopen`]). Not corruption: the file
    /// stays, and the node goes on once the disk takes writes again.
    Io(String),
    /// The database file does not open as a database (garbled or truncated):
    /// the data is gone from this node's point of view, and it re-syncs
    /// ([`crate::follow::open_store`] moves the file aside).
    Unreadable(String),
    /// The store is being re-opened after an I/O error right now; retrying
    /// shortly succeeds. Never surfaces for long.
    Reopening,
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
    /// The database's schema is newer than this binary reads
    /// (`min_read_version` above [`CURRENT_SCHEMA`]): intact data this binary
    /// must not touch — typically the app rolled back to `aether.prev` after
    /// crashes. Never corruption (`crate::follow::is_corruption`): the file is
    /// never moved aside, never deleted, never written. The node stops with
    /// the update-required exit code (3) so the app installs the newer signed
    /// release instead of rolling back over the data.
    TooNew {
        /// The schema version the database records, if it decodes.
        found: Option<u32>,
        /// The minimum reader it declares, if that decodes.
        min_read: Option<u32>,
        /// The schema this binary implements.
        ours: u32,
    },
}

impl StoreError {
    /// A disk problem (as opposed to bad data): the database is re-opened
    /// with backoff, and a restart retries the same file.
    pub fn is_disk(&self) -> bool {
        matches!(self, StoreError::Io(_) | StoreError::Reopening)
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for StoreError {}

fn dberr<E: Into<redb::Error>>(e: E) -> StoreError {
    match e.into() {
        // A file that ends mid-header or holds no database: not a disk
        // problem, and no re-open will ever fix it.
        redb::Error::Io(e) => match e.kind() {
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData => {
                StoreError::Unreadable(e.to_string())
            }
            _ => StoreError::Io(e.to_string()),
        },
        redb::Error::PreviousIo => {
            StoreError::Io("an earlier I/O error made the database unusable until it is re-opened".into())
        }
        redb::Error::Corrupted(m) => StoreError::Unreadable(m),
        e => StoreError::Db(e.to_string()),
    }
}

/// The process exits with this code when its store never becomes writable
/// again (docs/design/24-self-healing.md layer 1): the app then restarts the
/// node — from the last durable state, or a certified snapshot — and tells
/// the user what to do (free disk space) if it keeps failing.
pub const EXIT_STORAGE: i32 = 4;

/// How a node re-opens its store after an I/O error: the first try after
/// `backoff`, doubling to a minute, `attempts` times in all (~3 minutes)
/// before the process gives up ([`EXIT_STORAGE`]). `AETHER_STORE_RECOVERY`
/// (hidden, `<ms>,<attempts>`) overrides both for the fault tests.
#[derive(Clone, Copy, Debug)]
pub struct Recovery {
    pub backoff: std::time::Duration,
    pub attempts: u32,
}

impl Default for Recovery {
    fn default() -> Self {
        Recovery { backoff: std::time::Duration::from_secs(1), attempts: 8 }
    }
}

impl Recovery {
    pub fn from_env() -> Self {
        let mut r = Self::default();
        if let Some(v) = std::env::var("AETHER_STORE_RECOVERY").ok().as_deref() {
            let mut it = v.split(',');
            if let (Some(ms), Some(n)) = (it.next().and_then(|m| m.parse().ok()), it.next().and_then(|a| a.parse().ok())) {
                r.backoff = std::time::Duration::from_millis(ms);
                r.attempts = n;
            }
        }
        r
    }

    /// Re-open the store until the disk takes a write again, backing off
    /// between tries. The last error comes back when it never does.
    pub fn reopen(&self, store: &Store) -> Result<(), StoreError> {
        let mut wait = self.backoff;
        let mut last = StoreError::Db("never tried".into());
        for attempt in 1..=self.attempts {
            // Each try is work (red team #2): a node re-opening its database
            // under a rising backoff must not read as stuck between tries.
            crate::chain::tick();
            tracing::warn!(attempt, of = self.attempts, "re-opening the store database");
            match store.reopen() {
                Ok(()) => {
                    tracing::info!("the store database is open and takes writes again");
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(%e, "could not re-open the store database");
                    last = e;
                }
            }
            if attempt < self.attempts {
                std::thread::sleep(wait);
                wait = (wait * 2).min(std::time::Duration::from_secs(60));
            }
        }
        Err(last)
    }
}

/// The migration step marker (`meta.migration`, JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
struct MigrationMarker {
    name: String,
    step: u32,
    done: bool,
}

/// What `meta` records about the database's schema.
#[derive(Debug, PartialEq, Eq)]
enum Schema {
    /// No version recorded: a database from before this change, or a fresh
    /// file. It is version 1 — adopted in place, never rejected, never rebuilt.
    Unrecorded,
    /// A version row that does not decode as this binary writes it: a layout
    /// it does not understand. That reads as too new, never as corruption —
    /// the file is intact data that must not be destroyed for decoding wrong.
    Undecodable,
    /// `schema_version` decodes; `min_read_version` may be absent (then the
    /// version itself is the minimum a reader must support).
    Recorded { version: u32, min_read: Option<u32> },
}

/// One registered schema migration step: how a database at version `to - 1`
/// becomes one at `to`. None exist yet — [`CURRENT_SCHEMA`] is 1 and
/// [`migrations`] is empty; the first change to a table's meaning registers a
/// step there and bumps the version.
pub struct Migration {
    /// The schema version this step migrates the database to.
    pub to: u32,
    /// Its name, recorded in the step marker.
    pub name: &'static str,
    /// Runs the step's sub-steps, starting at `resume` (0 on a fresh run; the
    /// recorded marker's step after an interrupted one). Every sub-step is one
    /// redb write transaction and must be idempotent: an interrupted run
    /// resumes at the recorded sub-step, which re-runs it.
    pub run: fn(&MigrationStep<'_>, u32) -> Result<(), StoreError>,
}

/// The registered schema migrations, in order. Empty while
/// [`CURRENT_SCHEMA`] is 1; the mechanism's tests prove it with a test-only
/// list, and release code opens with exactly this one.
pub fn migrations() -> &'static [Migration] {
    &[]
}

/// What a migration step runs its sub-steps through: the database being
/// migrated, before the `Store` is handed out.
pub struct MigrationStep<'a> {
    db: &'a Database,
    name: &'static str,
    /// The last sub-step this step ran, for the done marker.
    last: std::cell::Cell<u32>,
}

impl MigrationStep<'_> {
    /// One sub-step: the marker `{name, step, done: false}` and `body`'s
    /// writes commit in one transaction, so a crash leaves either both or
    /// neither — never data the marker does not describe.
    pub fn sub(&self, step: u32, body: impl FnOnce(&redb::WriteTransaction) -> Result<(), StoreError>) -> Result<(), StoreError> {
        let tx = self.db.begin_write().map_err(dberr)?;
        {
            let marker = serde_json::to_vec(&MigrationMarker { name: self.name.to_string(), step, done: false })
                .map_err(|e| StoreError::Db(e.to_string()))?;
            tx.open_table(META).map_err(dberr)?.insert(MIGRATION, marker.as_slice()).map_err(dberr)?;
        }
        body(&tx)?;
        tx.commit().map_err(dberr)?;
        self.last.set(step);
        // A long migration is progress, not a stall (red team #2): every
        // committed sub-step counts as activity while the height is frozen.
        crate::chain::tick();
        Ok(())
    }

    /// The step finished: the done marker and the new versions
    /// (`schema_version = to`, `min_read_version = to`) commit in one
    /// transaction. From here a binary older than `to` refuses with
    /// [`StoreError::TooNew`] instead of misreading the migrated data.
    pub fn finish(&self, to: u32) -> Result<(), StoreError> {
        let tx = self.db.begin_write().map_err(dberr)?;
        {
            let mut meta = tx.open_table(META).map_err(dberr)?;
            let marker = serde_json::to_vec(&MigrationMarker { name: self.name.to_string(), step: self.last.get(), done: true })
                .map_err(|e| StoreError::Db(e.to_string()))?;
            meta.insert(MIGRATION, marker.as_slice()).map_err(dberr)?;
            meta.insert(SCHEMA_VERSION, to.to_be_bytes().as_slice()).map_err(dberr)?;
            meta.insert(MIN_READ_VERSION, to.to_be_bytes().as_slice()).map_err(dberr)?;
        }
        tx.commit().map_err(dberr)
    }
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
    pub upgrade_notices: Vec<crate::upgrade::SignedUpgrade>,
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
    pub upgrade_notices: &'a [crate::upgrade::SignedUpgrade],
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
        return serde_json::to_vec(s).map_err(|e| StoreError::Db(e.to_string()));
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
    out.extend(postcard::to_allocvec(&p).map_err(|e| StoreError::Db(e.to_string()))?);
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
    /// The open database, or `None` only in the short window while it is
    /// being re-opened after an I/O error. A transaction that was already
    /// running keeps the old database alive until it ends (redb keeps the
    /// file open per transaction, not per `Database` handle).
    db: Mutex<Option<Database>>,
    /// How the database file is opened; `reopen` goes through the same hook.
    /// The self-healing fault tests (docs/design/24-self-healing.md) inject a
    /// disk that fails through it.
    open: Arc<dyn Fn(&Path) -> Result<Database, StoreError> + Send + Sync>,
    /// The database file, re-opened on an I/O error.
    path: std::path::PathBuf,
    /// The directory the store lives in (era files go in its `eras` folder).
    dir: std::path::PathBuf,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::open_migrating(path, Arc::new(|p| Database::create(p).map_err(dberr)), migrations())
    }

    /// `open` with a custom way to open the database file.
    pub fn open_with(
        path: &Path,
        open: Arc<dyn Fn(&Path) -> Result<Database, StoreError> + Send + Sync>,
    ) -> Result<Self, StoreError> {
        Self::open_migrating(path, open, migrations())
    }

    /// `open` with an explicit migration list: the mechanism's tests prove it
    /// with a test-only list. No release path passes one that differs from
    /// [`migrations`].
    pub fn open_migrating(
        path: &Path,
        open: Arc<dyn Fn(&Path) -> Result<Database, StoreError> + Send + Sync>,
        migrations: &[Migration],
    ) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let db = Self::prepare(open(path)?, migrations)?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut store = Store { db: Mutex::new(Some(db)), open, path: path.to_path_buf(), dir };
        store.compact_if_sparse(path)?;
        Ok(store)
    }

    /// The tables this store uses: a fresh file gets them, an existing one
    /// keeps its rows.
    fn init_tables(tx: &redb::WriteTransaction) -> Result<(), StoreError> {
        for t in [STATE, CODE, RECEIPTS] {
            tx.open_table(t).map_err(dberr)?;
        }
        tx.open_table(BLOCKS).map_err(dberr)?;
        tx.open_table(META).map_err(dberr)?;
        tx.open_table(PROOFS).map_err(dberr)?;
        tx.open_table(REWARDS).map_err(dberr)?;
        tx.open_table(ACCOUNT_HISTORY).map_err(dberr)?;
        tx.open_table(ACCOUNT_BLOCK_KEYS).map_err(dberr)?;
        Ok(())
    }

    /// Read the schema `meta` records, before anything writes to the file.
    fn read_schema(db: &Database) -> Result<Schema, StoreError> {
        let tx = db.begin_read().map_err(dberr)?;
        let meta = match tx.open_table(META) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Schema::Unrecorded),
            Err(e) => return Err(dberr(e)),
        };
        // A row that is not a big-endian u32 is data a newer layout wrote.
        let word = |key: &str| -> Result<Option<Option<u32>>, StoreError> {
            Ok(meta.get(key).map_err(dberr)?.map(|v| v.value().try_into().ok().map(u32::from_be_bytes)))
        };
        Ok(match (word(SCHEMA_VERSION)?, word(MIN_READ_VERSION)?) {
            (None, None) => Schema::Unrecorded,
            (Some(Some(version)), min_read) => Schema::Recorded { version, min_read: min_read.flatten() },
            _ => Schema::Undecodable,
        })
    }

    /// Give a freshly opened database its tables and enforce the recorded
    /// schema (docs/design/24-self-healing.md, red team #15), all before the
    /// store is handed out:
    ///
    /// - nothing recorded — a database from before this change — is version 1:
    ///   stamped in place, never rejected, never rebuilt;
    /// - `min_read_version` above [`CURRENT_SCHEMA`] — data a newer binary
    ///   wrote — is refused with [`StoreError::TooNew`] before a single write,
    ///   so the caller can stop without touching it;
    /// - a version below [`CURRENT_SCHEMA`] runs the registered migration
    ///   steps in order, resuming an interrupted one at its recorded sub-step.
    fn prepare(db: Database, migrations: &[Migration]) -> Result<Database, StoreError> {
        match Self::read_schema(&db)? {
            Schema::Unrecorded => {
                tracing::info!(
                    version = CURRENT_SCHEMA,
                    "no schema version recorded: adopting this database as version {CURRENT_SCHEMA} in place"
                );
                let tx = db.begin_write().map_err(dberr)?;
                Self::init_tables(&tx)?;
                {
                    let mut meta = tx.open_table(META).map_err(dberr)?;
                    meta.insert(SCHEMA_VERSION, CURRENT_SCHEMA.to_be_bytes().as_slice()).map_err(dberr)?;
                    meta.insert(MIN_READ_VERSION, CURRENT_SCHEMA.to_be_bytes().as_slice()).map_err(dberr)?;
                }
                tx.commit().map_err(dberr)?;
            }
            Schema::Undecodable => {
                return Err(StoreError::TooNew { found: None, min_read: None, ours: CURRENT_SCHEMA });
            }
            Schema::Recorded { version, min_read } => {
                if min_read.unwrap_or(version) > CURRENT_SCHEMA {
                    return Err(StoreError::TooNew { found: Some(version), min_read, ours: CURRENT_SCHEMA });
                }
                let tx = db.begin_write().map_err(dberr)?;
                Self::init_tables(&tx)?;
                tx.commit().map_err(dberr)?;
                if version < CURRENT_SCHEMA {
                    Self::migrate(&db, version, migrations)?;
                }
            }
        }
        Ok(db)
    }

    /// Run the registered migration steps that take the database from `from`
    /// to [`CURRENT_SCHEMA`], in order. A step interrupted mid-run left its
    /// marker at the sub-step in flight: resume exactly there — every
    /// sub-step is idempotent, so re-running the recorded one is safe.
    fn migrate(db: &Database, from: u32, migrations: &[Migration]) -> Result<(), StoreError> {
        let mut next = from + 1;
        for m in migrations {
            if m.to <= from {
                continue; // the recorded version says an earlier run applied it
            }
            if m.to != next {
                return Err(StoreError::Db(format!("the migration list skips version {next}")));
            }
            let resume = match Self::marker(db)? {
                // A step in flight for this migration: resume at its recorded
                // sub-step.
                Some(marker) if !marker.done && marker.name == m.name => marker.step,
                _ => 0,
            };
            tracing::info!(migration = m.name, from, to = m.to, resume, "running a schema migration");
            let step = MigrationStep { db, name: m.name, last: std::cell::Cell::new(0) };
            (m.run)(&step, resume)?;
            step.finish(m.to)?;
            next = m.to + 1;
        }
        if next != CURRENT_SCHEMA + 1 {
            return Err(StoreError::Db(format!("the migration list does not reach version {CURRENT_SCHEMA}")));
        }
        Ok(())
    }

    /// The migration step marker, if one is recorded. A marker that does not
    /// decode is a restart from the first sub-step (they are idempotent),
    /// never a refusal: this is recovery data, and the file is never moved
    /// aside for it.
    fn marker(db: &Database) -> Result<Option<MigrationMarker>, StoreError> {
        let tx = db.begin_read().map_err(dberr)?;
        let meta = match tx.open_table(META) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(dberr(e)),
        };
        let Some(v) = meta.get(MIGRATION).map_err(dberr)? else { return Ok(None) };
        match serde_json::from_slice(v.value()) {
            Ok(marker) => Ok(Some(marker)),
            Err(_) => {
                tracing::warn!("the migration marker does not decode; restarting the step from its first sub-step");
                Ok(None)
            }
        }
    }

    /// Close the database and open the file again, then check the disk takes
    /// a write: after an I/O error redb refuses every operation ("Previous
    /// I/O error… close and re-open"), and a full disk only heals once space
    /// frees. Blocks committed without an fsync (a replayed backlog) may not
    /// have made it to the file: callers roll the chain back to the last
    /// durable checkpoint (`crate::follow::recover`).
    pub fn reopen(&self) -> Result<(), StoreError> {
        {
            let mut db = self.db.lock().expect("store lock");
            drop(db.take()); // the file lock goes with the old database
            // The schema check runs again too: a migration interrupted by the
            // I/O error resumes at its recorded sub-step.
            *db = Some(Self::prepare((self.open)(&self.path)?, migrations())?);
        }
        // The probe: a durable write, so a still-full disk fails here instead
        // of on the next block.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        self.put_meta("storage", &now.as_secs().to_be_bytes())
    }

    /// A write transaction on the open database. While it is being re-opened
    /// after an I/O error this is [`StoreError::Reopening`]; the caller
    /// retries, as it does for any I/O failure.
    fn write_tx(&self) -> Result<redb::WriteTransaction, StoreError> {
        match &*self.db.lock().expect("store lock") {
            Some(db) => db.begin_write().map_err(dberr),
            None => Err(StoreError::Reopening),
        }
    }

    fn read_tx(&self) -> Result<redb::ReadTransaction, StoreError> {
        match &*self.db.lock().expect("store lock") {
            Some(db) => db.begin_read().map_err(dberr),
            None => Err(StoreError::Reopening),
        }
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
            let tx = self.write_tx()?;
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
        let tx = self.write_tx()?;
        tx.open_table(META).map_err(dberr)?.insert(key, value).map_err(dberr)?;
        tx.commit().map_err(dberr)
    }

    /// The database file (its directory is where a recovery's space is
    /// checked, `crate::follow::require_space`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn meta(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let tx = self.read_tx()?;
        let t = tx.open_table(META).map_err(dberr)?;
        Ok(t.get(key).map_err(dberr)?.map(|v| v.value().to_vec()))
    }

    /// Record a reward paid to `prover` in block `height` (one per proven block: `proven`).
    pub fn put_reward(&self, prover: &[u8; 20], height: u64, proven: u64, record: &[u8]) -> Result<(), StoreError> {
        let mut key = prover.to_vec();
        key.extend_from_slice(&height.to_be_bytes());
        key.extend_from_slice(&proven.to_be_bytes());
        let tx = self.write_tx()?;
        tx.open_table(REWARDS).map_err(dberr)?.insert(key.as_slice(), record).map_err(dberr)?;
        tx.commit().map_err(dberr)
    }

    /// Every reward recorded for `prover`, oldest first.
    /// The newest `limit` rewards of `prover`, oldest first.
    pub fn rewards(&self, prover: &[u8; 20], limit: usize) -> Result<Vec<Vec<u8>>, StoreError> {
        let tx = self.read_tx()?;
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

    /// One page of `prover`'s rewards, newest first. `before` is a cursor this
    /// table handed out (hex of the last key served); the page that follows it
    /// continues strictly older. `total` is every reward recorded for `prover`,
    /// so a wallet can say "N of M" instead of silently truncating.
    pub fn rewards_page(
        &self,
        prover: &[u8; 20],
        before: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Vec<u8>>, Option<String>, u64), StoreError> {
        let tx = self.read_tx()?;
        let t = tx.open_table(REWARDS).map_err(dberr)?;
        let mut start = prover.to_vec();
        start.extend_from_slice(&[0; 16]);
        let mut end = prover.to_vec();
        end.extend_from_slice(&[0xff; 16]);
        let upper = match before {
            Some(cursor) => {
                let key = hex::decode(cursor.strip_prefix("0x").unwrap_or(cursor))
                    .map_err(|_| StoreError::Db("invalid rewards cursor".into()))?;
                if key.len() != 36 || !key.starts_with(prover) {
                    return Err(StoreError::Db("rewards cursor belongs to another address".into()));
                }
                key
            }
            None => end.clone(),
        };
        let mut total = 0u64;
        for row in t.range(start.as_slice()..=end.as_slice()).map_err(dberr)? {
            row.map_err(dberr)?;
            total += 1;
        }
        let mut rows = Vec::new();
        let mut next_cursor = None;
        for row in t.range(start.as_slice()..upper.as_slice()).map_err(dberr)?.rev() {
            let (key, value) = row.map_err(dberr)?;
            if rows.len() == limit {
                next_cursor = Some(hex::encode(key.value()));
                break;
            }
            rows.push(value.value().to_vec());
        }
        Ok((rows, next_cursor, total))
    }

    /// Newest account activity, with an exclusive opaque cursor. Rows are
    /// returned from one read transaction, so pagination cannot see a half
    /// committed block. The index begins at `history_start` on upgraded stores.
    pub fn account_history(&self, address: &[u8; 20], before: Option<&str>, limit: usize) -> Result<crate::account_history::Page, StoreError> {
        let tx = self.read_tx()?;
        let mut start = address.to_vec();
        start.extend_from_slice(&[0; 12]);
        let mut end = address.to_vec();
        end.extend_from_slice(&[0xff; 12]);
        let upper = match before {
            Some(cursor) => {
                let key = hex::decode(cursor.strip_prefix("0x").unwrap_or(cursor)).map_err(|_| StoreError::Db("invalid history cursor".into()))?;
                if key.len() != 32 || !key.starts_with(address) {
                    return Err(StoreError::Db("history cursor belongs to another address".into()));
                }
                key
            }
            None => end,
        };
        let rows = tx.open_table(ACCOUNT_HISTORY).map_err(dberr)?;
        let mut entries = Vec::new();
        let mut next_cursor = None;
        let range = rows.range(start.as_slice()..upper.as_slice()).map_err(dberr)?;
        for row in range.rev() {
            let (key, value) = row.map_err(dberr)?;
            if entries.len() == limit { next_cursor = entries.last().map(|(k, _): &(String, crate::account_history::Entry)| k.clone()); break; }
            let entry = serde_json::from_slice(value.value()).map_err(|_| StoreError::Corrupt("account history row"))?;
            entries.push((hex::encode(key.value()), entry));
        }
        let meta = tx.open_table(META).map_err(dberr)?;
        let read_word = |key| -> Result<Option<u64>, StoreError> {
            let Some(value) = meta.get(key).map_err(dberr)? else { return Ok(None) };
            let bytes: [u8; 8] = value.value().try_into().map_err(|_| StoreError::Corrupt("account history metadata"))?;
            Ok(Some(u64::from_be_bytes(bytes)))
        };
        let since = read_word(ACCOUNT_HISTORY_SINCE)?;
        let pruned = read_word(PRUNED_BELOW)?.unwrap_or(0);
        let head = read_word("height")?.unwrap_or(0);
        Ok(crate::account_history::Page {
            entries: entries.into_iter().map(|(_, e)| e).collect(), next_cursor,
            history_start: since.map(|h| h.max(pruned)).unwrap_or(head.saturating_add(1).max(pruned)),
            indexed_height: since.map(|_| head).unwrap_or(0),
        })
    }

    /// Certificates this node verified, served to wallets and other followers.
    /// Written without the fsync: they are re-fetchable certified data, and
    /// redb persists them with the next durable block commit.
    pub fn put_proof(&self, height: u64, proof: &[u8]) -> Result<(), StoreError> {
        let mut tx = self.write_tx()?;
        tx.set_durability(redb::Durability::None).map_err(dberr)?;
        tx.open_table(PROOFS).map_err(dberr)?.insert(height, proof).map_err(dberr)?;
        tx.commit().map_err(dberr)
    }

    pub fn proof(&self, height: u64) -> Result<Option<Vec<u8>>, StoreError> {
        let tx = self.read_tx()?;
        let t = tx.open_table(PROOFS).map_err(dberr)?;
        Ok(t.get(height).map_err(dberr)?.map(|v| v.value().to_vec()))
    }

    /// Where sealed era files go.
    pub fn era_dir(&self) -> std::path::PathBuf {
        self.dir.join("eras")
    }

    /// Eras with blocks kept for sealing, oldest first.
    pub fn staged_eras(&self) -> Result<Vec<u64>, StoreError> {
        let tx = self.read_tx()?;
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
        let tx = self.read_tx()?;
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
        let tx = self.write_tx()?;
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
        let tx = self.read_tx()?;
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
            let tx = self.write_tx()?;
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
                let mut account_keys = tx.open_table(ACCOUNT_BLOCK_KEYS).map_err(dberr)?;
                let mut account_rows = tx.open_table(ACCOUNT_HISTORY).map_err(dberr)?;
                for h in lo..hi {
                    if let Some(keys) = account_keys.remove(h).map_err(dberr)? {
                        let (chunks, remainder) = keys.value().as_chunks::<32>();
                        if !remainder.is_empty() { return Err(StoreError::Corrupt("account history block keys")); }
                        for key in chunks {
                            account_rows.remove(key.as_slice()).map_err(dberr)?;
                        }
                    }
                }
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
        let tx = self.write_tx()?;
        let tables = vec![
            one(&tx, STATE, "state")?,
            one(&tx, CODE, "code")?,
            one(&tx, BLOCKS, "blocks")?,
            one(&tx, RECEIPTS, "receipts")?,
            one(&tx, META, "meta")?,
            one(&tx, PROOFS, "proofs")?,
            one(&tx, REWARDS, "rewards")?,
            one(&tx, ACCOUNT_HISTORY, "account_history")?,
            one(&tx, ACCOUNT_BLOCK_KEYS, "account_block_keys")?,
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
        match &mut *self.db.lock().expect("store lock") {
            Some(db) => db.compact().map_err(dberr),
            None => Err(StoreError::Reopening),
        }
    }

    /// Persist one finalized block atomically (durable: the commit fsyncs).
    pub fn commit(&self, c: Commit<'_>) -> Result<(), StoreError> {
        self.write(c, &[], redb::Durability::Immediate, false)
    }

    pub fn commit_with_history(&self, c: Commit<'_>, history: &[crate::account_history::Entry], relaxed: bool) -> Result<(), StoreError> {
        self.write(c, history, if relaxed { redb::Durability::None } else { redb::Durability::Immediate }, true)
    }

    /// The same write without the fsync, while a certified backlog is being
    /// replayed (`Chain::relaxed`): redb holds it until the next durable
    /// commit, so a crash loses only the blocks after the last one — every
    /// block is re-fetchable, so they simply replay.
    pub fn commit_relaxed(&self, c: Commit<'_>) -> Result<(), StoreError> {
        self.write(c, &[], redb::Durability::None, false)
    }

    fn write(&self, c: Commit<'_>, history: &[crate::account_history::Entry], durability: redb::Durability, indexing: bool) -> Result<(), StoreError> {
        let mut tx = self.write_tx()?;
        tx.set_durability(durability).map_err(dberr)?;
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
                receipts.insert(h.as_slice(), serde_json::to_vec(&(c.height, r)).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
            }
            let mut keys = Vec::with_capacity(history.len() * 32);
            let mut account_rows = tx.open_table(ACCOUNT_HISTORY).map_err(dberr)?;
            for entry in history {
                let mut key = Vec::with_capacity(32);
                key.extend_from_slice(entry.address.as_slice());
                key.extend_from_slice(&entry.height.to_be_bytes());
                key.extend_from_slice(&entry.tx_index.to_be_bytes());
                account_rows.insert(key.as_slice(), serde_json::to_vec(entry).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
                keys.extend_from_slice(&key);
            }
            if !keys.is_empty() {
                tx.open_table(ACCOUNT_BLOCK_KEYS).map_err(dberr)?.insert(c.height, keys.as_slice()).map_err(dberr)?;
            }
            let mut meta = tx.open_table(META).map_err(dberr)?;
            if indexing && meta.get(ACCOUNT_HISTORY_SINCE).map_err(dberr)?.is_none() {
                meta.insert(ACCOUNT_HISTORY_SINCE, c.height.to_be_bytes().as_slice()).map_err(dberr)?;
            }
            meta.insert("height", c.height.to_be_bytes().as_slice()).map_err(dberr)?;
            meta.insert("digest", c.digest.as_slice()).map_err(dberr)?;
            meta.insert("root", c.root.as_slice()).map_err(dberr)?;
            if let Some(h) = c.handoff {
                meta.insert(HANDOFF, serde_json::to_vec(h).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
            }
            if let Some(s) = c.seed {
                meta.insert(SEED, serde_json::to_vec(s).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
            }
            meta.insert(HISTORY, serde_json::to_vec(c.history).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
            meta.insert(SCHEDULE, serde_json::to_vec(c.schedule).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
            meta.insert(UPGRADE_NOTICES, serde_json::to_vec(c.upgrade_notices).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
            meta.insert(STATEMENT, serde_json::to_vec(c.statement).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
            if let Some(s) = &c.staged {
                tx.open_table(ERA_BLOCKS).map_err(dberr)?.insert(c.height, s.block).map_err(dberr)?;
                if let Some(start) = s.era_start {
                    let key = era_start_key(c.height / aether_state::mmr::ERA_LEN);
                    meta.insert(key.as_str(), serde_json::to_vec(start).map_err(|e| StoreError::Db(e.to_string()))?.as_slice()).map_err(dberr)?;
                }
            }
        }
        tx.commit().map_err(dberr)
    }

    /// The head's integrity at start-up, cheaply (docs/design/24-self-healing.md
    /// layer 1): the checkpoint meta decodes and its block row parses as a
    /// summary. A file that fails this — garbled, truncated, half-written — is
    /// moved aside by `crate::follow::open_store` and re-synced; a disk problem
    /// (`Io`) fails instead, so a restart retries the same file. The full check
    /// (state rebuild, root comparison) is `load`, in `Chain::open`.
    pub fn verify_head(&self) -> Result<(), StoreError> {
        let tx = self.read_tx()?;
        let meta = tx.open_table(META).map_err(dberr)?;
        let Some(h) = meta.get("height").map_err(dberr)? else { return Ok(()) };
        let h = u64::from_be_bytes(h.value().try_into().map_err(|_| StoreError::Corrupt("height"))?);
        let row = tx
            .open_table(BLOCKS)
            .map_err(dberr)?
            .get(h)
            .map_err(dberr)?
            .ok_or(StoreError::Corrupt("the head's block summary is gone"))?;
        summary_links(row.value()).map(|_| ()).ok_or(StoreError::Corrupt("block summary"))
    }

    /// Height and block hash of the last persisted finalized block (cheap; no state rebuild).
    pub fn head(&self) -> Result<Option<(u64, [u8; 32])>, StoreError> {
        let tx = self.read_tx()?;
        let meta = tx.open_table(META).map_err(dberr)?;
        let Some(height) = meta.get("height").map_err(dberr)? else { return Ok(None) };
        let height = u64::from_be_bytes(height.value().try_into().map_err(|_| StoreError::Corrupt("height"))?);
        let digest: [u8; 32] =
            meta.get("digest").map_err(dberr)?.ok_or(StoreError::Corrupt("digest"))?.value().try_into().map_err(|_| StoreError::Corrupt("digest"))?;
        Ok(Some((height, digest)))
    }

    /// The last checkpoint, with its state rebuilt and its root checked.
    pub fn load(&self) -> Result<Option<Checkpoint>, StoreError> {
        let tx = self.read_tx()?;
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
        let upgrade_notices = match meta.get(UPGRADE_NOTICES).map_err(dberr)? {
            Some(v) => serde_json::from_slice(v.value()).map_err(|_| StoreError::Corrupt("upgrade notices"))?,
            None => Vec::new(),
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
        Ok(Some(Checkpoint { height, digest, state, blocks, receipts, handoff, seed, history, schedule, upgrade_notices, statement, pruned_below, era_roots }))
    }
}

/// A database file on a disk that can be "full" — fault injection for the
/// self-healing tests (docs/design/24-self-healing.md) and the hidden
/// `--dev-storage-fault` flag. While `full` says so, every write, resize and
/// sync fails with ENOSPC, exactly what a full system disk does to redb;
/// everything else, file locks included, passes through to the real file.
/// No release path opens its store this way.
struct FullDisk {
    inner: redb::backends::FileBackend,
    full: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl std::fmt::Debug for FullDisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FullDisk")
    }
}

impl FullDisk {
    fn write_allowed(&self) -> Result<(), std::io::Error> {
        if (self.full)() {
            return Err(std::io::Error::from_raw_os_error(28)); // ENOSPC
        }
        Ok(())
    }
}

impl redb::StorageBackend for FullDisk {
    fn len(&self) -> Result<u64, std::io::Error> {
        self.inner.len()
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), std::io::Error> {
        self.inner.read(offset, out)
    }
    fn set_len(&self, len: u64) -> Result<(), std::io::Error> {
        self.write_allowed()?;
        self.inner.set_len(len)
    }
    fn sync_data(&self) -> Result<(), std::io::Error> {
        self.write_allowed()?;
        self.inner.sync_data()
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), std::io::Error> {
        self.write_allowed()?;
        self.inner.write(offset, data)
    }
    fn try_lock_range(&self, start: std::ops::Bound<u64>, end: std::ops::Bound<u64>) -> Result<bool, redb::BackendError> {
        self.inner.try_lock_range(start, end)
    }
    fn try_lock_shared_range(&self, start: std::ops::Bound<u64>, end: std::ops::Bound<u64>) -> Result<bool, redb::BackendError> {
        self.inner.try_lock_shared_range(start, end)
    }
    fn lock_range(&self, start: std::ops::Bound<u64>, end: std::ops::Bound<u64>) -> Result<(), redb::BackendError> {
        self.inner.lock_range(start, end)
    }
    fn lock_shared_range(&self, start: std::ops::Bound<u64>, end: std::ops::Bound<u64>) -> Result<(), redb::BackendError> {
        self.inner.lock_shared_range(start, end)
    }
    fn unlock_range(&self, start: std::ops::Bound<u64>, end: std::ops::Bound<u64>) -> Result<(), redb::BackendError> {
        self.inner.unlock_range(start, end)
    }
}

/// Open `path` through [`FullDisk`], for `Store::open_with`.
pub fn open_on_a_full_disk(
    path: &Path,
    full: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<Database, StoreError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(|e| StoreError::Io(e.to_string()))?;
    let backend = FullDisk { inner: redb::backends::FileBackend::new(file).map_err(dberr)?, full };
    redb::Database::builder().create_with_backend(backend).map_err(dberr)
}

/// A store on a disk that takes writes for the first `after` and then fails
/// everything — the hidden `--dev-storage-fault` of the self-healing tests
/// (docs/design/24-self-healing.md), which needs the real process to reach its
/// storage exit code. No release path builds one.
pub fn open_with_a_disk_that_fills(path: &Path, after: std::time::Duration) -> Result<Store, StoreError> {
    let start = std::time::Instant::now();
    let full: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || start.elapsed() >= after);
    Store::open_with(path, Arc::new(move |p| open_on_a_full_disk(p, full.clone())))
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
                    upgrade_notices: &[],
                    statement: &Default::default(),
                    staged: None,
                })
                .unwrap()
        };
        commit(&store, &s0);
        // Block 0 as an older binary wrote it: JSON.
        let tx = store.write_tx().unwrap();
        tx.open_table(BLOCKS).unwrap().insert(0, serde_json::to_vec(&s0).unwrap().as_slice()).unwrap();
        tx.commit().unwrap();
        let s1 = BlockSummary { state_root: state.root(), ..summary(1, &s0.hash, s0.state_root) };
        commit(&store, &s1);
        drop(store);
        let cp = Store::open(&path).unwrap().load().unwrap().unwrap();
        same(&cp.blocks[&0], &s0);
        same(&cp.blocks[&1], &s1);
        let store = Store::open(&path).unwrap();
        let tx = store.read_tx().unwrap();
        let t = tx.open_table(BLOCKS).unwrap();
        assert_eq!(t.get(0).unwrap().unwrap().value()[0], b'{');
        assert_eq!(t.get(1).unwrap().unwrap().value()[0], PACKED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The store of the incident of 2026-09-29: the disk fills, one write
    /// fails, and redb refuses every operation after it — until the database
    /// is closed and re-opened, which the node now does by itself.
    #[test]
    fn a_full_disk_is_healed_by_reopening_the_database() {
        use std::sync::atomic::{AtomicBool, Ordering};
        fn flag(full: Arc<AtomicBool>) -> Arc<dyn Fn() -> bool + Send + Sync> {
            Arc::new(move || full.load(std::sync::atomic::Ordering::Relaxed))
        }
        let dir = std::env::temp_dir().join(format!("aether-store-full-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        let full = Arc::new(AtomicBool::new(false));
        let store = Store::open_with(&path, {
            let full = full.clone();
            Arc::new(move |p| open_on_a_full_disk(p, flag(full.clone())))
        })
        .unwrap();
        store.put_meta("height", &1u64.to_be_bytes()).unwrap();

        full.store(true, Ordering::SeqCst);
        assert!(matches!(store.put_meta("height", &2u64.to_be_bytes()), Err(StoreError::Io(_))));
        assert!(
            matches!(store.put_meta("height", &3u64.to_be_bytes()), Err(StoreError::Io(_))),
            "redb is unusable after an I/O error until it is re-opened"
        );

        // Space frees: one re-open, no restart, and the store takes writes again.
        full.store(false, Ordering::SeqCst);
        store.reopen().unwrap();
        store.put_meta("height", &4u64.to_be_bytes()).unwrap();
        drop(store);
        assert_eq!(
            Store::open(&path).unwrap().meta("height").unwrap().as_deref(),
            Some(&4u64.to_be_bytes()[..]),
            "what was written after the re-open is on disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A disk that stays full is not looped on: the recovery tries its
    /// attempts and fails, and the node exits with the storage code.
    #[test]
    fn a_disk_that_never_heals_exhausts_the_recovery_attempts() {
        use std::sync::atomic::{AtomicBool, Ordering};
        fn flag(full: Arc<AtomicBool>) -> Arc<dyn Fn() -> bool + Send + Sync> {
            Arc::new(move || full.load(std::sync::atomic::Ordering::Relaxed))
        }
        let dir = std::env::temp_dir().join(format!("aether-store-stuck-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        let full = Arc::new(AtomicBool::new(false));
        let store = Store::open_with(&path, {
            let full = full.clone();
            Arc::new(move |p| open_on_a_full_disk(p, flag(full.clone())))
        })
        .unwrap();
        store.put_meta("height", &1u64.to_be_bytes()).unwrap();
        full.store(true, Ordering::SeqCst);
        assert!(matches!(
            Recovery { backoff: std::time::Duration::from_millis(1), attempts: 3 }.reopen(&store),
            Err(StoreError::Io(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A garbled or truncated file is unreadable data, not a disk problem: a
    /// restart moves it aside and re-syncs; it never treats a full disk this way.
    #[test]
    fn a_garbled_or_truncated_database_file_is_unreadable_not_a_disk_error() {
        let dir = std::env::temp_dir().join(format!("aether-store-garbled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, [0x5au8; 4_096]).unwrap();
        match Store::open(&path).map_err(|e| e.to_string()) {
            Err(e) if e.contains("Unreadable") => {}
            other => panic!("garbled: expected Unreadable, got {:?}", other.map(|_| ())),
        }
        // A real database cut off in its header: the same class.
        let _ = std::fs::remove_file(&path);
        Store::open(&path).unwrap().put_meta("height", &1u64.to_be_bytes()).unwrap();
        drop(Store::open(&path).unwrap());
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(17).unwrap();
        match Store::open(&path).map_err(|e| e.to_string()) {
            Err(e) if e.contains("Unreadable") => {}
            other => panic!("truncated: expected Unreadable, got {:?}", other.map(|_| ())),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Schema versions (red team #15, docs/design/24-self-healing.md) ----

    fn dir_for(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("aether-store-{name}-{}", std::process::id()))
    }

    /// A meta row as a raw redb reader sees it: the test's proof of what the
    /// database holds, without going through the code under test.
    fn raw_meta_row(path: &Path, key: &str) -> Option<Vec<u8>> {
        let db = redb::Database::create(path).unwrap();
        let tx = db.begin_read().unwrap();
        let t = tx.open_table(META).unwrap();
        t.get(key).unwrap().map(|v| v.value().to_vec())
    }

    /// The migration step marker, read raw.
    fn raw_marker(path: &Path) -> Option<MigrationMarker> {
        raw_meta_row(path, MIGRATION).and_then(|v| serde_json::from_slice(&v).ok())
    }

    /// The default opener the mechanism's tests use.
    fn plain_open() -> Arc<dyn Fn(&Path) -> Result<Database, StoreError> + Send + Sync> {
        Arc::new(|p| redb::Database::create(p).map_err(dberr))
    }

    /// `Store::open`'s error (`Store` is not `Debug`, so no `unwrap_err`).
    fn refuse(path: &Path) -> StoreError {
        match Store::open(path) {
            Err(e) => e,
            Ok(_) => panic!("expected this database to be refused"),
        }
    }

    /// A migration step that bumps a per-sub-step counter row (`m/<k>`) each
    /// time it runs, starting at `resume`: the run counts are on disk, so the
    /// tests can see exactly which sub-steps ran how often.
    fn count_runs(step: &MigrationStep<'_>, resume: u32, upto: u32, die_at: Option<u32>) -> Result<(), StoreError> {
        for k in 0..upto {
            if k < resume {
                continue; // committed by the interrupted run: skipped, not redone
            }
            if die_at == Some(k) && resume == 0 {
                // Simulated death between sub-steps: sub-steps below `k` are
                // on disk with the marker at k - 1; nothing of `k` is.
                return Err(StoreError::Db(format!("simulated kill before sub-step {k}")));
            }
            step.sub(k, |tx| {
                let mut meta = tx.open_table(META).map_err(dberr)?;
                let key = format!("m/{k}");
                let now = match meta.get(key.as_str()).map_err(dberr)? {
                    Some(v) => {
                        let b: [u8; 4] = v.value().try_into().expect("counter row");
                        u32::from_be_bytes(b)
                    }
                    None => 0,
                };
                meta.insert(key.as_str(), (now + 1).to_be_bytes().as_slice()).map_err(dberr)?;
                Ok(())
            })?;
        }
        Ok(())
    }

    fn counter(path: &Path, k: u32) -> u32 {
        raw_meta_row(path, &format!("m/{k}")).map(|v| u32::from_be_bytes(v.try_into().expect("counter row"))).unwrap_or(0)
    }

    #[test]
    fn the_current_schema_is_one_and_nothing_migrates_yet() {
        assert_eq!(CURRENT_SCHEMA, 1, "the first schema change bumps this and registers a step");
        assert!(migrations().is_empty(), "an empty list means every unrecorded database opens as version 1");
    }

    #[test]
    fn a_fresh_database_is_stamped_with_the_current_schema() {
        let dir = dir_for("schema-fresh");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        let store = Store::open(&path).unwrap();
        assert_eq!(store.meta(SCHEMA_VERSION).unwrap().as_deref(), Some(&CURRENT_SCHEMA.to_be_bytes()[..]));
        assert_eq!(store.meta(MIN_READ_VERSION).unwrap().as_deref(), Some(&CURRENT_SCHEMA.to_be_bytes()[..]));
        drop(store);
        assert!(Store::open(&path).is_ok(), "a stamped database re-opens");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A database from before this change: tables and rows, no version. It is
    /// version 1 — adopted in place (stamped, its rows kept), never rejected,
    /// never rebuilt.
    #[test]
    fn a_database_from_before_schema_versions_is_adopted_as_version_1() {
        let dir = dir_for("schema-old");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.redb");
        {
            let db = redb::Database::create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(BLOCKS).unwrap();
            {
                let mut meta = tx.open_table(META).unwrap();
                meta.insert("height", 7u64.to_be_bytes().as_slice()).unwrap();
            }
            tx.commit().unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.meta("height").unwrap().as_deref(), Some(&7u64.to_be_bytes()[..]), "the old rows are kept");
        assert_eq!(store.meta(SCHEMA_VERSION).unwrap().as_deref(), Some(&CURRENT_SCHEMA.to_be_bytes()[..]), "stamped in place");
        assert_eq!(store.meta(MIN_READ_VERSION).unwrap().as_deref(), Some(&CURRENT_SCHEMA.to_be_bytes()[..]));
        drop(store);
        assert!(Store::open(&path).is_ok(), "and it opens again as a version-1 database");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Data a newer binary wrote (the app rolled back to `aether.prev` after
    /// crashes): refused with `TooNew` before a single write — not a disk
    /// problem, never corruption — and the rows are exactly as that binary
    /// left them. A version row that does not decode is the same refusal.
    #[test]
    fn a_database_written_by_a_newer_schema_is_refused_untouched() {
        let dir = dir_for("schema-new");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        {
            let store = Store::open(&path).unwrap();
            store.put_meta(SCHEMA_VERSION, &2u32.to_be_bytes()).unwrap();
            store.put_meta(MIN_READ_VERSION, &2u32.to_be_bytes()).unwrap();
            drop(store);
        }
        match Store::open(&path) {
            Err(StoreError::TooNew { found: Some(2), min_read: Some(2), ours: CURRENT_SCHEMA }) => {}
            other => panic!("expected TooNew(2, 2), got {:?}", other.map(|_| ())),
        }
        let refused = refuse(&path);
        assert!(!refused.is_disk(), "not a disk problem: no re-open loop");
        assert!(crate::follow::is_too_new_error(&refused.to_string()), "the update-required path recognizes it");
        assert!(!crate::follow::is_corruption(&refused), "never moved aside, never deleted");
        assert_eq!(raw_meta_row(&path, SCHEMA_VERSION), Some(2u32.to_be_bytes().to_vec()), "untouched");
        assert_eq!(raw_meta_row(&path, MIN_READ_VERSION), Some(2u32.to_be_bytes().to_vec()), "untouched");

        // A version row that is not a u32: a layout this binary does not
        // understand — the same refusal, not corruption.
        {
            let db = redb::Database::create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            tx.open_table(META).unwrap().insert(SCHEMA_VERSION, &b"not-a-u32"[..]).unwrap();
            tx.commit().unwrap();
        }
        match Store::open(&path) {
            Err(StoreError::TooNew { found: None, min_read: None, ours: CURRENT_SCHEMA }) => {}
            other => panic!("an undecodable version row is TooNew(None), got {:?}", other.map(|_| ())),
        }
        assert!(!crate::follow::is_corruption(&refuse(&path)));

        // A readable minimum declared by a newer schema: version 2, min 1.
        {
            let db = redb::Database::create(&path).unwrap();
            let tx = db.begin_write().unwrap();
            {
                let mut meta = tx.open_table(META).unwrap();
                meta.insert(SCHEMA_VERSION, 2u32.to_be_bytes().as_slice()).unwrap();
                meta.insert(MIN_READ_VERSION, 1u32.to_be_bytes().as_slice()).unwrap();
            }
            tx.commit().unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.meta(SCHEMA_VERSION).unwrap().as_deref(), Some(&2u32.to_be_bytes()[..]), "a newer schema that keeps version 1 readable opens, rows unchanged");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The mechanism, with a test-only migration: each sub-step runs exactly
    /// once, and the finished step stamps the schema and its done marker in
    /// one transaction with the last sub-step.
    #[test]
    fn a_registered_migration_runs_its_sub_steps_and_stamps_the_schema() {
        let dir = dir_for("schema-migrate");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        {
            let store = Store::open(&path).unwrap();
            store.put_meta(SCHEMA_VERSION, &0u32.to_be_bytes()).unwrap();
            drop(store);
        }
        fn run(step: &MigrationStep<'_>, resume: u32) -> Result<(), StoreError> {
            count_runs(step, resume, 3, None)
        }
        let list = [Migration { to: 1, name: "test-one", run }];
        Store::open_migrating(&path, plain_open(), &list).unwrap();
        for k in 0..3u32 {
            assert_eq!(counter(&path, k), 1, "sub-step {k} ran exactly once");
        }
        assert_eq!(raw_meta_row(&path, SCHEMA_VERSION), Some(1u32.to_be_bytes().to_vec()));
        assert_eq!(raw_meta_row(&path, MIN_READ_VERSION), Some(1u32.to_be_bytes().to_vec()));
        assert_eq!(raw_marker(&path), Some(MigrationMarker { name: "test-one".into(), step: 2, done: true }));
        // Re-opening a migrated database runs nothing again.
        Store::open_migrating(&path, plain_open(), &list).unwrap();
        for k in 0..3u32 {
            assert_eq!(counter(&path, k), 1, "a stamped database does not re-run the step");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "open, kill the step closure, reopen, finish": a run that dies between
    /// sub-steps leaves the marker at the last committed sub-step, and the
    /// next open resumes exactly there — re-running that sub-step (idempotent)
    /// and finishing, never redoing the committed ones.
    #[test]
    fn an_interrupted_migration_resumes_at_the_recorded_step() {
        let dir = dir_for("schema-resume");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        {
            let store = Store::open(&path).unwrap();
            store.put_meta(SCHEMA_VERSION, &0u32.to_be_bytes()).unwrap();
            drop(store);
        }
        fn run(step: &MigrationStep<'_>, resume: u32) -> Result<(), StoreError> {
            count_runs(step, resume, 4, Some(2))
        }
        let list = [Migration { to: 1, name: "test-resume", run }];
        let died = match Store::open_migrating(&path, plain_open(), &list) {
            Err(e) => e,
            Ok(_) => panic!("the interrupted run was supposed to fail the open"),
        };
        assert!(died.to_string().contains("simulated kill"), "{died}");
        // What the interrupted run left: sub-steps 0 and 1 committed, the
        // marker at 1, in flight.
        assert_eq!(raw_marker(&path), Some(MigrationMarker { name: "test-resume".into(), step: 1, done: false }));
        assert_eq!((counter(&path, 0), counter(&path, 1)), (1, 1));
        assert_eq!((counter(&path, 2), counter(&path, 3)), (0, 0), "nothing of the dying sub-step is on disk");

        // Reopen: it resumes at 1 — redoing 1 (idempotent), then 2 and 3.
        Store::open_migrating(&path, plain_open(), &list).unwrap();
        assert_eq!((counter(&path, 0), counter(&path, 1), counter(&path, 2), counter(&path, 3)), (1, 2, 1, 1));
        assert_eq!(raw_meta_row(&path, SCHEMA_VERSION), Some(1u32.to_be_bytes().to_vec()));
        assert_eq!(raw_marker(&path), Some(MigrationMarker { name: "test-resume".into(), step: 3, done: true }));

        // And a third open changes nothing.
        Store::open_migrating(&path, plain_open(), &list).unwrap();
        assert_eq!((counter(&path, 0), counter(&path, 1), counter(&path, 2), counter(&path, 3)), (1, 2, 1, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A sub-step that dies mid-commit (the disk fills between the marker's
    /// write and the commit) leaves either all or nothing: the marker still
    /// names the last committed sub-step and the dead sub-step's row is not
    /// there. The next open, once the disk takes writes, resumes from there.
    #[test]
    fn a_sub_step_that_dies_mid_commit_leaves_either_all_or_nothing() {
        let dir = dir_for("schema-enospc");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        let full_marker = dir.join("FULL");
        {
            let store = Store::open(&path).unwrap();
            store.put_meta(SCHEMA_VERSION, &0u32.to_be_bytes()).unwrap();
            drop(store);
        }
        // The disk is "full" exactly while <dir>/FULL exists: the sub-step's
        // body creates it outside redb (a plain file write, so it survives the
        // aborted transaction), and the test removes it to heal the disk. A
        // plain `fn` cannot capture the path, so it recomputes the same
        // test-scoped directory.
        fn run(step: &MigrationStep<'_>, resume: u32) -> Result<(), StoreError> {
            let fill = dir_for("schema-enospc").join("FULL");
            for k in 0..3u32 {
                if k < resume {
                    continue;
                }
                step.sub(k, |tx| {
                    if resume == 0 && k == 2 {
                        std::fs::write(&fill, b"").map_err(|e| StoreError::Io(e.to_string()))?;
                    }
                    let mut meta = tx.open_table(META).map_err(dberr)?;
                    let key = format!("m/{k}");
                    meta.insert(key.as_str(), 1u32.to_be_bytes().as_slice()).map_err(dberr)?;
                    Ok(())
                })?;
            }
            Ok(())
        }
        let list = [Migration { to: 1, name: "test-enospc", run }];
        let open = {
            let full_marker = full_marker.clone();
            Arc::new(move |p: &Path| open_on_a_full_disk(p, {
                let full_marker = full_marker.clone();
                Arc::new(move || full_marker.exists())
            })) as Arc<dyn Fn(&Path) -> Result<Database, StoreError> + Send + Sync>
        };
        match Store::open_migrating(&path, open.clone(), &list) {
            Err(StoreError::Io(_)) => {}
            other => panic!("the full disk fails the sub-step's commit, got {:?}", other.map(|_| ())),
        }
        // Either all or nothing: the marker still names sub-step 1 (the last
        // committed), and sub-step 2's row is not on disk.
        assert_eq!(raw_marker(&path), Some(MigrationMarker { name: "test-enospc".into(), step: 1, done: false }));
        assert_eq!(counter(&path, 2), 0, "the dead sub-step's row never landed");
        assert_eq!(raw_meta_row(&path, SCHEMA_VERSION), Some(0u32.to_be_bytes().to_vec()), "the schema did not advance");

        // The disk takes writes again: the next open finishes the step.
        std::fs::remove_file(&full_marker).unwrap();
        Store::open_migrating(&path, open.clone(), &list).unwrap();
        assert_eq!((counter(&path, 0), counter(&path, 1), counter(&path, 2)), (1, 1, 1));
        assert_eq!(raw_meta_row(&path, SCHEMA_VERSION), Some(1u32.to_be_bytes().to_vec()));
        assert_eq!(raw_marker(&path), Some(MigrationMarker { name: "test-enospc".into(), step: 2, done: true }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A recorded version below this binary's, with no registered steps for
    /// it, is a refusal — never a silent misread of rows as the wrong layout.
    #[test]
    fn a_database_below_the_current_schema_without_registered_steps_refuses() {
        let dir = dir_for("schema-nosteps");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.redb");
        {
            let store = Store::open(&path).unwrap();
            store.put_meta(SCHEMA_VERSION, &0u32.to_be_bytes()).unwrap();
            drop(store);
        }
        let e = refuse(&path).to_string();
        assert!(e.contains("does not reach"), "a version with no registered steps cannot open: {e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
