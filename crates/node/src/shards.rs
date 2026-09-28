//! Era shards (roadmap B5, design only; nothing in the node calls this yet).
//!
//! Each sealed era file is cut into 32 Reed-Solomon shards of which any 16
//! restore it (`commonware_coding::ReedSolomon` over BLAKE3: a Merkle
//! commitment over the shards, a path per shard). Every Mac keeps the shards
//! a public seed assigns it, so old eras survive even when few Macs keep whole
//! era files, at 2x the era's size across the network instead of one copy per
//! Mac.
//!
//! Trust: a shard is checked against the shard commitment, and the commitment
//! is only as good as whoever computed it. Phase 1 needs nothing more, because
//! the decoded bytes are an era file, which is checked against the era root in
//! the certified history (`era::read`, `era_net::verify`); a bad commitment or
//! a bad shard only makes a restore fail, and the node asks other holders. A
//! later phase puts each era's shard commitment on chain (the beacon carries
//! shard proofs), which storage rewards would need; phase 1 gives shards no
//! reward weight.
//!
//! Assignment: rendezvous (highest random weight) hashing. Shard `i` of era
//! `e` goes to the `replicas` nodes with the lowest
//! BLAKE3("aether-shard" ‖ seed ‖ e ‖ i ‖ node id). The seed is a
//! committee-signed draw seed (unknown before its draw, so nobody picks which
//! eras it will hold), every node computes the same assignment from public
//! data, and a node joining or leaving moves only the shards it wins or held.

use commonware_codec::{Decode as _, Encode as _};
use commonware_coding::{CodecConfig, Config, ReedSolomon, Scheme};
use commonware_cryptography::Blake3;
use commonware_parallel::Sequential;
use std::num::NonZeroU16;

/// Shards any restore needs.
pub const MIN_SHARDS: u16 = 16;
/// Shards per era.
pub const TOTAL_SHARDS: u16 = 32;

type Rs = ReedSolomon<Blake3>;
pub type Commitment = <Rs as Scheme>::Commitment;
pub type Shard = <Rs as Scheme>::Shard;
pub type CheckedShard = <Rs as Scheme>::CheckedShard;

pub fn config() -> Config {
    Config {
        minimum_shards: NonZeroU16::new(MIN_SHARDS).expect("nonzero"),
        extra_shards: NonZeroU16::new(TOTAL_SHARDS - MIN_SHARDS).expect("nonzero"),
    }
}

#[derive(Debug)]
pub enum ShardError {
    Coding(String),
    /// Fewer than `MIN_SHARDS` distinct checked shards.
    TooFew(usize),
    Encoding,
}

impl std::fmt::Display for ShardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

/// Cut an era file into `TOTAL_SHARDS` shards and their commitment.
pub fn encode(era_file: &[u8]) -> Result<(Commitment, Vec<Shard>), ShardError> {
    Rs::encode(&config(), era_file, &Sequential).map_err(|e| ShardError::Coding(format!("{e:?}")))
}

/// Check shard `index` against `commitment` (its Merkle path).
pub fn check(commitment: &Commitment, index: u16, shard: &Shard) -> Result<CheckedShard, ShardError> {
    Rs::check(&config(), commitment, index, shard).map_err(|e| ShardError::Coding(format!("{e:?}")))
}

/// Restore the era file from any `MIN_SHARDS` checked shards.
pub fn decode<'a>(commitment: &Commitment, shards: impl Iterator<Item = &'a CheckedShard>) -> Result<Vec<u8>, ShardError> {
    let shards: Vec<&CheckedShard> = shards.collect();
    if shards.len() < MIN_SHARDS as usize {
        return Err(ShardError::TooFew(shards.len()));
    }
    Rs::decode(&config(), commitment, shards.into_iter(), &Sequential).map_err(|e| ShardError::Coding(format!("{e:?}")))
}

/// A shard's bytes for disk or the wire.
pub fn to_bytes(shard: &Shard) -> Vec<u8> {
    shard.encode().to_vec()
}

/// A shard from bytes (at most `max` bytes of shard data).
pub fn from_bytes(bytes: &[u8], max: usize) -> Result<Shard, ShardError> {
    Shard::decode_cfg(bytes, &CodecConfig { maximum_shard_size: max }).map_err(|_| ShardError::Encoding)
}

fn score(seed: &[u8; 32], era: u64, shard: u16, node: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"aether-shard");
    h.update(seed);
    h.update(&era.to_be_bytes());
    h.update(&shard.to_be_bytes());
    h.update(node);
    *h.finalize().as_bytes()
}

/// The `replicas` nodes (indices into `nodes`) that hold shard `shard` of era `era`.
pub fn holders(seed: &[u8; 32], era: u64, shard: u16, nodes: &[[u8; 32]], replicas: usize) -> Vec<usize> {
    let mut ranked: Vec<(usize, [u8; 32])> = nodes.iter().enumerate().map(|(i, n)| (i, score(seed, era, shard, n))).collect();
    ranked.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().take(replicas).map(|(i, _)| i).collect()
}

