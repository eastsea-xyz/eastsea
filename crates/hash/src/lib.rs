//! Hasher trait and backends (docs/design/03-traits.md, 05-state.md).
//!
//! The chain config picks one `HashId` at genesis. Poseidon2<KoalaBear, 16> is
//! the default because it is the cheapest hash inside our small-field prover;
//! BLAKE3 is selectable and is the planned switch target once the prover has a
//! Flock-style Boolean gadget (Ethereum dropped Poseidon for L1 in 2026-08).

mod blake3_backend;
mod poseidon2_backend;

pub use blake3_backend::Blake3;
pub use poseidon2_backend::Poseidon2KoalaBear;

pub type Digest = [u8; 32];
pub const ZERO: Digest = [0u8; 32];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HashId {
    Poseidon2KoalaBear16,
    Blake3,
}

pub trait Hasher: Send + Sync + 'static {
    const ID: HashId;

    /// Hash arbitrary bytes. Injective encoding: different inputs never share
    /// a pre-image at the permutation level.
    fn hash_bytes(&self, data: &[u8]) -> Digest;

    /// 2-to-1 compression for Merkle internal nodes.
    fn compress(&self, left: &Digest, right: &Digest) -> Digest;

    /// Domain-separated leaf hash. Leaves are always pre-hashed before they enter
    /// compression (required for Merkle soundness with pseudo-compression).
    fn hash_leaf(&self, key: &[u8; 32], value: &[u8]) -> Digest {
        let mut buf = Vec::with_capacity(1 + 32 + value.len());
        buf.push(0x4c); // 'L'
        buf.extend_from_slice(key);
        buf.extend_from_slice(value);
        self.hash_bytes(&buf)
    }

    /// Whether `d` could have been produced by this hasher. Proof verifiers
    /// reject non-canonical sibling encodings.
    fn is_canonical(&self, _d: &Digest) -> bool {
        true
    }
}

/// Runtime-selected hasher for code that cannot be generic.
#[derive(Clone)]
pub enum AnyHasher {
    Poseidon2(Box<Poseidon2KoalaBear>),
    Blake3(Blake3),
}

impl AnyHasher {
    pub fn new(id: HashId) -> Self {
        match id {
            HashId::Poseidon2KoalaBear16 => AnyHasher::Poseidon2(Box::default()),
            HashId::Blake3 => AnyHasher::Blake3(Blake3),
        }
    }

    pub fn id(&self) -> HashId {
        match self {
            AnyHasher::Poseidon2(_) => HashId::Poseidon2KoalaBear16,
            AnyHasher::Blake3(_) => HashId::Blake3,
        }
    }

    pub fn hash_bytes(&self, d: &[u8]) -> Digest {
        match self {
            AnyHasher::Poseidon2(h) => h.hash_bytes(d),
            AnyHasher::Blake3(h) => h.hash_bytes(d),
        }
    }

    pub fn compress(&self, l: &Digest, r: &Digest) -> Digest {
        match self {
            AnyHasher::Poseidon2(h) => h.compress(l, r),
            AnyHasher::Blake3(h) => h.compress(l, r),
        }
    }

    pub fn hash_leaf(&self, k: &[u8; 32], v: &[u8]) -> Digest {
        match self {
            AnyHasher::Poseidon2(h) => h.hash_leaf(k, v),
            AnyHasher::Blake3(h) => h.hash_leaf(k, v),
        }
    }

    pub fn is_canonical(&self, d: &Digest) -> bool {
        match self {
            AnyHasher::Poseidon2(h) => h.is_canonical(d),
            AnyHasher::Blake3(h) => h.is_canonical(d),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn both() -> [AnyHasher; 2] {
        [AnyHasher::new(HashId::Poseidon2KoalaBear16), AnyHasher::new(HashId::Blake3)]
    }

    #[test]
    fn deterministic_and_distinct_backends() {
        let [p, b] = both();
        assert_eq!(p.hash_bytes(b"abc"), p.hash_bytes(b"abc"));
        assert_ne!(p.hash_bytes(b"abc"), b.hash_bytes(b"abc"));
    }

    #[test]
    fn length_extension_and_trailing_zero_do_not_collide() {
        for h in both() {
            assert_ne!(h.hash_bytes(b"ab"), h.hash_bytes(b"ab\0"));
            assert_ne!(h.hash_bytes(b""), h.hash_bytes(b"\0"));
            assert_ne!(h.hash_bytes(&[0u8; 24]), h.hash_bytes(&[0u8; 25]));
        }
    }

    #[test]
    fn compress_is_order_sensitive() {
        for h in both() {
            let (a, b) = (h.hash_bytes(b"a"), h.hash_bytes(b"b"));
            assert_ne!(h.compress(&a, &b), h.compress(&b, &a));
            assert!(h.is_canonical(&h.compress(&a, &b)));
        }
    }

    #[test]
    fn leaf_is_domain_separated_from_plain_hash() {
        for h in both() {
            let k = [7u8; 32];
            let mut raw = vec![0x4c];
            raw.extend_from_slice(&k);
            raw.extend_from_slice(b"v");
            assert_eq!(h.hash_leaf(&k, b"v"), h.hash_bytes(&raw));
            assert_ne!(h.hash_leaf(&k, b"v"), h.hash_bytes(b"v"));
        }
    }

    #[test]
    fn poseidon2_rejects_noncanonical_digest() {
        let p = Poseidon2KoalaBear::new();
        assert!(p.is_canonical(&ZERO));
        assert!(!p.is_canonical(&[0xff; 32]));
    }

    proptest! {
        #[test]
        fn no_collisions_on_random_distinct_inputs(a in proptest::collection::vec(any::<u8>(), 0..200),
                                                   b in proptest::collection::vec(any::<u8>(), 0..200)) {
            prop_assume!(a != b);
            for h in both() {
                prop_assert_ne!(h.hash_bytes(&a), h.hash_bytes(&b));
            }
        }
    }
}
