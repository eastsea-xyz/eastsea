//! EIP-7864 style binary tree.
//!
//! - A key is 32 bytes: 31-byte stem ‖ 1-byte sub-index.
//! - All values sharing a stem live in one stem node: a 256-leaf binary
//!   subtree over H(value) (empty leaf = ZERO), hashed as H(stem ‖ 0x00 ‖ root).
//! - Stems form a binary tree on their bits. A subtree holding exactly one stem
//!   collapses to that stem node; an empty subtree is ZERO; internal nodes are
//!   compress(left, right) with compress(ZERO, ZERO) = ZERO.
//!
//! Hashing is pluggable (`Hasher`) so the same structure runs on Poseidon2 or BLAKE3.

use aether_hash::{Digest, Hasher, ZERO};
use std::collections::BTreeMap;

pub type Stem = [u8; 31];
pub type TreeKey = [u8; 32];
pub type Value = [u8; 32];

const STEM_BITS: usize = 248;
const SUBTREE_DEPTH: usize = 8;

pub fn split_key(k: &TreeKey) -> (Stem, u8) {
    let mut stem = [0u8; 31];
    stem.copy_from_slice(&k[..31]);
    (stem, k[31])
}

fn stem_bit(stem: &Stem, depth: usize) -> bool {
    (stem[depth / 8] >> (7 - depth % 8)) & 1 == 1
}

fn compress_z<H: Hasher>(h: &H, l: &Digest, r: &Digest) -> Digest {
    if *l == ZERO && *r == ZERO {
        ZERO
    } else {
        h.compress(l, r)
    }
}

/// EIP-7864 zero rule: hashing an all-zero input yields ZERO.
fn hash_z<H: Hasher>(h: &H, data: &[u8]) -> Digest {
    if data.iter().all(|b| *b == 0) {
        ZERO
    } else {
        h.hash_bytes(data)
    }
}

fn leaf_hash<H: Hasher>(h: &H, v: &Option<Value>) -> Digest {
    match v {
        Some(v) => hash_z(h, v),
        None => ZERO,
    }
}

fn stem_node_hash<H: Hasher>(h: &H, stem: &Stem, subtree_root: &Digest) -> Digest {
    let mut buf = [0u8; 31 + 1 + 32];
    buf[..31].copy_from_slice(stem);
    buf[32..].copy_from_slice(subtree_root);
    hash_z(h, &buf)
}

/// A stem's values, sparse: only present sub-indices are stored. Its 256-leaf
/// subtree hashes empty ranges to ZERO (compress(ZERO, ZERO) = ZERO), so the
/// root is computed over the present leaves only: O(values × 8), not 511
/// hashes per write, and a copy moves only what is there.
#[derive(Clone)]
struct StemNode {
    values: BTreeMap<u8, Value>,
    /// Hash of the 256-leaf subtree; recomputed on every write to this stem.
    subtree_root: Digest,
}

impl StemNode {
    fn empty() -> Self {
        StemNode { values: BTreeMap::new(), subtree_root: ZERO }
    }

    /// Hash of the aligned leaf range [lo, lo + size) (size a power of two).
    fn range_hash<H: Hasher>(&self, h: &H, lo: usize, size: usize) -> Digest {
        let mut present = self.values.range(lo as u8..=(lo + size - 1) as u8);
        if size == 1 {
            return leaf_hash(h, &present.next().map(|(_, v)| *v));
        }
        if present.next().is_none() {
            return ZERO;
        }
        let half = size / 2;
        compress_z(h, &self.range_hash(h, lo, half), &self.range_hash(h, lo + half, half))
    }

    fn root<H: Hasher>(&self, h: &H) -> Digest {
        self.range_hash(h, 0, 256)
    }

    /// Siblings of leaf `sub` in the 256-leaf subtree, leaf level up.
    fn siblings<H: Hasher>(&self, h: &H, sub: u8) -> [Digest; SUBTREE_DEPTH] {
        core::array::from_fn(|lvl| self.range_hash(h, ((sub as usize >> lvl) ^ 1) << lvl, 1 << lvl))
    }

    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Clone)]
pub struct BinaryTree<H: Hasher> {
    hasher: H,
    stems: BTreeMap<Stem, StemNode>,
}

