//! World state on the EIP-7864 tree, exposed to revm as a `DatabaseRef`.

use aether_state::layout::{basic_data_key, chunkify_code, code_chunk_key, code_hash_key, storage_slot_key, BasicData};
use aether_state::{MemRepo, PartialTree, StateRepository, TreeKey, Value};
use aether_types::{Address, Bytes, B256, U256};
use revm::bytecode::Bytecode;
use revm::database_interface::{DBErrorMarker, DatabaseRef};
use revm::primitives::KECCAK_EMPTY;
use revm::state::{Account, AccountInfo};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// The chain's state hash (docs/design/00 D6): BLAKE3.
pub use aether_hash::ChainHasher;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    MissingCode(B256),
    BalanceOverflow(Address),
}

impl core::fmt::Display for StateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for StateError {}
impl DBErrorMarker for StateError {}

/// The full tree (nodes), or only a stateless witness's part of it (provers).
#[derive(Clone)]
enum Tree {
    Full(MemRepo<ChainHasher>),
    Partial(PartialTree<ChainHasher>),
}

impl Tree {
    fn get(&self, key: &TreeKey) -> Option<Value> {
        match self {
            Tree::Full(r) => r.get(key),
            Tree::Partial(p) => p.get(key),
        }
    }
    fn apply(&mut self, writes: &[(TreeKey, Option<Value>)]) {
        match self {
            Tree::Full(r) => r.apply(writes),
            Tree::Partial(p) => p.apply(writes),
        }
    }
    fn root(&self) -> aether_hash::Digest {
        match self {
            Tree::Full(r) => r.root(),
            Tree::Partial(p) => p.root(),
        }
    }
    fn hasher(&self) -> &ChainHasher {
        match self {
            Tree::Full(r) => r.hasher(),
            Tree::Partial(p) => p.hasher(),
        }
    }
}

/// What execution touched: tree keys (reads and writes) and code by hash.
#[derive(Default)]
struct Access {
    keys: BTreeSet<TreeKey>,
    codes: BTreeSet<B256>,
}

/// Everything a prover needs of the pre-state to re-execute a block: the
/// touched part of the tree and the code it runs (checked against the tree's
/// code hashes when used).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StateWitness {
    pub tree: aether_state::Witness,
    pub codes: Vec<Bytes>,
}

#[derive(Clone)]
pub struct WorldState {
    tree: Tree,
    /// Set while recording what execution touches (to build a stateless witness).
    access: Option<Arc<Mutex<Access>>>,
    /// Bytecode by keccak code hash. The tree holds code chunks for proofs;
    /// this index serves execution.
    codes: Arc<BTreeMap<B256, Bytes>>,
    /// Tree writes and new code since the last `clear_journal`: one block's diff,
    /// which the node persists on finalization.
    journal: Journal,
}

/// A state diff: tree writes in order (None deletes) and newly deployed code.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Journal {
    pub writes: Vec<(aether_state::TreeKey, Option<aether_state::Value>)>,
    pub codes: Vec<(B256, Bytes)>,
}

impl Default for WorldState {
    fn default() -> Self {
        WorldState { tree: Tree::Full(MemRepo::new(ChainHasher::new())), access: None, codes: Arc::new(BTreeMap::new()), journal: Journal::default() }
    }
}

impl WorldState {
    /// Rebuild from persisted tree entries and code (the full state).
    pub fn from_parts(entries: Vec<(aether_state::TreeKey, aether_state::Value)>, codes: BTreeMap<B256, Bytes>) -> Self {
        let mut s = WorldState { codes: Arc::new(codes), ..Default::default() };
        let writes: Vec<_> = entries.into_iter().map(|(k, v)| (k, Some(v))).collect();
        s.tree.apply(&writes);
        s
    }

    /// A prover's pre-state: only the witness's part of the tree. Its root equals
    /// the full state's root if the witness is genuine; reading or writing
    /// outside it panics. Code is indexed by its own keccak hash.
    pub fn from_witness(w: &StateWitness) -> Result<Self, aether_state::WitnessError> {
        let tree = Tree::Partial(PartialTree::new(ChainHasher::new(), &w.tree)?);
        let codes = w.codes.iter().map(|c| (revm::primitives::keccak256(c), c.clone())).collect();
        Ok(WorldState { tree, access: None, codes: Arc::new(codes), journal: Journal::default() })
    }

    /// Start recording every tree key and code hash that execution on this
    /// state (and its copies) touches.
    pub fn record_access(&mut self) {
        self.access = Some(Arc::new(Mutex::new(Access::default())));
    }

