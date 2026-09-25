//! Differential test against `ubt` (go-ethereum bintrie V3 / EIP-7864 reference).
//! Our tree is hash-generic; with plain SHA-256 / BLAKE3 hashers it must produce
//! the same keys and roots as the reference.

use aether_hash::{Digest, HashId, Hasher};
use aether_state::layout::{basic_data_key, code_chunk_key, code_hash_key, storage_slot_key};
use aether_state::BinaryTree;
use alloy_primitives::{Address, B256, U256};
use proptest::prelude::*;
use sha2::{Digest as _, Sha256};
use ubt::UnifiedBinaryTree;

#[derive(Clone)]
struct PlainSha256;
impl Hasher for PlainSha256 {
    const ID: HashId = HashId::Blake3; // unused in these tests
    fn hash_bytes(&self, d: &[u8]) -> Digest {
        Sha256::digest(d).into()
    }
    fn compress(&self, l: &Digest, r: &Digest) -> Digest {
        let mut h = Sha256::new();
        h.update(l);
        h.update(r);
        h.finalize().into()
    }
}

#[derive(Clone)]
struct PlainBlake3;
impl Hasher for PlainBlake3 {
    const ID: HashId = HashId::Blake3;
    fn hash_bytes(&self, d: &[u8]) -> Digest {
        *blake3::hash(d).as_bytes()
    }
    fn compress(&self, l: &Digest, r: &Digest) -> Digest {
        let mut b = [0u8; 64];
        b[..32].copy_from_slice(l);
        b[32..].copy_from_slice(r);
        *blake3::hash(&b).as_bytes()
    }
}

fn ubt_key(k: ubt::TreeKey) -> [u8; 32] {
    let b: B256 = k.to_bytes();
    b.0
}

#[test]
fn key_derivation_matches_reference() {
    let h = PlainSha256;
    for a in [Address::ZERO, Address::repeat_byte(0x11), Address::repeat_byte(0xfe)] {
        assert_eq!(basic_data_key(&h, &a), ubt_key(ubt::get_basic_data_key(&a)));
        assert_eq!(code_hash_key(&h, &a), ubt_key(ubt::get_code_hash_key(&a)));
        for slot in [U256::ZERO, U256::from(63), U256::from(64), U256::from(300), U256::MAX, U256::from(0xff) << 248] {
            assert_eq!(storage_slot_key(&h, &a, slot), ubt_key(ubt::get_storage_slot_key_u256(&a, slot)), "slot {slot}");
        }
        for chunk in [0u64, 127, 128, 1000] {
            assert_eq!(code_chunk_key(&h, &a, chunk), ubt_key(ubt::get_code_chunk_key(&a, chunk)), "chunk {chunk}");
        }
    }
}

fn check_roots(entries: &[([u8; 32], [u8; 32])]) {
    let mut ours = BinaryTree::new(PlainBlake3);
    let mut reference: UnifiedBinaryTree<ubt::Blake3Hasher> = UnifiedBinaryTree::new();
    let writes: Vec<_> = entries.iter().map(|(k, v)| (*k, Some(*v))).collect();
    ours.apply(&writes);
    for (k, v) in entries {
        reference.insert_b256(B256::from(*k), B256::from(*v));
    }
    assert_eq!(B256::from(ours.root()), reference.root_hash().unwrap());
}

#[test]
fn root_matches_reference_fixed_cases() {
    check_roots(&[]);
    check_roots(&[([1; 32], [2; 32])]);
    let mut same_stem_a = [5u8; 32];
    same_stem_a[31] = 0;
    let mut same_stem_b = same_stem_a;
    same_stem_b[31] = 255;
    check_roots(&[(same_stem_a, [1; 32]), (same_stem_b, [2; 32]), ([0x80; 32], [3; 32]), ([0x7f; 32], [4; 32])]);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn root_matches_reference_random(entries in proptest::collection::btree_map(any::<[u8; 32]>(), any::<[u8; 32]>(), 0..60)) {
        let entries: Vec<_> = entries.into_iter().filter(|(_, v)| *v != [0u8; 32]).collect();
        check_roots(&entries);
    }
}
