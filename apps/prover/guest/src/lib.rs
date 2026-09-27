//! aether-prover guest: re-execute an Aether block on its stateless witness and
//! output the block statement's commitment (pre-state root checked, post-state
//! root, context, txs, gas). The host embeds this crate's prebuilt ELF; any change
//! here (or in the crates it depends on) changes the program the verifier checks.

use aether_proving::block::{execute, BlockInput};

/// P-256 ECDSA through Jolt's inline (accelerated curve arithmetic).
fn jolt_p256(x: &[u8; 32], y: &[u8; 32], digest: &[u8; 32], r: &[u8; 32], s: &[u8; 32]) -> bool {
    use jolt_inlines_p256::{ecdsa_verify, P256Fr, P256Point};
    fn limbs(be: &[u8; 32]) -> [u64; 4] {
        core::array::from_fn(|i| u64::from_be_bytes(be[24 - 8 * i..32 - 8 * i].try_into().unwrap()))
    }
    // z = digest mod n (digest < 2n, so at most one subtraction).
    const N: [u64; 4] = [0xf3b9cac2fc632551, 0xbce6faada7179e84, 0xffffffffffffffff, 0xffffffff00000000];
    let mut z = limbs(digest);
    if (0..4).rev().map(|i| z[i].cmp(&N[i])).find(|o| o.is_ne()).is_none_or(|o| o.is_gt()) {
        let mut borrow = 0u128;
        for i in 0..4 {
            let d = (z[i] as u128).wrapping_sub(N[i] as u128 + borrow);
            z[i] = d as u64;
            borrow = (d >> 127) & 1;
        }
    }
    let (xl, yl) = (limbs(x), limbs(y));
    let q = [xl[0], xl[1], xl[2], xl[3], yl[0], yl[1], yl[2], yl[3]];
    let (Ok(z), Ok(r), Ok(s), Ok(q)) = (P256Fr::from_u64_arr(&z), P256Fr::from_u64_arr(&limbs(r)), P256Fr::from_u64_arr(&limbs(s)), P256Point::from_u64_arr(&q))
    else {
        return false;
    };
    ecdsa_verify(z, r, s, q).is_ok()
}

/// The state hash's 64-byte keyed BLAKE3 through Jolt's inline (one per tree hash).
fn jolt_keyed64(key: &[u8; 32], left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    use jolt_inlines_blake3::{blake3_keyed64, AlignedHash32};
    let (l, r) = (AlignedHash32::new(*left), AlignedHash32::new(*right));
    let mut k = AlignedHash32::new(*key);
    blake3_keyed64(&l, &r, &mut k);
    *k.as_bytes()
}

fn accelerate() {
    let _ = aether_crypto::set_p256_backend(jolt_p256);
    let _ = aether_hash::set_blake3_backend(jolt_keyed64);
}

/// The block's statement commitment: the witness proves the pre-state root, the
/// block re-executes on it, and the output binds context, txs and both roots
/// (aether_proving::block).
///
/// The postcard `BlockInput` is untrusted advice, not a public input: the
/// verifier needs only the commitment (the statement is "some input makes this
/// program output C without panicking"), and the proof stays small.
#[jolt::provable(max_untrusted_advice_size = 4194304, heap_size = 268435456, stack_size = 4194304, max_trace_length = 67108864)]
fn prove_block(input: jolt::UntrustedAdvice<Vec<u8>>) -> [u8; 32] {
    accelerate();
    let input: BlockInput = postcard::from_bytes(&input).expect("input");
    execute(&input).expect("valid block").commitment()
}
