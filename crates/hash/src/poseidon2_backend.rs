//! Poseidon2 over KoalaBear, width 16, rate 8, 8-element (248-bit) digests.
//!
//! Round constants are Plonky3's published Grain-LFSR constants (the standard,
//! nothing-up-my-sleeve procedure), so any Plonky3-based circuit reproduces
//! this hash exactly.

use crate::{Digest, HashId, Hasher};
use p3_field::{PrimeCharacteristicRing, PrimeField32};
use p3_koala_bear::{default_koalabear_poseidon2_16, KoalaBear, Poseidon2KoalaBear as Perm16};
use p3_symmetric::{CryptographicHasher, Increment, Pad10Sponge, PseudoCompressionFunction, TruncatedPermutation};

const WIDTH: usize = 16;
const RATE: usize = 8;
const OUT: usize = 8;
/// Bytes packed per field element; 2^24 < p = 2^31 - 2^24 + 1, so injective.
const BYTES_PER_ELEM: usize = 3;
const KOALABEAR_P: u32 = 0x7f00_0001;

type Sponge = Pad10Sponge<KoalaBear, Perm16<WIDTH>, Increment<KoalaBear>, WIDTH, RATE, OUT>;
type Compress = TruncatedPermutation<Perm16<WIDTH>, 2, OUT, WIDTH>;

#[derive(Clone)]
pub struct Poseidon2KoalaBear {
    sponge: Sponge,
    compress: Compress,
}

impl Default for Poseidon2KoalaBear {
    fn default() -> Self {
        Self::new()
    }
}

impl Poseidon2KoalaBear {
    pub fn new() -> Self {
        let perm = default_koalabear_poseidon2_16();
        Poseidon2KoalaBear {
            sponge: Pad10Sponge::new(perm.clone(), Increment::new(KoalaBear::ONE)),
            compress: TruncatedPermutation::new(perm),
        }
    }

    /// Length-prefixed 24-bit packing: [len limbs (3)] ++ [3-byte chunks].
    fn encode(data: &[u8]) -> Vec<KoalaBear> {
        let len = data.len() as u64;
        let mut out = Vec::with_capacity(3 + data.len().div_ceil(BYTES_PER_ELEM));
        for i in 0..3 {
            out.push(KoalaBear::from_u32(((len >> (24 * i)) & 0xff_ffff) as u32));
        }
        for chunk in data.chunks(BYTES_PER_ELEM) {
            let mut v = 0u32;
            for (i, b) in chunk.iter().enumerate() {
                v |= (*b as u32) << (8 * i);
            }
            out.push(KoalaBear::from_u32(v));
        }
        out
    }

    fn to_digest(elems: [KoalaBear; OUT]) -> Digest {
        let mut d = [0u8; 32];
        for (i, e) in elems.iter().enumerate() {
            d[4 * i..4 * i + 4].copy_from_slice(&e.as_canonical_u32().to_le_bytes());
        }
        d
    }

    fn from_digest(d: &Digest) -> [KoalaBear; OUT] {
        core::array::from_fn(|i| {
            let v = u32::from_le_bytes(d[4 * i..4 * i + 4].try_into().expect("4 bytes"));
            KoalaBear::from_u32(v)
        })
    }
}

impl Hasher for Poseidon2KoalaBear {
    const ID: HashId = HashId::Poseidon2KoalaBear16;

    fn hash_bytes(&self, data: &[u8]) -> Digest {
        Self::to_digest(self.sponge.hash_iter(Self::encode(data)))
    }

    fn compress(&self, left: &Digest, right: &Digest) -> Digest {
        Self::to_digest(self.compress.compress([Self::from_digest(left), Self::from_digest(right)]))
    }

    fn is_canonical(&self, d: &Digest) -> bool {
        d.as_chunks::<4>().0.iter().all(|c| u32::from_le_bytes(*c) < KOALABEAR_P)
    }
}
