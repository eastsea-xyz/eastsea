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
        if self.index >= self.leaves || self.path.len() > 64 {
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
        let Some((peak_index, ht, mut pos)) = target else { return false };
        if self.path.len() != ht as usize {
            return false;
        }
        let mut node = *leaf_digest;
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
