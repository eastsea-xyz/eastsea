use crate::{Digest, HashId, Hasher};

/// The chain's state hash (D6), shaped for the Jolt `blake3_keyed64` inline
/// (one BLAKE3 compression of 64 bytes under a key) so a proof pays a single
/// inline per tree hash. Every state-tree input is 32 or 64 bytes:
/// - 64-byte input `a ‖ b` (key derivation, stem nodes): `keyed_hash(K64, a ‖ b)`;
/// - 32-byte input `v` (leaf values): `keyed_hash(K32, v ‖ 0^32)`;
/// - internal nodes: `keyed_hash(KNODE, left ‖ right)`;
/// - any other length (tx hashes, blobs): `keyed_hash(KANY, data)`.
///
/// The keys separate the domains (no input of one kind collides with another).
/// Natively this is the `blake3` crate (NEON on Apple silicon); a zkVM guest
/// installs the inline with `set_blake3_backend`. Both compute the same bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Blake3;

impl Blake3 {
    pub fn new() -> Self {
        Blake3
    }
}

struct Keys {
    k32: [u8; 32],
    k64: [u8; 32],
    node: [u8; 32],
    any: [u8; 32],
}

fn keys() -> &'static Keys {
    static KEYS: std::sync::OnceLock<Keys> = std::sync::OnceLock::new();
    KEYS.get_or_init(|| {
        let k = |ctx: &str| *blake3::hash(ctx.as_bytes()).as_bytes();
        Keys {
            k32: k("aether 2026-09 bytes32 v2"),
            k64: k("aether 2026-09 bytes64 v2"),
            node: k("aether 2026-09 compress v2"),
            any: k("aether 2026-09 bytes v2"),
        }
    })
}

/// `keyed64(key, left, right)` = `keyed_hash(key, left ‖ right)`.
pub type Keyed64 = fn(&[u8; 32], &[u8; 32], &[u8; 32]) -> [u8; 32];

static BACKEND: std::sync::OnceLock<Keyed64> = std::sync::OnceLock::new();

/// Install an accelerated 64-byte keyed BLAKE3 (once per process, e.g. a
/// zkVM inline). Returns false if one was already set.
pub fn set_blake3_backend(f: Keyed64) -> bool {
    BACKEND.set(f).is_ok()
}

fn keyed64(key: &[u8; 32], left: &[u8; 32], right: &[u8; 32]) -> Digest {
    match BACKEND.get() {
        Some(f) => f(key, left, right),
        None => {
            let mut h = blake3::Hasher::new_keyed(key);
            h.update(left);
            h.update(right);
            *h.finalize().as_bytes()
        }
    }
}

impl Hasher for Blake3 {
    const ID: HashId = HashId::Blake3;

    fn hash_bytes(&self, data: &[u8]) -> Digest {
        let k = keys();
        match data.len() {
            32 => keyed64(&k.k32, data.try_into().expect("32 bytes"), &[0; 32]),
            64 => keyed64(&k.k64, data[..32].try_into().expect("32 bytes"), data[32..].try_into().expect("32 bytes")),
            _ => *blake3::keyed_hash(&k.any, data).as_bytes(),
        }
    }

    fn compress(&self, left: &Digest, right: &Digest) -> Digest {
        keyed64(&keys().node, left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_length_is_keyed_blake3_in_its_own_domain() {
        let h = Blake3::new();
        let (l, r) = ([1u8; 32], [2u8; 32]);
        let both: Vec<u8> = [l, r].concat();
        let k = keys();
        assert_eq!(h.compress(&l, &r), *blake3::keyed_hash(&k.node, &both).as_bytes());
        assert_eq!(h.hash_bytes(&both), *blake3::keyed_hash(&k.k64, &both).as_bytes());
        assert_eq!(h.hash_bytes(&l), *blake3::keyed_hash(&k.k32, &[l, [0; 32]].concat()).as_bytes());
        assert_eq!(h.hash_bytes(b"tx"), *blake3::keyed_hash(&k.any, b"tx").as_bytes());
        // Domains do not collide: a 32-byte input is not its zero-padded 64-byte form, nor a node.
        assert_ne!(h.hash_bytes(&l), h.hash_bytes(&[l, [0; 32]].concat()));
        assert_ne!(h.hash_bytes(&both), h.compress(&l, &r));
        assert_ne!(h.compress(&l, &r), h.compress(&r, &l));
    }
}