/// The shards of era `era` that `node` holds.
pub fn shards_of(seed: &[u8; 32], era: u64, node: &[u8; 32], nodes: &[[u8; 32]], replicas: usize) -> Vec<u16> {
    let Some(me) = nodes.iter().position(|n| n == node) else { return vec![] };
    (0..TOTAL_SHARDS).filter(|&i| holders(seed, era, i, nodes, replicas).contains(&me)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(n: usize, salt: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        let mut x = blake3::hash(&[salt]);
        while out.len() < n {
            out.extend_from_slice(x.as_bytes());
            x = blake3::hash(x.as_bytes());
        }
        out.truncate(n);
        out
    }

    fn checked(c: &Commitment, shards: &[Shard]) -> Vec<CheckedShard> {
        shards.iter().enumerate().map(|(i, s)| check(c, i as u16, s).unwrap()).collect()
    }

    #[test]
    fn any_16_of_32_restore_the_era() {
        let era = data(300_001, 1);
        let (c, shards) = encode(&era).unwrap();
        assert_eq!(shards.len(), TOTAL_SHARDS as usize);
        let all = checked(&c, &shards);
        let total: usize = shards.iter().map(|s| to_bytes(s).len()).sum();
        println!("{} B era -> {} B in 32 shards ({:.2}x)", era.len(), total, total as f64 / era.len() as f64);
        assert!(total < era.len() * 23 / 10, "about 2x plus Merkle paths");
        // Data shards only, parity shards only, interleaved, and pseudo-random subsets.
        let mut subsets: Vec<Vec<usize>> = vec![(0..16).collect(), (16..32).collect(), (0..32).step_by(2).collect(), (1..32).step_by(2).collect()];
        for k in 0..20u8 {
            let mut idx: Vec<usize> = (0..32).collect();
            idx.sort_by_key(|i| blake3::hash(&[k, *i as u8]).as_bytes()[0]);
            idx.truncate(16);
            subsets.push(idx);
        }
        for s in subsets {
            let back = decode(&c, s.iter().map(|&i| &all[i])).unwrap();
            assert_eq!(back, era, "subset {s:?}");
        }
        // 15 are not enough.
        assert!(matches!(decode(&c, all[..15].iter()), Err(ShardError::TooFew(15))));
    }

    #[test]
    fn bad_shards_and_foreign_commitments_are_refused() {
        let (c, shards) = encode(&data(50_000, 2)).unwrap();
        // A shard under another index, or with a flipped bit, fails its check.
        assert!(check(&c, 1, &shards[0]).is_err());
        let mut bytes = to_bytes(&shards[3]);
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let flipped = from_bytes(&bytes, 1 << 20).map(|s| check(&c, 3, &s).is_err()).unwrap_or(true);
        assert!(flipped, "a corrupted shard never checks");
        // Shards round-trip through bytes.
        assert_eq!(from_bytes(&to_bytes(&shards[5]), 1 << 20).unwrap(), shards[5]);
        // Shards of another era do not check against this era's commitment.
        let (c2, shards2) = encode(&data(50_000, 3)).unwrap();
        assert_ne!(c, c2);
        assert!(check(&c, 0, &shards2[0]).is_err());
        // Mixing shards checked under different commitments does not decode.
        let a = checked(&c, &shards);
        let b = checked(&c2, &shards2);
        let mixed: Vec<&CheckedShard> = a[..8].iter().chain(b[8..16].iter()).collect();
        assert!(decode(&c, mixed.into_iter()).is_err());
    }

    #[test]
    fn the_seed_assigns_every_shard_evenly_and_stably() {
        let nodes: Vec<[u8; 32]> = (0..64u8).map(|i| *blake3::hash(&[b'n', i]).as_bytes()).collect();
        let seed = [9u8; 32];
        let replicas = 3;
        let mut load = vec![0usize; nodes.len()];
        for era in 0..100u64 {
            for s in 0..TOTAL_SHARDS {
                let h = holders(&seed, era, s, &nodes, replicas);
                assert_eq!(h.len(), replicas);
                assert_eq!(h, holders(&seed, era, s, &nodes, replicas), "deterministic");
                for i in h {
                    load[i] += 1;
                }
            }
        }
        // 100 eras x 32 shards x 3 replicas over 64 nodes: 150 each on average.
        let (min, max) = (*load.iter().min().unwrap(), *load.iter().max().unwrap());
        println!("shards per node over 100 eras: min {min}, max {max}, mean 150");
        assert!(min > 100 && max < 200, "{min}..{max}");
        // A node's own view agrees with the per-shard view.
        let mine = shards_of(&seed, 7, &nodes[5], &nodes, replicas);
        for s in 0..TOTAL_SHARDS {
            assert_eq!(mine.contains(&s), holders(&seed, 7, s, &nodes, replicas).contains(&5));
        }
        // A node leaving moves only the shards it held.
        let fewer: Vec<[u8; 32]> = nodes.iter().copied().filter(|n| *n != nodes[5]).collect();
        for s in 0..TOTAL_SHARDS {
            let before: Vec<[u8; 32]> = holders(&seed, 7, s, &nodes, replicas).into_iter().map(|i| nodes[i]).collect();
            let after: Vec<[u8; 32]> = holders(&seed, 7, s, &fewer, replicas).into_iter().map(|i| fewer[i]).collect();
            let kept = before.iter().filter(|n| after.contains(n)).count();
            let expected = if before.contains(&nodes[5]) { replicas - 1 } else { replicas };
            assert_eq!(kept, expected, "shard {s}");
        }
        // Another seed, another assignment.
        let other: Vec<Vec<usize>> = (0..TOTAL_SHARDS).map(|s| holders(&[8u8; 32], 7, s, &nodes, replicas)).collect();
        let this: Vec<Vec<usize>> = (0..TOTAL_SHARDS).map(|s| holders(&seed, 7, s, &nodes, replicas)).collect();
        assert_ne!(other, this);
    }
}
