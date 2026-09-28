//! Merkle Mountain Range over block hashes (docs/research/history-compression-2026.md).
//!
//! Every block carries the root of the MMR of all blocks before it, so one
//! finality certificate on a block proves every earlier block with a short
//! inclusion path. A node keeps only the peaks (at most 64 digests): appending
//! is amortized O(1) hashes.
//!
//! Hashes have their own domains (tags and lengths other than the state tree's
//! 32/64-byte inputs): leaf = H('L' ‖ height ‖ block hash), merge = H('M' ‖ l ‖ r),
//! bagging = H('B' ‖ right ‖ left) from the smallest peak up. The empty root is ZERO.

use aether_hash::{Digest, Hasher, ZERO};
use serde::{Deserialize, Serialize};

/// The peaks of an MMR: (height of the peak's tree, digest), largest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mmr {
    /// Number of leaves.
    pub leaves: u64,
    pub peaks: Vec<(u8, Digest)>,
}

pub fn leaf<H: Hasher>(h: &H, height: u64, block_hash: &Digest) -> Digest {
    let mut buf = [0u8; 1 + 8 + 32];
    buf[0] = b'L';
    buf[1..9].copy_from_slice(&height.to_be_bytes());
    buf[9..].copy_from_slice(block_hash);
    h.hash_bytes(&buf)
}

fn merge<H: Hasher>(h: &H, l: &Digest, r: &Digest) -> Digest {
    let mut buf = [0u8; 1 + 64];
    buf[0] = b'M';
    buf[1..33].copy_from_slice(l);
    buf[33..].copy_from_slice(r);
    h.hash_bytes(&buf)
}

fn bag<H: Hasher>(h: &H, right: &Digest, left: &Digest) -> Digest {
    let mut buf = [0u8; 1 + 64];
    buf[0] = b'B';
    buf[1..33].copy_from_slice(right);
    buf[33..].copy_from_slice(left);
    h.hash_bytes(&buf)
}

impl Mmr {
    /// Append the next block (its height must be `self.leaves`).
    pub fn append<H: Hasher>(&self, h: &H, height: u64, block_hash: &Digest) -> Mmr {
        let mut peaks = self.peaks.clone();
        let mut node = (0u8, leaf(h, height, block_hash));
        while let Some(&(ph, pd)) = peaks.last() {
            if ph != node.0 {
                break;
            }
            peaks.pop();
            node = (ph + 1, merge(h, &pd, &node.1));
        }
        peaks.push(node);
        Mmr { leaves: self.leaves + 1, peaks }
    }

    /// One digest for all leaves: peaks bagged from the smallest up.
    pub fn root<H: Hasher>(&self, h: &H) -> Digest {
        let mut it = self.peaks.iter().rev();
        let Some(&(_, first)) = it.next() else { return ZERO };
        it.fold(first, |acc, (_, p)| bag(h, &acc, p))
    }
}

/// Inclusion of leaf `index` (of `leaves`) under an MMR root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MmrProof {
    pub index: u64,
    pub leaves: u64,
    /// Siblings from the leaf up to its peak.
    pub path: Vec<Digest>,
    /// Every peak, largest first (the one the path reaches included).
    pub peaks: Vec<Digest>,
}

impl MmrProof {
    /// Check that `leaf_digest` is leaf `index` under `root`.
    pub fn verify<H: Hasher>(&self, h: &H, leaf_digest: &Digest, root: &Digest) -> bool {
        self.verify_node(h, leaf_digest, 0, root)
    }

