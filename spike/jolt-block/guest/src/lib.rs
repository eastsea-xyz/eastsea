//! Jolt guest: execute an Aether block and return the post-state root.
//! The pre-state is given as tree entries (only what the block touches); the
//! host checks the returned root against native execution.

use aether_execution::{execute_block_sequential, BlockContext, WorldState};
use aether_types::{Address, GasVector, TxEnvelope};
use serde::{Deserialize, Serialize};

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

fn accelerate() {
    let _ = aether_crypto::set_p256_backend(jolt_p256);
}

#[derive(Serialize, Deserialize)]
pub struct Witness {
    pub entries: Vec<([u8; 32], [u8; 32])>,
    pub chain_id: u64,
    pub number: u64,
    pub timestamp: u64,
    pub beneficiary: [u8; 20],
    pub txs: Vec<TxEnvelope>,
}

#[jolt::provable(max_input_size = 1048576, heap_size = 268435456, stack_size = 4194304, max_trace_length = 67108864)]
fn prove_block(witness: Vec<u8>) -> [u8; 32] {
    accelerate();
    let w: Witness = serde_json::from_slice(&witness).expect("witness");
    let pre = WorldState::from_parts(w.entries, Default::default());
    let ctx = BlockContext {
        chain_id: w.chain_id,
        number: w.number,
        timestamp: w.timestamp,
        beneficiary: Address::from(w.beneficiary),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: u64::MAX },
    };
    let out = execute_block_sequential(&pre, &ctx, &w.txs).expect("valid block");
    out.state.root().0
}

/// Signature cost alone: verify every tx's P-256 signature.
#[jolt::provable(max_input_size = 1048576, heap_size = 268435456, stack_size = 4194304, max_trace_length = 67108864)]
fn verify_signatures(witness: Vec<u8>) -> u32 {
    accelerate();
    let w: Witness = serde_json::from_slice(&witness).expect("witness");
    w.txs.iter().filter(|t| aether_execution::validate_stateless(t, w.chain_id).is_ok()).count() as u32
}

/// Debug: root of the pre-state alone (tree + Poseidon2, no execution).
#[jolt::provable(max_input_size = 1048576, heap_size = 268435456, stack_size = 4194304, max_trace_length = 67108864)]
fn pre_root(witness: Vec<u8>) -> [u8; 32] {
    let w: Witness = serde_json::from_slice(&witness).expect("witness");
    WorldState::from_parts(w.entries, Default::default()).root().0
}

/// Debug: Poseidon2 of a fixed input.
#[jolt::provable(max_trace_length = 1048576)]
fn hash_probe(x: [u8; 32]) -> [u8; 32] {
    use aether_execution::ChainHasher;
    let h = ChainHasher::new();
    aether_hash_probe(&h, &x)
}

fn aether_hash_probe(h: &aether_execution::ChainHasher, x: &[u8; 32]) -> [u8; 32] {
    use aether_hash::Hasher;
    h.hash_bytes(x)
}

/// Debug: the execution error, if any, as text.
#[jolt::provable(max_input_size = 1048576, heap_size = 268435456, stack_size = 4194304, max_trace_length = 67108864, backtrace = "dwarf")]
fn block_error(witness: Vec<u8>) -> String {
    accelerate();
    let w: Witness = serde_json::from_slice(&witness).expect("witness");
    let pre = WorldState::from_parts(w.entries, Default::default());
    let ctx = BlockContext {
        chain_id: w.chain_id,
        number: w.number,
        timestamp: w.timestamp,
        beneficiary: Address::from(w.beneficiary),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: u64::MAX },
    };
    match execute_block_sequential(&pre, &ctx, &w.txs) {
        Ok(_) => "ok".into(),
        Err(e) => format!("{e:?}"),
    }
}
