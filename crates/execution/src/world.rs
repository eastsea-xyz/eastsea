//! World state on the EIP-7864 tree, exposed to revm as a `DatabaseRef`.

use aether_hash::Poseidon2KoalaBear;
use aether_state::layout::{basic_data_key, chunkify_code, code_chunk_key, code_hash_key, storage_slot_key, BasicData};
use aether_state::{MemRepo, StateRepository};
use aether_types::{Address, Bytes, B256, U256};
use revm::bytecode::Bytecode;
use revm::database_interface::{DBErrorMarker, DatabaseRef};
use revm::primitives::KECCAK_EMPTY;
use revm::state::{Account, AccountInfo};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The chain's state hash (docs/design/00 D6).
pub type ChainHasher = Poseidon2KoalaBear;

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

#[derive(Clone)]
pub struct WorldState {
    tree: MemRepo<ChainHasher>,
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
        WorldState { tree: MemRepo::new(ChainHasher::new()), codes: Arc::new(BTreeMap::new()), journal: Journal::default() }
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

    /// The diff recorded since the last `clear_journal`.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn clear_journal(&mut self) {
        self.journal = Journal::default();
    }

    /// All bytecode by code hash.
    pub fn codes(&self) -> &BTreeMap<B256, Bytes> {
        &self.codes
    }

    fn write(&mut self, writes: Vec<(aether_state::TreeKey, Option<aether_state::Value>)>) {
        self.tree.apply(&writes);
        self.journal.writes.extend(writes);
    }

    pub fn root(&self) -> B256 {
        B256::from(self.tree.root())
    }

    pub fn repo(&self) -> &MemRepo<ChainHasher> {
        &self.tree
    }

    fn h(&self) -> &ChainHasher {
        self.tree.hasher()
    }

    pub fn account(&self, a: &Address) -> Option<BasicData> {
        self.tree.get(&basic_data_key(self.h(), a)).map(|v| BasicData::decode(&v))
    }

    pub fn balance(&self, a: &Address) -> U256 {
        self.account(a).map(|d| U256::from(d.balance)).unwrap_or_default()
    }

    pub fn nonce(&self, a: &Address) -> u64 {
        self.account(a).map(|d| d.nonce).unwrap_or_default()
    }

    pub fn storage(&self, a: &Address, slot: U256) -> U256 {
        self.tree.get(&storage_slot_key(self.h(), a, slot)).map(|v| U256::from_be_bytes(v)).unwrap_or_default()
    }

    pub fn code_hash(&self, a: &Address) -> B256 {
        self.tree.get(&code_hash_key(self.h(), a)).map(B256::from).unwrap_or(KECCAK_EMPTY)
    }

    pub fn code(&self, a: &Address) -> Bytes {
        self.codes.get(&self.code_hash(a)).cloned().unwrap_or_default()
    }

    /// Genesis predeploy: put `code` at `a` (no constructor runs).
    pub fn set_code(&mut self, a: Address, code: Bytes) -> Result<(), StateError> {
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

    /// Genesis / faucet allocation.
    pub fn set_balance(&mut self, a: Address, balance: U256) -> Result<(), StateError> {
        let d = self.account(&a).unwrap_or_default().with_balance(balance).map_err(|_| StateError::BalanceOverflow(a))?;
        let key = basic_data_key(self.h(), &a);
        self.write(vec![(key, Some(d.encode().expect("code size unchanged")))]);
        Ok(())
    }

    /// Apply revm's post-transaction account changes to the tree.
    pub(crate) fn commit(&mut self, changes: &revm::state::EvmState) -> Result<(), StateError> {
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
        let code = self.codes.get(&code_hash).map(|b| Bytecode::new_raw(b.clone()));
        Ok(Some(AccountInfo { balance: U256::from(d.balance), nonce: d.nonce, code_hash, code, ..Default::default() }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == KECCAK_EMPTY {
            return Ok(Bytecode::default());
        }
        self.codes.get(&code_hash).map(|b| Bytecode::new_raw(b.clone())).ok_or(StateError::MissingCode(code_hash))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self.storage(&address, index))
    }

    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        Ok(B256::ZERO)
    }
}