    /// Check that `node` is the root of the perfect subtree of `2^level` leaves
    /// starting at leaf `index` (an era's identity is such a node, `level` = `ERA_BITS`).
    pub fn verify_node<H: Hasher>(&self, h: &H, node: &Digest, level: u8, root: &Digest) -> bool {
        if self.index >= self.leaves || self.path.len() > 64 || level >= 64 || self.index & ((1u64 << level) - 1) != 0 {
            return false;
        }
        // The peaks' heights follow from the leaf count's binary digits, largest first.
        let heights: Vec<u8> = (0..64u8).rev().filter(|b| self.leaves >> b & 1 == 1).collect();
        if heights.len() != self.peaks.len() {
            return false;
        }
        // Which peak holds the leaf, and its position inside that peak's tree.
        let (mut start, mut target) = (0u64, None);
        for (i, &ht) in heights.iter().enumerate() {
            let size = 1u64 << ht;
            if self.index < start + size {
                target = Some((i, ht, self.index - start));
                break;
            }
            start += size;
        }
        let Some((peak_index, ht, pos)) = target else { return false };
        if ht < level || self.path.len() != (ht - level) as usize {
            return false;
        }
        let mut pos = pos >> level;
        let mut node = *node;
        for sib in &self.path {
            node = if pos & 1 == 0 { merge(h, &node, sib) } else { merge(h, sib, &node) };
            pos >>= 1;
        }
        if node != self.peaks[peak_index] {
            return false;
        }
        let mmr = Mmr { leaves: self.leaves, peaks: heights.into_iter().zip(self.peaks.iter().copied()).collect() };
        mmr.root(h) == *root
    }
}

/// Build the inclusion proof of leaf `index` from all leaves (archive side).
pub fn prove<H: Hasher>(h: &H, leaves: &[Digest], index: u64) -> Option<MmrProof> {
    let n = leaves.len() as u64;
    if index >= n {
        return None;
    }
    let heights: Vec<u8> = (0..64u8).rev().filter(|b| n >> b & 1 == 1).collect();
    let (mut start, mut peaks, mut path) = (0usize, Vec::new(), Vec::new());
    for &ht in &heights {
        let size = 1usize << ht;
        let mut level: Vec<Digest> = leaves[start..start + size].to_vec();
        let inside = (index as usize).checked_sub(start).filter(|p| *p < size);
        let mut pos = inside;
        while level.len() > 1 {
            if let Some(p) = pos {
                path.push(level[p ^ 1]);
                pos = Some(p >> 1);
            }
            level = level.chunks(2).map(|c| merge(h, &c[0], &c[1])).collect();
        }
        peaks.push(level[0]);
        start += size;
    }
    Some(MmrProof { index, leaves: n, path, peaks })
}

/// Blocks per era (docs/research/history-compression-2026.md): 8192 = 2^13, so
/// every complete era is one perfect subtree of the MMR (genesis is leaf 0) and
/// its root, the era's identity, is a node of the MMR.
pub const ERA_BITS: u8 = 13;
pub const ERA_LEN: u64 = 1 << ERA_BITS;

/// Reduce one perfect level to its root, collecting the siblings of `pos` on the way.
fn reduce<H: Hasher>(h: &H, mut level: Vec<Digest>, mut pos: Option<usize>, path: &mut Vec<Digest>) -> Digest {
    while level.len() > 1 {
        if let Some(p) = pos {
            path.push(level[p ^ 1]);
            pos = Some(p >> 1);
        }
        level = level.chunks(2).map(|c| merge(h, &c[0], &c[1])).collect();
    }
    level.first().copied().unwrap_or(ZERO)
}

/// Root of a perfect tree of leaf digests (length a power of two): an era's identity.
pub fn subtree_root<H: Hasher>(h: &H, leaves: &[Digest]) -> Digest {
    reduce(h, leaves.to_vec(), None, &mut Vec::new())
}

