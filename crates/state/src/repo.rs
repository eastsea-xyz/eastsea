//! `StateRepository` (docs/design/03-traits.md) and the in-memory reference
//! implementation. Persistent engines are tested for root equality against
//! `MemRepo` in differential tests.

use crate::tree::{BinaryTree, Proof, TreeKey, Value};
use aether_hash::{Digest, Hasher};

pub trait StateRepository {
    fn get(&self, key: &TreeKey) -> Option<Value>;
    /// Atomically apply a batch of writes (None deletes).
    fn apply(&mut self, writes: &[(TreeKey, Option<Value>)]);
    fn root(&self) -> Digest;
    /// One proof per key (inclusion or absence) against `root()`.
    fn prove(&self, keys: &[TreeKey]) -> Vec<Proof>;
}

#[derive(Clone)]
pub struct MemRepo<H: Hasher> {
    tree: BinaryTree<H>,
}

impl<H: Hasher> MemRepo<H> {
    pub fn new(hasher: H) -> Self {
        MemRepo { tree: BinaryTree::new(hasher) }
    }

    pub fn hasher(&self) -> &H {
        self.tree.hasher()
    }
}

impl<H: Hasher> StateRepository for MemRepo<H> {
    fn get(&self, key: &TreeKey) -> Option<Value> {
        self.tree.get(key)
    }
    fn apply(&mut self, writes: &[(TreeKey, Option<Value>)]) {
        self.tree.apply(writes)
    }
    fn root(&self) -> Digest {
        self.tree.root()
    }
    fn prove(&self, keys: &[TreeKey]) -> Vec<Proof> {
        keys.iter().map(|k| self.tree.prove(k)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{basic_data_key, storage_slot_key, BasicData};
    use aether_hash::Poseidon2KoalaBear;
    use alloy_primitives::{Address, U256};

    #[test]
    fn account_round_trip_with_proofs() {
        let h = Poseidon2KoalaBear::new();
        let mut repo = MemRepo::new(h.clone());
        let alice = Address::repeat_byte(0xa1);
        let acct = BasicData { nonce: 3, balance: 1_000, ..Default::default() };
        let slot = storage_slot_key(&h, &alice, U256::from(1234));
        repo.apply(&[(basic_data_key(&h, &alice), Some(acct.encode().unwrap())), (slot, Some([7u8; 32]))]);
        let root = repo.root();
        let proofs = repo.prove(&[basic_data_key(&h, &alice), slot, basic_data_key(&h, &Address::repeat_byte(0xb0))]);
        assert_eq!(BasicData::decode(&proofs[0].value.unwrap()), acct);
        assert_eq!(proofs[1].value, Some([7u8; 32]));
        assert_eq!(proofs[2].value, None, "unknown account is provably absent");
        for p in &proofs {
            assert!(p.verify(repo.hasher(), &root).is_ok());
        }
    }
}
