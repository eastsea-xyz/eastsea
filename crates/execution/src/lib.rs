//! Execution layer (docs/design/04-execution.md): revm over the EIP-7864 state.

pub mod account;
pub mod block;
pub mod fees;
mod parallel;
pub mod tx;
pub mod world;

pub use account::{encode_execute, encode_set_guardian, AccountCall};
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