/// Inclusion proof under the MMR of the first `n` leaves, built from era roots
/// instead of every leaf: `level` 0 proves leaf `index`, `level` `ERA_BITS`
/// proves the root of the era starting at `index` (`MmrProof::verify_node`).
/// `eras` holds the roots of the complete eras below `n`; `era_leaves(e)` gives
/// era `e`'s leaf digests (the whole era, or at least its first `n % ERA_LEN`
/// for the open one). Reads at most two eras' leaves: O(ERA_LEN) hashes, not O(n).
pub fn prove_by_eras<H: Hasher>(
    h: &H,
    n: u64,
    index: u64,
    level: u8,
    eras: &[Digest],
    mut era_leaves: impl FnMut(u64) -> Option<Vec<Digest>>,
) -> Option<MmrProof> {
    let full = n / ERA_LEN;
    if index >= n || (level != 0 && level != ERA_BITS) || (eras.len() as u64) < full || index & ((1u64 << level) - 1) != 0 {
        return None;
    }
    // A level-ERA_BITS node exists only for a complete era.
    if level == ERA_BITS && index / ERA_LEN >= full {
        return None;
    }
    let tail = match n % ERA_LEN {
        0 => Vec::new(),
        k => {
            let mut t = era_leaves(full)?;
            if (t.len() as u64) < k {
                return None;
            }
            t.truncate(k as usize);
            t
        }
    };
    let heights: Vec<u8> = (0..64u8).rev().filter(|b| n >> b & 1 == 1).collect();
    let (mut start, mut peaks, mut path) = (0u64, Vec::new(), Vec::new());
    for &ht in &heights {
        let size = 1u64 << ht;
        let inside = (start..start + size).contains(&index);
        let peak = if ht >= ERA_BITS {
            // Made of whole eras: the leaf's path inside its era, then over era roots.
            let (first, count) = ((start / ERA_LEN) as usize, (size / ERA_LEN) as usize);
            let mut pos = None;
            if inside {
                let e = index / ERA_LEN;
                if level == 0 {
                    let leaves = era_leaves(e)?;
                    if leaves.len() as u64 != ERA_LEN {
                        return None;
                    }
                    let root = reduce(h, leaves, Some((index % ERA_LEN) as usize), &mut path);
                    if root != eras[e as usize] {
                        return None;
                    }
                }
                pos = Some(e as usize - first);
            }
            reduce(h, eras[first..first + count].to_vec(), pos, &mut path)
        } else {
            // Inside the open era.
            let off = (start - full * ERA_LEN) as usize;
            let pos = inside.then(|| (index - start) as usize >> level);
            let level_nodes = tail[off..off + size as usize].to_vec();
            reduce(h, level_nodes, pos, &mut path)
        };
        peaks.push(peak);
        start += size;
    }
    Some(MmrProof { index, leaves: n, path, peaks })
}

/// What a node keeps to prove any earlier block: the roots of complete eras
/// (32 bytes per 8192 blocks) and the leaves of the open era. Leaves of older
/// eras come back from the archive (era files, or the blocks kept locally).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EraIndex {
    pub eras: Vec<Digest>,
    pub open: Vec<Digest>,
}

impl EraIndex {
    pub fn leaves(&self) -> u64 {
        self.eras.len() as u64 * ERA_LEN + self.open.len() as u64
    }

    /// Add the next leaf digest; closing an era keeps only its root.
    pub fn push<H: Hasher>(&mut self, h: &H, leaf_digest: Digest) {
        self.open.push(leaf_digest);
        if self.open.len() as u64 == ERA_LEN {
            self.eras.push(subtree_root(h, &self.open));
            self.open.clear();
        }
    }

