use crate::types::{AccountState, Address, ContractInfo, Hash256};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PersistentChainState {
    pub round: u64,
    pub total_rewards: u64,
    pub accounts: HashMap<String, AccountState>,
    pub contracts: HashMap<String, ContractInfo>,
    pub contract_slots: Vec<(String, u64, u64)>,
}

#[derive(Clone, Debug)]
pub struct FlatStateStore {
    accounts: Arc<RwLock<HashMap<Address, AccountState>>>,
    storage_slots: Arc<RwLock<HashMap<(Address, Hash256), u64>>>,
    contracts: Arc<RwLock<HashMap<Address, ContractInfo>>>,
    contract_slots: Arc<RwLock<HashMap<(Address, u64), u64>>>,
}

impl FlatStateStore {
    pub fn new() -> Self {
        FlatStateStore {
            accounts: Arc::new(RwLock::new(HashMap::new())),
            storage_slots: Arc::new(RwLock::new(HashMap::new())),
            contracts: Arc::new(RwLock::new(HashMap::new())),
            contract_slots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get_account(&self, addr: &Address) -> AccountState {
        self.accounts.read().get(addr).cloned().unwrap_or_default()
    }

    pub fn get_balance(&self, addr: &Address) -> u64 {
        self.get_account(addr).balance
    }

    pub fn set_account(&self, addr: Address, state: AccountState) {
        self.accounts.write().insert(addr, state);
    }

    pub fn deposit(&self, addr: Address, amount: u64) {
        let mut w = self.accounts.write();
        let entry = w.entry(addr).or_default();
        entry.balance += amount;
    }

    pub fn transfer(&self, from: Address, to: Address, amount: u64) -> bool {
        let mut w = self.accounts.write();
        let from_acc = w.entry(from).or_default();
        if from_acc.balance < amount {
            return false;
        }
        from_acc.balance -= amount;
        let to_acc = w.entry(to).or_default();
        to_acc.balance += amount;
        true
    }

    pub fn get_slot(&self, addr: &Address, slot: &Hash256) -> u64 {
        self.storage_slots.read().get(&(*addr, *slot)).cloned().unwrap_or(0)
    }

    pub fn set_slot(&self, addr: Address, slot: Hash256, val: u64) {
        self.storage_slots.write().insert((addr, slot), val);
    }

    // Smart Contracts Support
    pub fn register_contract(&self, info: ContractInfo) {
        self.contracts.write().insert(info.address, info);
    }

    pub fn get_contract(&self, addr: &Address) -> Option<ContractInfo> {
        self.contracts.read().get(addr).cloned()
    }

    pub fn list_contracts(&self) -> Vec<ContractInfo> {
        self.contracts.read().values().cloned().collect()
    }

    pub fn get_contract_slot(&self, addr: &Address, slot: u64) -> u64 {
        self.contract_slots.read().get(&(*addr, slot)).cloned().unwrap_or(0)
    }

    pub fn set_contract_slot(&self, addr: Address, slot: u64, val: u64) {
        self.contract_slots.write().insert((addr, slot), val);
    }

    pub fn get_all_contract_slots(&self, addr: &Address) -> HashMap<u64, u64> {
        let r = self.contract_slots.read();
        let mut map = HashMap::new();
        for ((c_addr, slot), val) in r.iter() {
            if c_addr == addr {
                map.insert(*slot, *val);
            }
        }
        map
    }

    pub fn export_all_contract_slots(&self) -> Vec<(Address, u64, u64)> {
        let r = self.contract_slots.read();
        r.iter().map(|((addr, slot), val)| (*addr, *slot, *val)).collect()
    }

    pub fn export_all_accounts(&self) -> Vec<(Address, AccountState)> {
        let r = self.accounts.read();
        r.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    pub fn save_to_disk(&self, round: u64, total_rewards: u64, path: &std::path::Path) {
        let acc_r = self.accounts.read();
        let c_r = self.contracts.read();
        let cs_r = self.contract_slots.read();

        let mut p_accounts = HashMap::new();
        for (addr, state) in acc_r.iter() {
            p_accounts.insert(addr.to_hex(), state.clone());
        }

        let mut p_contracts = HashMap::new();
        for (addr, c) in c_r.iter() {
            p_contracts.insert(addr.to_hex(), c.clone());
        }

        let mut p_slots = Vec::new();
        for ((addr, slot), val) in cs_r.iter() {
            p_slots.push((addr.to_hex(), *slot, *val));
        }

        let state = PersistentChainState {
            round,
            total_rewards,
            accounts: p_accounts,
            contracts: p_contracts,
            contract_slots: p_slots,
        };

        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let tmp_path = path.with_extension("tmp");
            if std::fs::write(&tmp_path, json).is_ok() {
                let _ = std::fs::rename(tmp_path, path);
            }
        }
    }

    pub fn load_from_disk(&self, path: &std::path::Path) -> Option<(u64, u64)> {
        let content = std::fs::read_to_string(path).ok()?;
        let state: PersistentChainState = serde_json::from_str(&content).ok()?;

        let mut acc_w = self.accounts.write();
        for (addr_str, acc) in state.accounts {
            if let Some(addr) = Address::from_hex(&addr_str) {
                acc_w.insert(addr, acc);
            }
        }

        let mut c_w = self.contracts.write();
        for (addr_str, c) in state.contracts {
            if let Some(addr) = Address::from_hex(&addr_str) {
                c_w.insert(addr, c);
            }
        }

        let mut cs_w = self.contract_slots.write();
        for (addr_str, slot, val) in state.contract_slots {
            if let Some(addr) = Address::from_hex(&addr_str) {
                cs_w.insert((addr, slot), val);
            }
        }

        Some((state.round, state.total_rewards))
    }

    pub fn apply_batch(
        &self,
        account_writes: HashMap<Address, AccountState>,
        slot_writes: HashMap<(Address, Hash256), u64>,
        new_contracts: Vec<ContractInfo>,
        contract_slot_writes: HashMap<(Address, u64), u64>,
    ) {
        let mut acc_w = self.accounts.write();
        for (addr, state) in account_writes {
            acc_w.insert(addr, state);
        }

        let mut slot_w = self.storage_slots.write();
        for (k, v) in slot_writes {
            slot_w.insert(k, v);
        }

        let mut c_w = self.contracts.write();
        for c in new_contracts {
            c_w.insert(c.address, c);
        }

        let mut cs_w = self.contract_slots.write();
        for (k, v) in contract_slot_writes {
            cs_w.insert(k, v);
        }
    }

    pub fn state_root(&self) -> Hash256 {
        let r = self.accounts.read();
        let mut sorted_keys: Vec<_> = r.keys().collect();
        sorted_keys.sort_by_key(|a| a.0);

        let mut bytes = Vec::new();
        for k in sorted_keys {
            bytes.extend_from_slice(&k.0);
            let state = &r[k];
            bytes.extend_from_slice(&state.balance.to_be_bytes());
            bytes.extend_from_slice(&state.nonce.to_be_bytes());
        }

        let cs = self.contract_slots.read();
        let mut sorted_cs: Vec<_> = cs.iter().collect();
        sorted_cs.sort_by_key(|((addr, slot), _)| (addr.0, *slot));
        for ((addr, slot), val) in sorted_cs {
            bytes.extend_from_slice(&addr.0);
            bytes.extend_from_slice(&slot.to_be_bytes());
            bytes.extend_from_slice(&val.to_be_bytes());
        }

        Hash256::of(&bytes)
    }
}
