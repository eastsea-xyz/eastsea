//! Execution layer (docs/design/04-execution.md): revm over the EIP-7864 state.

pub mod block;
pub mod fees;
mod parallel;
pub mod tx;
pub mod world;

pub use block::{
    build_block, build_block_sequential, can_append, execute_block, execute_block_sequential, BlockContext, BlockOutcome, ExecError, ProveGasMeter, Receipt,
};
pub use fees::{FeePolicy, Settlement, FEE_COLLECTOR, PROVER_ESCROW};
pub use tx::{sign_call, sign_call_with, tx_hash, validate_stateless, EvmCall, TxError};
pub use world::{ChainHasher, Journal, StateError, WorldState};

/// Where the account contract lives (predeployed at genesis). Accounts delegate
/// to it with EIP-7702 to get batched calls (`contracts/src/AetherAccount.sol`).
pub const AETHER_ACCOUNT: alloy_primitives::Address = alloy_primitives::address!("0000000000000000000000000000000000007702");

/// Runtime bytecode of `AetherAccount` (solc 0.8.19, optimizer 200 runs).
pub fn aether_account_code() -> alloy_primitives::Bytes {
    alloy_primitives::Bytes::from(alloy_primitives::hex::decode(include_str!("aether_account.bin.hex").trim()).expect("valid hex"))
}

/// Calls for `AetherAccount`: (to, value, data).
pub type AccountCall = (alloy_primitives::Address, alloy_primitives::U256, alloy_primitives::Bytes);

fn word(v: alloy_primitives::U256) -> [u8; 32] {
    v.to_be_bytes::<32>()
}

/// ABI encoding of a `(address,uint256,bytes)[]` value (length, offsets, tuples).
fn encode_calls(calls: &[AccountCall]) -> Vec<u8> {
    use alloy_primitives::U256;
    let padded = |data: &[u8]| {
        let mut v = data.to_vec();
        v.resize(data.len().div_ceil(32) * 32, 0);
        v
    };
    let tuples: Vec<Vec<u8>> = calls
        .iter()
        .map(|(to, value, data)| {
            let mut t = Vec::new();
            t.extend_from_slice(&to.into_word().0);
            t.extend_from_slice(&word(*value));
            t.extend_from_slice(&word(U256::from(0x60)));
            t.extend_from_slice(&word(U256::from(data.len())));
            t.extend_from_slice(&padded(data));
            t
        })
        .collect();
    let mut out = word(U256::from(calls.len())).to_vec();
    let mut offset = 32 * calls.len();
    for t in &tuples {
        out.extend_from_slice(&word(U256::from(offset)));
        offset += t.len();
    }
    for t in tuples {
        out.extend_from_slice(&t);
    }
    out
}

/// `AetherAccount.execute((address,uint256,bytes)[])`.
pub fn encode_execute(calls: &[AccountCall]) -> alloy_primitives::Bytes {
    let mut out = vec![0x3f, 0x70, 0x7e, 0x6b];
    out.extend_from_slice(&word(alloy_primitives::U256::from(0x20)));
    out.extend_from_slice(&encode_calls(calls));
    out.into()
}

/// `AetherAccount.setGuardian(bytes32 x, bytes32 y)`: a second device's P-256 key (uncompressed x, y).
pub fn encode_set_guardian(x: [u8; 32], y: [u8; 32]) -> alloy_primitives::Bytes {
    let mut out = vec![0xbe, 0x58, 0xd9, 0xf2];
    out.extend_from_slice(&x);
    out.extend_from_slice(&y);
    out.into()
}

/// The bytes a guardian signs (P-256 over SHA-256, as a Secure Enclave does):
/// `abi.encode(chainid, account, nonce, calls)`; the contract checks
/// `sha256(...)` of this with P256VERIFY.
pub fn guardian_message(chain_id: u64, account: alloy_primitives::Address, nonce: u64, calls: &[AccountCall]) -> Vec<u8> {
    use alloy_primitives::U256;
    let mut out = Vec::new();
    out.extend_from_slice(&word(U256::from(chain_id)));
    out.extend_from_slice(&account.into_word().0);
    out.extend_from_slice(&word(U256::from(nonce)));
    out.extend_from_slice(&word(U256::from(0x80)));
    out.extend_from_slice(&encode_calls(calls));
    out
}

/// `AetherAccount.guardianExecute(calls, r, s)`.
pub fn encode_guardian_execute(calls: &[AccountCall], r: [u8; 32], s: [u8; 32]) -> alloy_primitives::Bytes {
    let mut out = vec![0x22, 0xa1, 0xca, 0x2d];
    out.extend_from_slice(&word(alloy_primitives::U256::from(0x60)));
    out.extend_from_slice(&r);
    out.extend_from_slice(&s);
    out.extend_from_slice(&encode_calls(calls));
    out.into()
}
