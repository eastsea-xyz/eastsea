//! Execution layer (docs/design/04-execution.md): revm over the EIP-7864 state.

pub mod account;
pub mod block;
pub mod fees;
pub mod forks;
mod parallel;
pub mod proofs;
pub mod receipt;
pub mod registry;
pub mod release_log;
pub mod tx;
pub mod world;

pub use account::{encode_execute, encode_set_guardian, AccountCall};
pub use block::{
    build_block, build_block_sequential, call, can_append, check_admission, check_admission_cost, execute_block, execute_block_sequential, AdmissionCost, BlockContext, BlockOutcome, CallResult, Event, ExecError,
    ProveGasMeter, Receipt,
};
pub use fees::{FeePolicy, Settlement, FEE_COLLECTOR, PROVER_ESCROW};
pub use tx::{plain_transfer_gas_limit, recommended_state_budget, sign_call, sign_call_group, sign_call_with, tx_hash, validate_stateless, EvmCall, TxError};
pub use world::{ChainHasher, Journal, StateError, StateWitness, WorldState};

/// Where the account contract lives (predeployed at genesis). Accounts delegate
/// to it with EIP-7702 to get batched calls (`contracts/src/EastSeaAccount.sol`).
pub const AETHER_ACCOUNT: alloy_primitives::Address = alloy_primitives::address!("0000000000000000000000000000000000007702");

/// Runtime bytecode of `EastSeaAccount` (solc 0.8.19, optimizer 200 runs).
pub fn aether_account_code() -> alloy_primitives::Bytes {
    alloy_primitives::Bytes::from(alloy_primitives::hex::decode(include_str!("aether_account.bin.hex").trim()).expect("valid hex"))
}

/// New-genesis account code. The original artifact stays pinned for testnet 7780.
pub fn aether_account_code_v2() -> alloy_primitives::Bytes {
    alloy_primitives::Bytes::from(alloy_primitives::hex::decode(include_str!("aether_account_v2.bin.hex").trim()).expect("valid hex"))
}

/// Only new-genesis networks install the token-capable account code.
pub fn aether_account_code_for_genesis(node_rewards: bool, history_v2: bool) -> alloy_primitives::Bytes {
    if node_rewards || history_v2 { aether_account_code_v2() } else { aether_account_code() }
}