    /// The stateless witness of this (full) state for what `touched` recorded.
    pub fn witness_for(&self, touched: &WorldState) -> StateWitness {
        let Tree::Full(repo) = &self.tree else { panic!("a stateless state cannot build witnesses") };
        let (keys, hashes) = touched
            .access
            .as_ref()
            .map(|a| {
                let a = a.lock().expect("access log");
                (a.keys.iter().copied().collect::<Vec<_>>(), a.codes.clone())
            })
            .unwrap_or_default();
        let codes = hashes.iter().filter_map(|h| self.codes.get(h).cloned()).collect();
        StateWitness { tree: repo.witness(&keys), codes }
    }

    fn get(&self, key: &TreeKey) -> Option<Value> {
        if let Some(a) = &self.access {
            a.lock().expect("access log").keys.insert(*key);
        }
        self.tree.get(key)
    }

    fn code_by_hash(&self, hash: &B256) -> Option<&Bytes> {
        if let Some(a) = &self.access {
            a.lock().expect("access log").codes.insert(*hash);
        }
        self.codes.get(hash)
    }

    /// The diff recorded since the last `clear_journal`.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn clear_journal(&mut self) {
        self.journal = Journal::default();
    }

    /// Put `earlier` (e.g. a protocol activation's writes, made before the
    /// block's transactions) in front of this state's journal.
    pub fn prepend_journal(&mut self, earlier: &Journal) {
        let mut j = earlier.clone();
        j.writes.append(&mut self.journal.writes);
        j.codes.append(&mut self.journal.codes);
        self.journal = j;
    }

    /// All bytecode by code hash.
    pub fn codes(&self) -> &BTreeMap<B256, Bytes> {
        &self.codes
    }

    fn write(&mut self, writes: Vec<(aether_state::TreeKey, Option<aether_state::Value>)>) {
        if let Some(a) = &self.access {
            a.lock().expect("access log").keys.extend(writes.iter().map(|(k, _)| *k));
        }
        self.tree.apply(&writes);
        self.journal.writes.extend(writes);
    }

    pub fn root(&self) -> B256 {
        B256::from(self.tree.root())
    }

    /// The full tree. Panics on a prover's stateless state.
    pub fn repo(&self) -> &MemRepo<ChainHasher> {
        match &self.tree {
            Tree::Full(r) => r,
            Tree::Partial(_) => panic!("a stateless state has no full repository"),
        }
    }

    fn h(&self) -> &ChainHasher {
        self.tree.hasher()
    }

    pub fn account(&self, a: &Address) -> Option<BasicData> {
        self.get(&basic_data_key(self.h(), a)).map(|v| BasicData::decode(&v))
    }

    pub fn balance(&self, a: &Address) -> U256 {
        self.account(a).map(|d| U256::from(d.balance)).unwrap_or_default()
    }

    pub fn nonce(&self, a: &Address) -> u64 {
        self.account(a).map(|d| d.nonce).unwrap_or_default()
    }

    pub fn storage(&self, a: &Address, slot: U256) -> U256 {
        self.get(&storage_slot_key(self.h(), a, slot)).map(|v| U256::from_be_bytes(v)).unwrap_or_default()
    }

    pub fn code_hash(&self, a: &Address) -> B256 {
        self.get(&code_hash_key(self.h(), a)).map(B256::from).unwrap_or(KECCAK_EMPTY)
    }

    pub fn code(&self, a: &Address) -> Bytes {
        self.code_by_hash(&self.code_hash(a)).cloned().unwrap_or_default()
    }

    /// Genesis predeploy: put `code` at `a` (no constructor runs).
    pub fn set_code(&mut self, a: Address, code: Bytes) -> Result<(), StateError> {
        #[allow(clippy::clone_on_copy)] // Copy for BLAKE3, not for Poseidon2 (measurement feature)
        let h = self.h().clone();
        let hash = revm::primitives::keccak256(&code);
        let d = BasicData { code_size: code.len() as u32, ..self.account(&a).unwrap_or_default() };
        let mut writes = vec![(basic_data_key(&h, &a), Some(d.encode().map_err(|_| StateError::BalanceOverflow(a))?)), (code_hash_key(&h, &a), Some(hash.0))];
        for (i, chunk) in chunkify_code(&code).into_iter().enumerate() {
            writes.push((code_chunk_key(&h, &a, i as u64), Some(chunk)));
        }
        self.write(writes);
        Arc::make_mut(&mut self.codes).insert(hash, code.clone());
        self.journal.codes.push((hash, code));
        Ok(())
    }

    /// Genesis predeploy: a contract storage slot (zero removes it).
    pub fn set_storage(&mut self, a: Address, slot: U256, value: U256) {
        let key = storage_slot_key(self.h(), &a, slot);
        self.write(vec![(key, (!value.is_zero()).then(|| value.to_be_bytes::<32>()))]);
    }

    /// Genesis / faucet allocation.
    pub fn set_balance(&mut self, a: Address, balance: U256) -> Result<(), StateError> {
        let d = self.account(&a).unwrap_or_default().with_balance(balance).map_err(|_| StateError::BalanceOverflow(a))?;
        let key = basic_data_key(self.h(), &a);
        self.write(vec![(key, Some(d.encode().expect("code size unchanged")))]);
        Ok(())
    }

    /// Apply revm's post-transaction account changes to the tree.
    pub(crate) fn commit(&mut self, changes: &revm::state::EvmState) -> Result<(), StateError> {
        #[allow(clippy::clone_on_copy)] // Copy for BLAKE3, not for Poseidon2 (measurement feature)
        let h = self.h().clone();
        let mut writes = Vec::new();
        let mut new_codes: Vec<(B256, Bytes)> = Vec::new();
        for (addr, acc) in changes.iter() {
            if !acc.is_touched() {
                continue;
            }
            // EIP-161 / EIP-6780: destroyed or touched-empty accounts are removed.
            if acc.is_selfdestructed() || (acc.is_empty() && !acc.is_created()) {
                writes.push((basic_data_key(&h, addr), None));
                writes.push((code_hash_key(&h, addr), None));
                continue;
            }
            self.account_writes(&h, addr, acc, &mut writes, &mut new_codes)?;
        }
        self.write(writes);
        if !new_codes.is_empty() {
            let codes = Arc::make_mut(&mut self.codes);
            for (hash, code) in &new_codes {
                codes.insert(*hash, code.clone());
            }
            self.journal.codes.extend(new_codes);
        }
        Ok(())
    }

    fn account_writes(
        &self,
        h: &ChainHasher,
        addr: &Address,
        acc: &Account,
        writes: &mut Vec<(aether_state::TreeKey, Option<aether_state::Value>)>,
        new_codes: &mut Vec<(B256, Bytes)>,
    ) -> Result<(), StateError> {
        let code = acc.info.code.as_ref().map(|c| c.original_bytes()).unwrap_or_default();
        let code_size = if acc.info.code_hash == KECCAK_EMPTY {
            0
        } else if code.is_empty() {
            self.code(addr).len()
        } else {
            code.len()
        };
        let data = BasicData { version: 0, code_size: code_size as u32, nonce: acc.info.nonce, balance: 0 }
            .with_balance(acc.info.balance)
            .map_err(|_| StateError::BalanceOverflow(*addr))?;
        writes.push((basic_data_key(h, addr), Some(data.encode().map_err(|_| StateError::BalanceOverflow(*addr))?)));
        // New code: contract creation, or an EIP-7702 delegation being set.
        let old_hash = self.code_hash(addr);
        if acc.info.code_hash != old_hash {
            // Remove the previous code's chunks (a delegation replaced or cleared).
            let old_len = self.code(addr).len();
            for i in 0..old_len.div_ceil(31) {
                writes.push((code_chunk_key(h, addr, i as u64), None));
            }
            if acc.info.code_hash == KECCAK_EMPTY || code.is_empty() {
                writes.push((code_hash_key(h, addr), None));
            } else {
                writes.push((code_hash_key(h, addr), Some(acc.info.code_hash.0)));
                for (i, chunk) in chunkify_code(&code).into_iter().enumerate() {
                    writes.push((code_chunk_key(h, addr, i as u64), Some(chunk)));
                }
                new_codes.push((acc.info.code_hash, code));
            }
        }
        for (slot, value) in acc.storage.iter() {
            if value.is_changed() {
                let v = value.present_value;
                writes.push((storage_slot_key(h, addr, *slot), (!v.is_zero()).then(|| v.to_be_bytes::<32>())));
            }
        }
        Ok(())
    }
}

impl DatabaseRef for WorldState {
    type Error = StateError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let Some(d) = self.account(&address) else { return Ok(None) };
        let code_hash = self.code_hash(&address);
        let code = self.code_by_hash(&code_hash).map(|b| Bytecode::new_raw(b.clone()));
        Ok(Some(AccountInfo { balance: U256::from(d.balance), nonce: d.nonce, code_hash, code, ..Default::default() }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == KECCAK_EMPTY {
            return Ok(Bytecode::default());
        }
        self.code_by_hash(&code_hash).map(|b| Bytecode::new_raw(b.clone())).ok_or(StateError::MissingCode(code_hash))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self.storage(&address, index))
    }

    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        Ok(B256::ZERO)
    }
}
