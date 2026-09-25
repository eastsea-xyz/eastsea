use crate::{Digest, HashId, Hasher};

/// BLAKE3 with separate derive-key contexts for bytes and node compression.
#[derive(Clone, Copy, Debug, Default)]
pub struct Blake3;

const BYTES_CTX: &str = "aether 2026-09 hash_bytes v1";
const NODE_CTX: &str = "aether 2026-09 compress v1";

impl Hasher for Blake3 {
    const ID: HashId = HashId::Blake3;

    fn hash_bytes(&self, data: &[u8]) -> Digest {
        let mut h = blake3::Hasher::new_derive_key(BYTES_CTX);
        h.update(data);
        *h.finalize().as_bytes()
    }

    fn compress(&self, left: &Digest, right: &Digest) -> Digest {
        let mut h = blake3::Hasher::new_derive_key(NODE_CTX);
        h.update(left);
        h.update(right);
        *h.finalize().as_bytes()
    }
}
