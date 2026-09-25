use crate::storage::FlatStateStore;
use crate::types::{AccountState, Address, ContractInfo, Hash256, Transaction, TxPayload};
use crate::vm::SmartContractEngine;
use parking_lot::RwLock;
use rayon::prelude::*;
use sha2::Digest;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateKey {
    Account(Address),
    Pool(u64),
    ContractMeta(Address),
    ContractSlot(Address, u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateValue {
    Account(AccountState),
    Pool { reserve_a: u64, reserve_b: u64 },
    ContractMeta(Option<ContractInfo>),
    ContractSlot(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxVersion {
    Storage,
    Tx(usize),
}

const NUM_SHARDS: usize = 64;

pub struct MVMemory {
    base_store: FlatStateStore,
    shards: Vec<RwLock<HashMap<StateKey, BTreeMap<usize, Option<StateValue>>>>>,
}

impl MVMemory {
    pub fn new(base_store: FlatStateStore) -> Self {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        for _ in 0..NUM_SHARDS {
            shards.push(RwLock::new(HashMap::new()));
        }
        MVMemory { base_store, shards }
    }

    fn shard_idx(&self, key: &StateKey) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(key, &mut hasher);
        (std::hash::Hasher::finish(&hasher) as usize) % NUM_SHARDS
    }

    pub fn read(&self, key: &StateKey, tx_idx: usize) -> (StateValue, TxVersion) {
        let shard = &self.shards[self.shard_idx(key)];
        let r = shard.read();
        if let Some(versions) = r.get(key) {
            for (&writer_idx, val_opt) in versions.range(..tx_idx).rev() {
                if let Some(val) = val_opt {
                    return (val.clone(), TxVersion::Tx(writer_idx));
                }
            }
        }

        let val = match key {
            StateKey::Account(addr) => StateValue::Account(self.base_store.get_account(addr)),
            StateKey::Pool(pool_id) => {
                let slot_a = Hash256::of(b"reserve_a");
                let slot_b = Hash256::of(b"reserve_b");
                let addr = Address::new(*pool_id);
                let reserve_a = self.base_store.get_slot(&addr, &slot_a);
                let reserve_b = self.base_store.get_slot(&addr, &slot_b);
                StateValue::Pool {
                    reserve_a: if reserve_a == 0 { 10_000_000 } else { reserve_a },
                    reserve_b: if reserve_b == 0 { 10_000_000 } else { reserve_b },
                }
            }
            StateKey::ContractMeta(addr) => {
                StateValue::ContractMeta(self.base_store.get_contract(addr))
            }
            StateKey::ContractSlot(addr, slot) => {
                StateValue::ContractSlot(self.base_store.get_contract_slot(addr, *slot))
            }
        };
        (val, TxVersion::Storage)
    }

    pub fn write_batch(&self, tx_idx: usize, writes: &HashMap<StateKey, StateValue>) {
        for (k, v) in writes {
            let shard = &self.shards[self.shard_idx(k)];
            let mut w = shard.write();
            w.entry(k.clone()).or_default().insert(tx_idx, Some(v.clone()));
        }
    }

    pub fn clear_writes(&self, tx_idx: usize, keys: &[StateKey]) {
        for k in keys {
            let shard = &self.shards[self.shard_idx(k)];
            let mut w = shard.write();
            if let Some(versions) = w.get_mut(k) {
                versions.remove(&tx_idx);
            }
        }
    }
}

pub struct ExecutionResult {
    pub read_set: HashMap<StateKey, TxVersion>,
    pub write_set: HashMap<StateKey, StateValue>,
}

fn execute_tx_logic(
    tx: &Transaction,
    tx_idx: usize,
    mv: &MVMemory,
) -> ExecutionResult {
    let mut read_set = HashMap::new();
    let mut write_set = HashMap::new();

    // Sender balance read
    let sender_key = StateKey::Account(tx.sender);
    let (sender_val, sender_ver) = mv.read(&sender_key, tx_idx);
    read_set.insert(sender_key.clone(), sender_ver);

    let mut sender_acc = match sender_val {
        StateValue::Account(acc) => acc,
        _ => AccountState::default(),
    };

    match &tx.payload {
        TxPayload::Transfer { to, amount } => {
            let receiver_key = StateKey::Account(*to);
            let (recv_val, recv_ver) = mv.read(&receiver_key, tx_idx);
            read_set.insert(receiver_key.clone(), recv_ver);

            let mut recv_acc = match recv_val {
                StateValue::Account(acc) => acc,
                _ => AccountState::default(),
            };

            // Simulate EVM bytecode interpretation & gas metering (~30us)
            let mut hash_acc = [0u8; 32];
            hash_acc[..8].copy_from_slice(&tx.id.to_be_bytes());
            for _ in 0..60 {
                let mut hasher = sha2::Sha256::new();
                hasher.update(&hash_acc);
                hasher.update(sender_acc.balance.to_be_bytes());
                hash_acc.copy_from_slice(&hasher.finalize());
            }

            if sender_acc.balance >= *amount {
                sender_acc.balance -= amount;
                sender_acc.nonce += 1;
                recv_acc.balance += amount;

                write_set.insert(sender_key, StateValue::Account(sender_acc));
                write_set.insert(receiver_key, StateValue::Account(recv_acc));
            }
        }
        TxPayload::Swap {
            pool_id,
            amount_in,
            min_out,
        } => {
            let pool_key = StateKey::Pool(*pool_id);
            let (pool_val, pool_ver) = mv.read(&pool_key, tx_idx);
            read_set.insert(pool_key.clone(), pool_ver);

            let (mut res_a, mut res_b) = match pool_val {
                StateValue::Pool { reserve_a, reserve_b } => (reserve_a, reserve_b),
                _ => (10_000_000, 10_000_000),
            };

            let mut hash_acc = [0u8; 32];
            hash_acc[..8].copy_from_slice(&pool_id.to_be_bytes());
            for _ in 0..60 {
                let mut hasher = sha2::Sha256::new();
                hasher.update(&hash_acc);
                hasher.update(amount_in.to_be_bytes());
                hash_acc.copy_from_slice(&hasher.finalize());
            }

            let dy = (res_b * amount_in) / (res_a + amount_in);
            if sender_acc.balance >= *amount_in && dy >= *min_out {
                sender_acc.balance -= amount_in;
                sender_acc.nonce += 1;
                res_a += amount_in;
                res_b -= dy;

                write_set.insert(sender_key, StateValue::Account(sender_acc));
                write_set.insert(pool_key, StateValue::Pool { reserve_a: res_a, reserve_b: res_b });
            }
        }
        TxPayload::DeployContract { name, template, params } => {
            let (info, initial_slots) = SmartContractEngine::deploy(
                tx.sender,
                sender_acc.nonce,
                0,
                name,
                template,
                params,
            );
            sender_acc.nonce += 1;

            write_set.insert(sender_key, StateValue::Account(sender_acc));
            write_set.insert(StateKey::ContractMeta(info.address), StateValue::ContractMeta(Some(info.clone())));
            for (slot, val) in initial_slots {
                write_set.insert(StateKey::ContractSlot(info.address, slot), StateValue::ContractSlot(val));
            }
        }
        TxPayload::CallContract { contract, method, args } => {
            let meta_key = StateKey::ContractMeta(*contract);
            let (meta_val, meta_ver) = mv.read(&meta_key, tx_idx);
            read_set.insert(meta_key, meta_ver);

            if let StateValue::ContractMeta(Some(info)) = meta_val {
                sender_acc.nonce += 1;
                write_set.insert(sender_key, StateValue::Account(sender_acc));

                use std::cell::RefCell;
                let contract_reads = RefCell::new(Vec::new());
                let read_slot_fn = |slot: u64| -> u64 {
                    let slot_key = StateKey::ContractSlot(*contract, slot);
                    let (val, ver) = mv.read(&slot_key, tx_idx);
                    contract_reads.borrow_mut().push((slot_key, ver));
                    match val {
                        StateValue::ContractSlot(v) => v,
                        _ => 0,
                    }
                };

                let vm_res = SmartContractEngine::execute_call(tx.sender, &info, method, args, read_slot_fn);
                for (slot, val) in vm_res.contract_slot_writes {
                    write_set.insert(StateKey::ContractSlot(*contract, slot), StateValue::ContractSlot(val));
                }
                for (k, ver) in contract_reads.into_inner() {
                    read_set.insert(k, ver);
                }
            }
        }
    }

    ExecutionResult { read_set, write_set }
}

pub struct BlockSTMExecutor;

impl BlockSTMExecutor {
    pub fn execute_block(
        transactions: &[Transaction],
        base_store: &FlatStateStore,
    ) -> (FlatStateStore, usize) {
        let n = transactions.len();
        if n == 0 {
            return (base_store.clone(), 0);
        }

        let mv = Arc::new(MVMemory::new(base_store.clone()));
        let results: Vec<RwLock<Option<ExecutionResult>>> = (0..n).map(|_| RwLock::new(None)).collect();
        let executed: Vec<AtomicBool> = (0..n).map(|_| AtomicBool::new(false)).collect();
        let validated: Vec<AtomicBool> = (0..n).map(|_| AtomicBool::new(false)).collect();
        let abort_counts = Arc::new(AtomicUsize::new(0));
        let completed = AtomicBool::new(false);

        while !completed.load(Ordering::SeqCst) {
            // Step 1: Parallel Execution
            (0..n).into_par_iter().for_each(|i| {
                if !executed[i].load(Ordering::Acquire) {
                    let res = execute_tx_logic(&transactions[i], i, &mv);
                    mv.write_batch(i, &res.write_set);
                    *results[i].write() = Some(res);
                    executed[i].store(true, Ordering::Release);
                    validated[i].store(false, Ordering::Release);
                }
            });

            // Step 2: Parallel Validation
            let has_conflict = AtomicBool::new(false);
            (0..n).into_par_iter().for_each(|i| {
                if executed[i].load(Ordering::Acquire) && !validated[i].load(Ordering::Acquire) {
                    let r = results[i].read();
                    if let Some(res) = r.as_ref() {
                        let mut valid = true;
                        for (key, prior_ver) in &res.read_set {
                            let (_, current_ver) = mv.read(key, i);
                            if current_ver != *prior_ver {
                                valid = false;
                                break;
                            }
                        }

                        if valid {
                            validated[i].store(true, Ordering::Release);
                        } else {
                            has_conflict.store(true, Ordering::Release);
                            abort_counts.fetch_add(1, Ordering::Relaxed);

                            let keys_to_clear: Vec<_> = res.write_set.keys().cloned().collect();
                            drop(r);

                            mv.clear_writes(i, &keys_to_clear);
                            *results[i].write() = None;
                            executed[i].store(false, Ordering::Release);

                            for j in (i + 1)..n {
                                if executed[j].load(Ordering::Acquire) {
                                    let r_j = results[j].read();
                                    if let Some(res_j) = r_j.as_ref() {
                                        for key in &keys_to_clear {
                                            if res_j.read_set.contains_key(key) {
                                                validated[j].store(false, Ordering::Release);
                                                executed[j].store(false, Ordering::Release);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            });

            let all_done = (0..n).all(|i| executed[i].load(Ordering::Acquire) && validated[i].load(Ordering::Acquire));
            if all_done && !has_conflict.load(Ordering::Acquire) {
                completed.store(true, Ordering::SeqCst);
            }
        }

        // Apply final committed writes
        let final_store = base_store.clone();
        let mut account_writes = HashMap::new();
        let mut slot_writes = HashMap::new();
        let mut new_contracts = Vec::new();
        let mut contract_slot_writes = HashMap::new();

        for i in 0..n {
            let r = results[i].read();
            if let Some(res) = r.as_ref() {
                for (k, v) in &res.write_set {
                    match (k, v) {
                        (StateKey::Account(addr), StateValue::Account(acc)) => {
                            account_writes.insert(*addr, acc.clone());
                        }
                        (StateKey::Pool(pool_id), StateValue::Pool { reserve_a, reserve_b }) => {
                            let addr = Address::new(*pool_id);
                            slot_writes.insert((addr, Hash256::of(b"reserve_a")), *reserve_a);
                            slot_writes.insert((addr, Hash256::of(b"reserve_b")), *reserve_b);
                        }
                        (StateKey::ContractMeta(_addr), StateValue::ContractMeta(Some(info))) => {
                            new_contracts.push(info.clone());
                        }
                        (StateKey::ContractSlot(addr, slot), StateValue::ContractSlot(val)) => {
                            contract_slot_writes.insert((*addr, *slot), *val);
                        }
                        _ => {}
                    }
                }
            }
        }

        final_store.apply_batch(account_writes, slot_writes, new_contracts, contract_slot_writes);
        (final_store, abort_counts.load(Ordering::SeqCst))
    }
}

pub struct SequentialExecutor;

impl SequentialExecutor {
    pub fn execute_block(
        transactions: &[Transaction],
        base_store: &FlatStateStore,
    ) -> FlatStateStore {
        let store = base_store.clone();
        for tx in transactions {
            let mut sender = store.get_account(&tx.sender);
            match &tx.payload {
                TxPayload::Transfer { to, amount } => {
                    let mut recv = store.get_account(to);
                    let mut hash_acc = [0u8; 32];
                    hash_acc[..8].copy_from_slice(&tx.id.to_be_bytes());
                    for _ in 0..60 {
                        let mut hasher = sha2::Sha256::new();
                        hasher.update(&hash_acc);
                        hasher.update(sender.balance.to_be_bytes());
                        hash_acc.copy_from_slice(&hasher.finalize());
                    }

                    if sender.balance >= *amount {
                        sender.balance -= amount;
                        sender.nonce += 1;
                        recv.balance += amount;
                        store.set_account(tx.sender, sender);
                        store.set_account(*to, recv);
                    }
                }
                TxPayload::Swap { pool_id, amount_in, min_out } => {
                    let pool_addr = Address::new(*pool_id);
                    let slot_a = Hash256::of(b"reserve_a");
                    let slot_b = Hash256::of(b"reserve_b");
                    let mut res_a = store.get_slot(&pool_addr, &slot_a);
                    let mut res_b = store.get_slot(&pool_addr, &slot_b);
                    if res_a == 0 { res_a = 10_000_000; }
                    if res_b == 0 { res_b = 10_000_000; }

                    let mut hash_acc = [0u8; 32];
                    hash_acc[..8].copy_from_slice(&pool_id.to_be_bytes());
                    for _ in 0..60 {
                        let mut hasher = sha2::Sha256::new();
                        hasher.update(&hash_acc);
                        hasher.update(amount_in.to_be_bytes());
                        hash_acc.copy_from_slice(&hasher.finalize());
                    }

                    let dy = (res_b * amount_in) / (res_a + amount_in);
                    if sender.balance >= *amount_in && dy >= *min_out {
                        sender.balance -= amount_in;
                        sender.nonce += 1;
                        res_a += amount_in;
                        res_b -= dy;
                        store.set_account(tx.sender, sender);
                        store.set_slot(pool_addr, slot_a, res_a);
                        store.set_slot(pool_addr, slot_b, res_b);
                    }
                }
                TxPayload::DeployContract { name, template, params } => {
                    let (info, initial_slots) = SmartContractEngine::deploy(
                        tx.sender,
                        sender.nonce,
                        0,
                        name,
                        template,
                        params,
                    );
                    sender.nonce += 1;
                    store.set_account(tx.sender, sender);
                    store.register_contract(info.clone());
                    for (slot, val) in initial_slots {
                        store.set_contract_slot(info.address, slot, val);
                    }
                }
                TxPayload::CallContract { contract, method, args } => {
                    if let Some(info) = store.get_contract(contract) {
                        sender.nonce += 1;
                        store.set_account(tx.sender, sender);
                        let read_slot_fn = |slot: u64| store.get_contract_slot(contract, slot);
                        let vm_res = SmartContractEngine::execute_call(tx.sender, &info, method, args, read_slot_fn);
                        for (slot, val) in vm_res.contract_slot_writes {
                            store.set_contract_slot(*contract, slot, val);
                        }
                    }
                }
            }
        }
        store
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_stm_contract_execution() {
        let base_store = FlatStateStore::new();
        let creator = Address::new(1);
        base_store.deposit(creator, 1_000_000);

        let contract_addr = SmartContractEngine::derive_contract_address(&creator, 0, "CounterTest");

        let txs = vec![
            Transaction {
                id: 1,
                sender: creator,
                nonce: 0,
                payload: TxPayload::DeployContract {
                    name: "CounterTest".to_string(),
                    template: "counter".to_string(),
                    params: vec![100],
                },
                gas_limit: 50000,
            },
            Transaction {
                id: 2,
                sender: creator,
                nonce: 1,
                payload: TxPayload::CallContract {
                    contract: contract_addr,
                    method: "increment".to_string(),
                    args: vec![25],
                },
                gas_limit: 30000,
            },
            Transaction {
                id: 3,
                sender: creator,
                nonce: 2,
                payload: TxPayload::CallContract {
                    contract: contract_addr,
                    method: "decrement".to_string(),
                    args: vec![10],
                },
                gas_limit: 30000,
            },
        ];

        let (new_store, aborts) = BlockSTMExecutor::execute_block(&txs, &base_store);
        let contract = new_store.get_contract(&contract_addr);
        assert!(contract.is_some(), "Contract should be deployed");
        let slot0 = new_store.get_contract_slot(&contract_addr, 0);
        // 100 + 25 - 10 = 115
        assert_eq!(slot0, 115, "Slot 0 should reflect incremental updates in serial dependency");
        println!("Contract execution passed! Aborts: {}", aborts);
    }
}

