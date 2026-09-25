//! Execution layer (docs/design/04-execution.md): revm over the EIP-7864 state.

pub mod block;
mod parallel;
pub mod tx;
pub mod world;

pub use block::{
    build_block, build_block_sequential, can_append, execute_block, execute_block_sequential, BlockContext, BlockOutcome, ExecError, ProveGasMeter, Receipt,
};
pub use tx::{sign_call, tx_hash, validate_stateless, EvmCall, TxError};
pub use world::{ChainHasher, Journal, StateError, WorldState};

/// Where the account contract lives (predeployed at genesis). Accounts delegate
/// to it with EIP-7702 to get batched calls (`contracts/src/AetherAccount.sol`).
pub const AETHER_ACCOUNT: alloy_primitives::Address = alloy_primitives::address!("0000000000000000000000000000000000007702");

/// Runtime bytecode of `AetherAccount` (solc 0.8.19, optimizer 200 runs).
pub fn aether_account_code() -> alloy_primitives::Bytes {
    alloy_primitives::Bytes::from(alloy_primitives::hex::decode(include_str!("aether_account.bin.hex").trim()).expect("valid hex"))
}

/// ABI-encode `AetherAccount.execute((address,uint256,bytes)[])`.
pub fn encode_execute(calls: &[(alloy_primitives::Address, alloy_primitives::U256, alloy_primitives::Bytes)]) -> alloy_primitives::Bytes {
    use alloy_primitives::U256;
    fn word(v: U256) -> [u8; 32] {
        v.to_be_bytes::<32>()
    }
    fn padded(data: &[u8]) -> Vec<u8> {
        let mut v = data.to_vec();
        v.resize(data.len().div_ceil(32) * 32, 0);
        v
    }
    // Each tuple: to, value, offset(0x60), len, data (padded).
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
    let mut out = vec![0x3f, 0x70, 0x7e, 0x6b]; // execute((address,uint256,bytes)[])
    out.extend_from_slice(&word(U256::from(0x20)));
    out.extend_from_slice(&word(U256::from(calls.len())));
    let mut offset = 32 * calls.len();
    for t in &tuples {
        out.extend_from_slice(&word(U256::from(offset)));
        offset += t.len();
    }
    for t in tuples {
        out.extend_from_slice(&t);
    }
    out.into()
}