/// Proof that `key` holds `value` (Some) or is absent (None) under a root.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Proof {
    pub key: TreeKey,
    pub value: Option<Value>,
    /// Siblings of the stem-tree path, from the root down.
    pub stem_path: Vec<Digest>,
    pub bottom: Bottom,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Bottom {
    /// The key's stem exists; 8 sibling hashes inside its 256-leaf subtree, leaf up.
    Stem { subtree_siblings: [Digest; SUBTREE_DEPTH] },
    /// The path ends at a different stem (absence).
    OtherStem { stem: Stem, subtree_root: Digest },
    /// The path ends at an empty subtree (absence).
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofError {
    RootMismatch,
    Malformed,
    NonCanonicalDigest,
}

impl<H: Hasher> BinaryTree<H> {
    pub fn new(hasher: H) -> Self {
        BinaryTree { hasher, stems: BTreeMap::new() }
    }

    pub fn hasher(&self) -> &H {
        &self.hasher
    }

    pub fn get(&self, key: &TreeKey) -> Option<Value> {
        let (stem, sub) = split_key(key);
        self.stems.get(&stem).and_then(|n| n.values.get(&sub).copied())
    }

    /// Apply writes. `None` and the all-zero value both delete (EVM semantics:
    /// unset == 0). Stem hashes are refreshed once per touched stem.
    pub fn apply(&mut self, writes: &[(TreeKey, Option<Value>)]) {
        let mut touched = std::collections::BTreeSet::new();
        for (k, v) in writes {
            let (stem, sub) = split_key(k);
            let node = self.stems.entry(stem).or_insert_with(StemNode::empty);
            match v.filter(|v| *v != [0u8; 32]) {
                Some(v) => node.values.insert(sub, v),
                None => node.values.remove(&sub),
            };
            touched.insert(stem);
        }
        for stem in touched {
            let node = self.stems.get(&stem).expect("inserted above");
            if node.is_empty() {
                self.stems.remove(&stem);
                continue;
            }
            let root = node.root(&self.hasher);
            self.stems.get_mut(&stem).expect("present").subtree_root = root;
        }
    }

    pub fn root(&self) -> Digest {
        let stems: Vec<(&Stem, &StemNode)> = self.stems.iter().collect();
        self.subtree(&stems, 0)
    }

    fn subtree(&self, stems: &[(&Stem, &StemNode)], depth: usize) -> Digest {
        match stems {
            [] => ZERO,
            [(s, n)] => stem_node_hash(&self.hasher, s, &n.subtree_root),
            _ => {
                let split = stems.partition_point(|(s, _)| !stem_bit(s, depth));
                let l = self.subtree(&stems[..split], depth + 1);
                let r = self.subtree(&stems[split..], depth + 1);
                compress_z(&self.hasher, &l, &r)
            }
        }
    }

    pub fn prove(&self, key: &TreeKey) -> Proof {
        let (stem, sub) = split_key(key);
        let all: Vec<(&Stem, &StemNode)> = self.stems.iter().collect();
        let mut slice: &[(&Stem, &StemNode)] = &all;
        let mut stem_path = Vec::new();
        let mut depth = 0;
        while slice.len() > 1 {
            let split = slice.partition_point(|(s, _)| !stem_bit(s, depth));
            let (left, right) = slice.split_at(split);
            if stem_bit(&stem, depth) {
                stem_path.push(self.subtree(left, depth + 1));
                slice = right;
            } else {
                stem_path.push(self.subtree(right, depth + 1));
                slice = left;
            }
            depth += 1;
        }
        let bottom = match slice {
            [] => Bottom::Empty,
            [(s, n)] if **s == stem => Bottom::Stem { subtree_siblings: n.siblings(&self.hasher, sub) },
            [(s, n)] => Bottom::OtherStem { stem: **s, subtree_root: n.subtree_root },
            _ => unreachable!("loop ends with at most one stem"),
        };
        Proof { key: *key, value: self.get(key), stem_path, bottom }
    }
}

impl Proof {
    /// Check this proof against `root`. On success the proof's `value` is the
    /// key's value under that root (None = absent).
    pub fn verify<H: Hasher>(&self, h: &H, root: &Digest) -> Result<(), ProofError> {
        if self.stem_path.len() > STEM_BITS {
            return Err(ProofError::Malformed);
        }
        let digests = self.stem_path.iter().chain(match &self.bottom {
            Bottom::Stem { subtree_siblings } => subtree_siblings.iter().collect::<Vec<_>>(),
            Bottom::OtherStem { subtree_root, .. } => vec![subtree_root],
            Bottom::Empty => vec![],
        });
        if !digests.into_iter().all(|d| h.is_canonical(d)) {
            return Err(ProofError::NonCanonicalDigest);
        }

        let (stem, sub) = split_key(&self.key);
        let depth = self.stem_path.len();
        let mut acc = match &self.bottom {
            Bottom::Stem { subtree_siblings } => {
                let mut node = leaf_hash(h, &self.value);
                let mut idx = sub as usize;
                for sib in subtree_siblings {
                    node = if idx & 1 == 0 { compress_z(h, &node, sib) } else { compress_z(h, sib, &node) };
                    idx >>= 1;
                }
                // A present stem must hold at least one value; an all-empty subtree
                // would mean the stem does not exist and the proof is forged.
                if node == ZERO {
                    return Err(ProofError::Malformed);
                }
                stem_node_hash(h, &stem, &node)
            }
            Bottom::OtherStem { stem: other, subtree_root } => {
                if self.value.is_some() || *other == stem || *subtree_root == ZERO {
                    return Err(ProofError::Malformed);
                }
                // The other stem must share the path prefix walked so far.
                if (0..depth).any(|d| stem_bit(other, d) != stem_bit(&stem, d)) {
                    return Err(ProofError::Malformed);
                }
                stem_node_hash(h, other, subtree_root)
            }
            Bottom::Empty => {
                if self.value.is_some() {
                    return Err(ProofError::Malformed);
                }
                ZERO
            }
        };
        for d in (0..depth).rev() {
            let sib = &self.stem_path[d];
            acc = if stem_bit(&stem, d) { compress_z(h, sib, &acc) } else { compress_z(h, &acc, sib) };
        }
        if acc == *root {
            Ok(())
        } else {
            Err(ProofError::RootMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_hash::{Blake3, Poseidon2KoalaBear};
    use proptest::prelude::*;

    fn key(stem_byte: u8, sub: u8) -> TreeKey {
        let mut k = [stem_byte; 32];
        k[31] = sub;
        k
    }

    fn v(x: u8) -> Option<Value> {
        Some([x; 32])
    }

    #[test]
    fn empty_tree_root_is_zero() {
        assert_eq!(BinaryTree::new(Blake3).root(), ZERO);
    }

    #[test]
    fn root_independent_of_write_order_and_deletion_restores() {
        let mut a = BinaryTree::new(Blake3);
        a.apply(&[(key(1, 0), v(1)), (key(2, 5), v(2)), (key(0x80, 9), v(3))]);
        let mut b = BinaryTree::new(Blake3);
        b.apply(&[(key(0x80, 9), v(3))]);
        b.apply(&[(key(2, 5), v(2)), (key(1, 0), v(1))]);
        assert_eq!(a.root(), b.root());

        let before = a.root();
        a.apply(&[(key(7, 7), v(9))]);
        assert_ne!(a.root(), before);
        a.apply(&[(key(7, 7), None)]);
        assert_eq!(a.root(), before);
    }

    fn check_proofs<H: Hasher>(h: H) {
        let mut t = BinaryTree::new(h);
        t.apply(&[(key(1, 0), v(1)), (key(1, 200), v(4)), (key(2, 5), v(2)), (key(0x80, 9), v(3))]);
        let root = t.root();
        for (k, expect) in [
            (key(1, 0), v(1)),    // present
            (key(1, 200), v(4)),  // present, same stem
            (key(1, 1), None),    // absent sub-index in existing stem
            (key(3, 0), None),    // absent stem, path ends at another stem
            (key(0xff, 0), None), // absent stem
        ] {
            let p = t.prove(&k);
            assert_eq!(p.value, expect);
            assert_eq!(p.verify(t.hasher(), &root), Ok(()), "{:?}", k);
        }
    }

    #[test]
    fn inclusion_and_absence_proofs_verify() {
        check_proofs(Blake3);
        check_proofs(Poseidon2KoalaBear::new());
    }

    #[test]
    fn tampered_proofs_fail() {
        let mut t = BinaryTree::new(Blake3);
        t.apply(&[(key(1, 0), v(1)), (key(2, 5), v(2)), (key(0x80, 9), v(3))]);
        let root = t.root();

        let mut p = t.prove(&key(1, 0));
        p.value = v(99);
        assert_eq!(p.verify(&Blake3, &root), Err(ProofError::RootMismatch));

        let mut p = t.prove(&key(1, 0));
        p.value = None; // claim absence of a present key
        assert!(p.verify(&Blake3, &root).is_err());

        let mut p = t.prove(&key(3, 0));
        if let Bottom::OtherStem { stem, .. } = &mut p.bottom {
            stem[0] ^= 0x01; // keep the prefix bit, change the stem
        }
        assert!(p.verify(&Blake3, &root).is_err());

        let mut p = t.prove(&key(1, 0));
        if let Some(s) = p.stem_path.first_mut() {
            s[0] ^= 1;
        }
        assert!(p.verify(&Blake3, &root).is_err());
    }

    #[test]
    fn poseidon2_rejects_noncanonical_sibling() {
        let mut t = BinaryTree::new(Poseidon2KoalaBear::new());
        t.apply(&[(key(1, 0), v(1)), (key(2, 5), v(2))]);
        let mut p = t.prove(&key(1, 0));
        p.stem_path[0] = [0xff; 32];
        assert_eq!(p.verify(t.hasher(), &t.root()), Err(ProofError::NonCanonicalDigest));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn random_trees_every_key_proves(entries in proptest::collection::btree_map(any::<[u8; 32]>(), any::<[u8; 32]>(), 1..40),
                                         probe in any::<[u8; 32]>()) {
            let mut t = BinaryTree::new(Blake3);
            let writes: Vec<_> = entries.iter().map(|(k, v)| (*k, Some(*v))).collect();
            t.apply(&writes);
            let root = t.root();
            for (k, val) in &entries {
                let p = t.prove(k);
                prop_assert_eq!(p.value, Some(*val));
                prop_assert!(p.verify(&Blake3, &root).is_ok());
            }
            let p = t.prove(&probe);
            prop_assert_eq!(p.value, entries.get(&probe).copied());
            prop_assert!(p.verify(&Blake3, &root).is_ok());
        }
    }
}
