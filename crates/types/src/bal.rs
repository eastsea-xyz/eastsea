//! Block-level Access List (EIP-7928 semantics). A validity condition: the
//! set of state a block touches, exactly. Drives static DAG scheduling and is
//! the prover's witness budget.

use crate::{put_u64, Canonical, TxIndex};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};

pub type StorageKey = B256;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAccess {
    pub address: Address,
    /// Sorted, unique.
    pub reads: Vec<StorageKey>,
    /// Sorted by key; value = index of the last tx writing that key.
    pub writes: Vec<(StorageKey, TxIndex)>,
    pub balance_touched: bool,
    pub nonce_touched: bool,
    pub code_touched: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockAccessList {
    /// Sorted by address, unique.
    pub accounts: Vec<AccountAccess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BalError {
    AccountsNotSorted,
    ReadsNotSorted(Address),
    WritesNotSorted(Address),
}

impl BlockAccessList {
    /// Structural well-formedness (sortedness, uniqueness). Checked before use.
    pub fn validate_shape(&self) -> Result<(), BalError> {
        if !self.accounts.windows(2).all(|w| w[0].address < w[1].address) {
            return Err(BalError::AccountsNotSorted);
        }
        for a in &self.accounts {
            if !a.reads.windows(2).all(|w| w[0] < w[1]) {
                return Err(BalError::ReadsNotSorted(a.address));
            }
            if !a.writes.windows(2).all(|w| w[0].0 < w[1].0) {
                return Err(BalError::WritesNotSorted(a.address));
            }
        }
        Ok(())
    }

    /// Number of distinct (address, slot) keys: the witness size driver.
    pub fn key_count(&self) -> usize {
        self.accounts
            .iter()
            .map(|a| {
                let mut s: BTreeSet<&StorageKey> = a.reads.iter().collect();
                s.extend(a.writes.iter().map(|(k, _)| k));
                1 + s.len()
            })
            .sum()
    }
}

/// Collects accesses during execution and produces a canonical BAL.
#[derive(Default)]
pub struct BalBuilder {
    accounts: BTreeMap<Address, Acc>,
}

#[derive(Default)]
struct Acc {
    reads: BTreeSet<StorageKey>,
    writes: BTreeMap<StorageKey, TxIndex>,
    balance: bool,
    nonce: bool,
    code: bool,
}

impl BalBuilder {
    pub fn touch_account(&mut self, a: Address) {
        self.accounts.entry(a).or_default();
    }
    pub fn read(&mut self, a: Address, k: StorageKey) {
        self.accounts.entry(a).or_default().reads.insert(k);
    }
    pub fn write(&mut self, a: Address, k: StorageKey, tx: TxIndex) {
        self.accounts.entry(a).or_default().writes.insert(k, tx);
    }
    pub fn balance(&mut self, a: Address) {
        self.accounts.entry(a).or_default().balance = true;
    }
    pub fn nonce(&mut self, a: Address) {
        self.accounts.entry(a).or_default().nonce = true;
    }
    pub fn code(&mut self, a: Address) {
        self.accounts.entry(a).or_default().code = true;
    }

    pub fn build(self) -> BlockAccessList {
        BlockAccessList {
            accounts: self
                .accounts
                .into_iter()
                .map(|(address, a)| AccountAccess {
                    address,
                    reads: a.reads.into_iter().collect(),
                    writes: a.writes.into_iter().collect(),
                    balance_touched: a.balance,
                    nonce_touched: a.nonce,
                    code_touched: a.code,
                })
                .collect(),
        }
    }
}

impl Canonical for BlockAccessList {
    fn encode_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"aether/bal/v1");
        put_u64(out, self.accounts.len() as u64);
        for a in &self.accounts {
            out.extend_from_slice(a.address.as_slice());
            out.push(a.balance_touched as u8 | (a.nonce_touched as u8) << 1 | (a.code_touched as u8) << 2);
            put_u64(out, a.reads.len() as u64);
            for r in &a.reads {
                out.extend_from_slice(r.as_slice());
            }
            put_u64(out, a.writes.len() as u64);
            for (k, tx) in &a.writes {
                out.extend_from_slice(k.as_slice());
                out.extend_from_slice(&tx.to_be_bytes());
            }
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_output_is_canonical_regardless_of_insertion_order() {
        let (a, b) = (Address::repeat_byte(2), Address::repeat_byte(1));
        let mut x = BalBuilder::default();
        x.write(a, B256::repeat_byte(5), 1);
        x.read(b, B256::repeat_byte(7));
        x.read(b, B256::repeat_byte(3));
        let mut y = BalBuilder::default();
        y.read(b, B256::repeat_byte(3));
        y.read(b, B256::repeat_byte(7));
        y.write(a, B256::repeat_byte(5), 1);
        let (x, y) = (x.build(), y.build());
        assert_eq!(x, y);
        assert_eq!(x.to_canonical_bytes(), y.to_canonical_bytes());
        assert!(x.validate_shape().is_ok());
        assert_eq!(x.key_count(), 2 + 3);
    }

    #[test]
    fn later_write_wins_tx_index() {
        let a = Address::repeat_byte(1);
        let mut b = BalBuilder::default();
        b.write(a, B256::ZERO, 3);
        b.write(a, B256::ZERO, 8);
        assert_eq!(b.build().accounts[0].writes, alloc::vec![(B256::ZERO, 8)]);
    }

    #[test]
    fn rejects_unsorted_shape() {
        let bal = BlockAccessList {
            accounts: alloc::vec![
                AccountAccess { address: Address::repeat_byte(2), ..Default::default() },
                AccountAccess { address: Address::repeat_byte(1), ..Default::default() },
            ],
        };
        assert_eq!(bal.validate_shape(), Err(BalError::AccountsNotSorted));
    }
}
