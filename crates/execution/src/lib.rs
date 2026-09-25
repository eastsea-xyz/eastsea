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