    /// Proof of leaf `index` under the MMR of the first `n` leaves (`n` <= `leaves()`).
    pub fn prove<H: Hasher>(&self, h: &H, n: u64, index: u64, mut era_leaves: impl FnMut(u64) -> Option<Vec<Digest>>) -> Option<MmrProof> {
        if n > self.leaves() {
            return None;
        }
        let open = self.eras.len() as u64;
        prove_by_eras(h, n, index, 0, &self.eras, |e| if e == open { Some(self.open.clone()) } else { era_leaves(e) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_hash::Blake3;

    fn block(i: u64) -> Digest {
        let mut d = [0u8; 32];
        d[..8].copy_from_slice(&i.to_be_bytes());
        d
    }

    #[test]
    fn peaks_match_the_leaf_count_and_every_leaf_proves() {
        let h = Blake3;
        let mut mmr = Mmr::default();
        let mut leaves = Vec::new();
        assert_eq!(mmr.root(&h), ZERO);
        for i in 0..37u64 {
            mmr = mmr.append(&h, i, &block(i));
            leaves.push(leaf(&h, i, &block(i)));
            assert_eq!(mmr.peaks.len() as u32, (i + 1).count_ones(), "one peak per set bit");
            let root = mmr.root(&h);
            for j in 0..=i {
                let p = prove(&h, &leaves, j).unwrap();
                assert_eq!(p.peaks, mmr.peaks.iter().map(|(_, d)| *d).collect::<Vec<_>>());
                assert!(p.verify(&h, &leaves[j as usize], &root), "leaf {j} of {}", i + 1);
            }
        }
    }

    /// Era-based proofs equal the all-leaves proofs, at every kind of boundary.
    #[test]
    fn proofs_from_era_roots_equal_proofs_from_all_leaves() {
        let h = Blake3;
        let total = 3 * ERA_LEN + 37;
        let leaves: Vec<Digest> = (0..total).map(|i| leaf(&h, i, &block(i))).collect();
        let mut idx = EraIndex::default();
        let mut mmr = Mmr::default();
        for (i, l) in leaves.iter().enumerate() {
            idx.push(&h, *l);
            mmr = mmr.append(&h, i as u64, &block(i as u64));
        }
        assert_eq!(idx.eras.len(), 3);
        assert_eq!(idx.leaves(), total);
        let era = |e: u64| Some(leaves[(e * ERA_LEN) as usize..((e + 1) * ERA_LEN).min(total) as usize].to_vec());
        for n in [1, 5, ERA_LEN - 1, ERA_LEN, ERA_LEN + 1, 2 * ERA_LEN, 2 * ERA_LEN + 9, 3 * ERA_LEN, total] {
            for index in [0, 1, n / 2, n - 1, ERA_LEN - 1, ERA_LEN, 2 * ERA_LEN + 3].into_iter().filter(|i| *i < n) {
                let want = prove(&h, &leaves[..n as usize], index).unwrap();
                let got = idx.prove(&h, n, index, era).unwrap();
                assert_eq!(got, want, "n {n} index {index}");
            }
        }
        // The full MMR's root is the one appends give.
        let p = idx.prove(&h, total, 5, era).unwrap();
        assert!(p.verify(&h, &leaves[5], &mmr.root(&h)));
    }

    /// An era's root is a node of the MMR: one proof ties a whole era to a later root.
    #[test]
    fn an_era_root_proves_against_a_later_history_root() {
        let h = Blake3;
        let total = 4 * ERA_LEN + 3;
        let leaves: Vec<Digest> = (0..total).map(|i| leaf(&h, i, &block(i))).collect();
        let mut idx = EraIndex::default();
        let mut mmr = Mmr::default();
        for (i, l) in leaves.iter().enumerate() {
            idx.push(&h, *l);
            mmr = mmr.append(&h, i as u64, &block(i as u64));
        }
        let root = mmr.root(&h);
        let open = idx.open.clone();
        for e in 0..4u64 {
            let p = prove_by_eras(&h, total, e * ERA_LEN, ERA_BITS, &idx.eras, |_| Some(open.clone())).unwrap();
            assert!(p.verify_node(&h, &idx.eras[e as usize], ERA_BITS, &root), "era {e}");
            assert!(!p.verify_node(&h, &idx.eras[(e as usize + 1) % 4], ERA_BITS, &root), "another era's root");
            assert!(!p.verify(&h, &idx.eras[e as usize], &root), "an era root is not a leaf");
        }
        assert_eq!(subtree_root(&h, &leaves[..ERA_LEN as usize]), idx.eras[0]);
        // No node for the open era, nor for a position inside an era.
        assert!(prove_by_eras(&h, total, 4 * ERA_LEN, ERA_BITS, &idx.eras, |_| Some(open.clone())).is_none());
        assert!(prove_by_eras(&h, total, 3, ERA_BITS, &idx.eras, |_| Some(open.clone())).is_none());
        // Wrong era leaves are refused, not turned into a bad proof.
        let bad = prove_by_eras(&h, total, 7, 0, &idx.eras, |e| if e == 0 { Some(vec![ZERO; ERA_LEN as usize]) } else { Some(open.clone()) });
        assert!(bad.is_none());
    }

    #[test]
    fn forged_proofs_fail() {
        let h = Blake3;
        let leaves: Vec<Digest> = (0..10u64).map(|i| leaf(&h, i, &block(i))).collect();
        let mut mmr = Mmr::default();
        for i in 0..10u64 {
            mmr = mmr.append(&h, i, &block(i));
        }
        let root = mmr.root(&h);
        let p = prove(&h, &leaves, 3).unwrap();
        assert!(!p.verify(&h, &leaves[4], &root), "another leaf");
        let mut q = p.clone();
        q.index = 2;
        assert!(!q.verify(&h, &leaves[3], &root), "wrong position");
        let mut r = p.clone();
        r.leaves = 11;
        assert!(!r.verify(&h, &leaves[3], &root), "wrong size");
        // The leaf hash binds the height: the same block hash at another height differs.
        assert_ne!(leaf(&h, 3, &block(3)), leaf(&h, 4, &block(3)));
    }
}
