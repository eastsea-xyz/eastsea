//! Stateless witness: the part of the tree a block touches, enough to check the
//! pre-state root and compute the post-state root without the rest of the state
//! (docs/design/06-proving.md, launch plan step 6).
//!
//! A witness lists
//! - **known stems**: every stem a block reads or writes, with all its values;
//!   and stems the paths end at instead (absence), with only their subtree root;
//! - **opaque nodes**: untouched subtrees, each given as the two child digests of
//!   an internal node (at a stem-bit prefix).
//!
//! Its root is computed with the tree's own canonical rules (a subtree holding
//! one stem collapses to it, an empty one is ZERO). An opaque node is an
//! internal node (its digest is a `compress` of two children, never a stem-node
//! hash, which lives in another hash domain), so it holds at least two stems:
//! no write outside it can make it collapse. The root therefore equals the full
//! tree's root before and after any writes the witness covers; a read or write
//! it does not cover panics (`Missing`), so a prover with an incomplete or
//! forged witness produces no proof rather than a wrong one.

use crate::tree::{compress_z, split_key, stem_bit, stem_node_hash, BinaryTree, Stem, StemNode, TreeKey, Value, STEM_BITS};
use aether_hash::{Digest, Hasher, ZERO};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Witness {
    /// Stems with all their values (touched) or only their subtree root (not touched).
    pub stems: Vec<(Stem, StemWitness)>,
    /// Untouched internal nodes: (depth, prefix, left child, right child). The
    /// prefix is the node's first `depth` stem bits, the rest zero.
    pub opaque: Vec<Opaque>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StemWitness {
    Values(Vec<(u8, Value)>),
    Root(Digest),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opaque {
    pub depth: u16,
    pub prefix: Stem,
    pub left: Digest,
    pub right: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessError {
    Malformed(&'static str),
}

#[derive(Clone)]
enum Known {
    Values(StemNode),
    Root(Digest),
}

/// A tree that knows only a witness's part of the state.
#[derive(Clone)]
pub struct PartialTree<H: Hasher> {
    hasher: H,
    stems: BTreeMap<Stem, Known>,
    /// Sorted by prefix; never nested, never above a known stem.
    opaque: Vec<Opaque>,
}

/// The first `depth` bits of `stem`, the rest zero.
fn masked(stem: &Stem, depth: usize) -> Stem {
    let mut out = [0u8; 31];
    let full = depth / 8;
    out[..full].copy_from_slice(&stem[..full]);
    if !depth.is_multiple_of(8) {
        out[full] = stem[full] & (0xffu8 << (8 - depth % 8));
    }
    out
}

fn covers(o: &Opaque, stem: &Stem) -> bool {
    masked(stem, o.depth as usize) == o.prefix
}

impl<H: Hasher> PartialTree<H> {
    /// Build and validate (the root is computed once here).
    pub fn new(hasher: H, w: &Witness) -> Result<Self, WitnessError> {
        let bad = WitnessError::Malformed;
        let mut stems = BTreeMap::new();
        for (stem, sw) in &w.stems {
            let known = match sw {
                StemWitness::Values(vals) => {
                    let mut node = StemNode::empty();
                    for (sub, v) in vals {
                        if *v == [0u8; 32] || node.values.insert(*sub, *v).is_some() {
                            return Err(bad("zero or repeated value"));
                        }
                    }
                    if node.values.is_empty() {
                        return Err(bad("stem without values"));
                    }
                    node.subtree_root = node.root(&hasher);
                    Known::Values(node)
                }
                StemWitness::Root(r) if *r != ZERO && hasher.is_canonical(r) => Known::Root(*r),
                StemWitness::Root(_) => return Err(bad("stem root")),
            };
            if stems.insert(*stem, known).is_some() {
                return Err(bad("repeated stem"));
            }
        }
        let mut opaque = w.opaque.clone();
        opaque.sort_by(|a, b| a.prefix.cmp(&b.prefix).then(a.depth.cmp(&b.depth)));
        for o in &opaque {
            let depth = o.depth as usize;
            if depth >= STEM_BITS || masked(&o.prefix, depth) != o.prefix {
                return Err(bad("opaque prefix"));
            }
            if (o.left == ZERO && o.right == ZERO) || !hasher.is_canonical(&o.left) || !hasher.is_canonical(&o.right) {
                return Err(bad("opaque children"));
            }
        }
        // The root computation rejects nested opaque nodes and known stems inside one.
        let t = PartialTree { hasher, stems, opaque };
        t.try_root()?;
        Ok(t)
    }

    pub fn hasher(&self) -> &H {
        &self.hasher
    }

    /// The value at `key` (None = absent). Panics if the witness does not cover it.
    pub fn get(&self, key: &TreeKey) -> Option<Value> {
        let (stem, sub) = split_key(key);
        match self.stems.get(&stem) {
            Some(Known::Values(n)) => n.values.get(&sub).copied(),
            Some(Known::Root(_)) => missing(key),
            None if self.in_opaque(&stem) => missing(key),
            None => None,
        }
    }

    /// Apply writes (None or zero deletes). Panics on a key the witness does not cover.
    pub fn apply(&mut self, writes: &[(TreeKey, Option<Value>)]) {
        let mut touched = std::collections::BTreeSet::new();
        for (k, v) in writes {
            let (stem, sub) = split_key(k);
            if !self.stems.contains_key(&stem) && self.in_opaque(&stem) {
                missing(k);
            }
            let node = match self.stems.entry(stem).or_insert_with(|| Known::Values(StemNode::empty())) {
                Known::Values(n) => n,
                Known::Root(_) => missing(k),
            };
            match v.filter(|v| *v != [0u8; 32]) {
                Some(v) => node.values.insert(sub, v),
                None => node.values.remove(&sub),
            };
            touched.insert(stem);
        }
        for stem in touched {
            if let Some(Known::Values(n)) = self.stems.get_mut(&stem) {
                if n.values.is_empty() {
                    self.stems.remove(&stem);
                } else {
                    n.subtree_root = n.root(&self.hasher);
                }
            }
        }
    }

    pub fn root(&self) -> Digest {
        self.try_root().expect("validated witness")
    }

    /// Whether `stem` lies under an opaque node. They never overlap, so only
    /// the last one at or before it (by prefix) can hold it.
    fn in_opaque(&self, stem: &Stem) -> bool {
        let i = self.opaque.partition_point(|o| o.prefix <= *stem);
        i > 0 && covers(&self.opaque[i - 1], stem)
    }

    fn try_root(&self) -> Result<Digest, WitnessError> {
        let stems: Vec<(&Stem, Digest)> = self
            .stems
            .iter()
            .map(|(s, k)| {
                (
                    s,
                    match k {
                        Known::Values(n) => n.subtree_root,
                        Known::Root(r) => *r,
                    },
                )
            })
            .collect();
        self.subtree(&stems, &self.opaque, 0)
    }

    fn subtree(&self, stems: &[(&Stem, Digest)], opaque: &[Opaque], depth: usize) -> Result<Digest, WitnessError> {
        match (stems, opaque) {
            ([], []) => Ok(ZERO),
            ([(s, r)], []) => Ok(stem_node_hash(&self.hasher, s, r)),
            ([], [o]) if o.depth as usize == depth => Ok(compress_z(&self.hasher, &o.left, &o.right)),
            _ if depth >= STEM_BITS || opaque.iter().any(|o| o.depth as usize <= depth) => Err(WitnessError::Malformed("opaque node placement")),
            _ => {
                let s = stems.partition_point(|(s, _)| !stem_bit(s, depth));
                let o = opaque.partition_point(|o| !stem_bit(&o.prefix, depth));
                let l = self.subtree(&stems[..s], &opaque[..o], depth + 1)?;
                let r = self.subtree(&stems[s..], &opaque[o..], depth + 1)?;
                Ok(compress_z(&self.hasher, &l, &r))
            }
        }
    }
}

fn missing(key: &TreeKey) -> ! {
    panic!("stateless witness does not cover key {}", hex_key(key))
}

fn hex_key(k: &TreeKey) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

impl<H: Hasher> BinaryTree<H> {
    /// The witness for a block that touches `keys` (reads and writes).
    pub fn witness(&self, keys: &[TreeKey]) -> Witness {
        let mut targets: Vec<Stem> = keys.iter().map(|k| split_key(k).0).collect();
        targets.sort();
        targets.dedup();
        let all: Vec<(&Stem, &StemNode)> = self.stems.iter().collect();
        let mut w = Witness::default();
        self.collect(&all, &targets, 0, &mut w);
        w
    }

    fn collect(&self, stems: &[(&Stem, &StemNode)], targets: &[Stem], depth: usize, w: &mut Witness) {
        match stems {
            [] => {}
            [(s, n)] => {
                let sw = if targets.binary_search(s).is_ok() {
                    StemWitness::Values(n.values.iter().map(|(k, v)| (*k, *v)).collect())
                } else {
                    StemWitness::Root(n.subtree_root)
                };
                w.stems.push((**s, sw));
            }
            _ => {
                let split = stems.partition_point(|(s, _)| !stem_bit(s, depth));
                if targets.is_empty() {
                    w.opaque.push(Opaque {
                        depth: depth as u16,
                        prefix: masked(stems[0].0, depth),
                        left: self.subtree(&stems[..split], depth + 1),
                        right: self.subtree(&stems[split..], depth + 1),
                    });
                    return;
                }
                let t = targets.partition_point(|s| !stem_bit(s, depth));
                self.collect(&stems[..split], &targets[..t], depth + 1, w);
                self.collect(&stems[split..], &targets[t..], depth + 1, w);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_hash::Blake3;
    use proptest::prelude::*;

    fn tree(entries: &BTreeMap<[u8; 32], [u8; 32]>) -> BinaryTree<Blake3> {
        let mut t = BinaryTree::new(Blake3);
        t.apply(&entries.iter().map(|(k, v)| (*k, Some(*v))).collect::<Vec<_>>());
        t
    }

    fn key(stem_byte: u8, sub: u8) -> TreeKey {
        let mut k = [stem_byte; 32];
        k[31] = sub;
        k
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]
        /// Reads, inserts, updates and deletions through a witness give the full tree's roots.
        #[test]
        fn witness_roots_match_the_full_tree(
            entries in proptest::collection::btree_map(any::<[u8; 32]>(), any::<[u8; 32]>(), 0..40),
            fresh in proptest::collection::vec((any::<[u8; 32]>(), any::<[u8; 32]>()), 0..6),
            picks in proptest::collection::vec((any::<prop::sample::Index>(), any::<Option<[u8; 32]>>()), 0..8),
            reads in proptest::collection::vec(any::<[u8; 32]>(), 0..4),
        ) {
            let full = tree(&entries);
            let existing: Vec<[u8; 32]> = entries.keys().copied().collect();
            let mut writes: Vec<(TreeKey, Option<Value>)> = fresh.iter().map(|(k, v)| (*k, Some(*v))).collect();
            if !existing.is_empty() {
                writes.extend(picks.iter().map(|(i, v)| (*i.get(&existing), *v)));
            }
            let mut keys: Vec<TreeKey> = writes.iter().map(|(k, _)| *k).collect();
            keys.extend(&reads);
            let w = full.witness(&keys);
            let mut partial = PartialTree::new(Blake3, &w).unwrap();
            prop_assert_eq!(partial.root(), full.root());
            for k in &keys {
                prop_assert_eq!(partial.get(k), full.get(k));
            }
            let mut after = full.clone();
            after.apply(&writes);
            partial.apply(&writes);
            prop_assert_eq!(partial.root(), after.root());
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        /// Crowded stems (shared prefixes, many values per stem, frequent deletions).
        #[test]
        fn crowded_trees_match_too(
            entries in proptest::collection::btree_map((0u8..12, 0u8..6), 1u8..=255, 0..30),
            writes in proptest::collection::vec(((0u8..12, 0u8..6), proptest::option::of(1u8..=255)), 0..10),
        ) {
            let k = |(s, sub): (u8, u8)| key(s, sub);
            let full = tree(&entries.iter().map(|(ks, v)| (k(*ks), [*v; 32])).collect());
            let writes: Vec<(TreeKey, Option<Value>)> = writes.iter().map(|(ks, v)| (k(*ks), v.map(|v| [v; 32]))).collect();
            let keys: Vec<TreeKey> = writes.iter().map(|(k, _)| *k).collect();
            let mut partial = PartialTree::new(Blake3, &full.witness(&keys)).unwrap();
            prop_assert_eq!(partial.root(), full.root());
            let mut after = full.clone();
            after.apply(&writes);
            partial.apply(&writes);
            prop_assert_eq!(partial.root(), after.root());
        }
    }

    #[test]
    fn deleting_a_stem_collapses_its_sibling_like_the_full_tree() {
        // 0x00.. and 0x01.. split deep; 0x80.. sits alone on the right.
        let entries: BTreeMap<_, _> = [(key(0x00, 1), [1; 32]), (key(0x01, 2), [2; 32]), (key(0x80, 3), [3; 32])].into();
        let full = tree(&entries);
        let writes = [(key(0x00, 1), None)];
        let mut partial = PartialTree::new(Blake3, &full.witness(&[key(0x00, 1)])).unwrap();
        let mut after = full.clone();
        after.apply(&writes);
        partial.apply(&writes);
        assert_eq!(partial.root(), after.root());
    }

    #[test]
    fn uncovered_keys_panic() {
        let entries: BTreeMap<_, _> = [(key(0x00, 1), [1; 32]), (key(0x01, 2), [2; 32]), (key(0x80, 3), [3; 32]), (key(0x81, 4), [4; 32])].into();
        let full = tree(&entries);
        let w = full.witness(&[key(0x00, 1)]);
        let p = PartialTree::new(Blake3, &w).unwrap();
        assert!(std::panic::catch_unwind(|| p.get(&key(0x80, 3))).is_err(), "inside an opaque node");
        let mut q = p.clone();
        assert!(std::panic::catch_unwind(move || q.apply(&[(key(0x81, 9), Some([9; 32]))])).is_err());
        let r = PartialTree::new(Blake3, &full.witness(&[key(0x02, 0)])).unwrap();
        // The path to 0x02 ends at stem 0x00 or 0x01 (root only): its values are not known.
        let other = if r.stems.contains_key(&split_key(&key(0x00, 0)).0) { key(0x00, 1) } else { key(0x01, 2) };
        assert!(std::panic::catch_unwind(|| r.get(&other)).is_err());
    }

    #[test]
    fn forged_witnesses_do_not_give_the_root() {
        let entries: BTreeMap<_, _> = [(key(0x00, 1), [1; 32]), (key(0x01, 2), [2; 32]), (key(0x80, 3), [3; 32]), (key(0x81, 4), [4; 32])].into();
        let full = tree(&entries);
        let w = full.witness(&[key(0x00, 1)]);
        // Another value, a dropped opaque node, or a moved one: a different root (or rejected).
        let mut a = w.clone();
        if let Some((_, StemWitness::Values(v))) = a.stems.iter_mut().find(|(_, s)| matches!(s, StemWitness::Values(_))) {
            v[0].1 = [9; 32];
        }
        assert_ne!(PartialTree::new(Blake3, &a).unwrap().root(), full.root());
        let mut b = w.clone();
        b.opaque.clear();
        assert!(PartialTree::new(Blake3, &b).map(|t| t.root() != full.root()).unwrap_or(true));
        let mut c = w.clone();
        c.opaque[0].depth += 1;
        assert!(PartialTree::new(Blake3, &c).map(|t| t.root() != full.root()).unwrap_or(true));
        // Structural nonsense is rejected outright.
        let mut d = w.clone();
        d.opaque.push(d.opaque[0].clone());
        assert!(PartialTree::new(Blake3, &d).is_err(), "nested/repeated opaque");
        let mut e = w;
        e.stems.push((e.stems[0].0, StemWitness::Root([5; 32])));
        assert!(PartialTree::new(Blake3, &e).is_err(), "repeated stem");
    }
}
