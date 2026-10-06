//! Contracts on the real EastSea executor; no node process or RPC server is started.
//!
//! - [`harness`]: signed P-256 transactions through `execute_block`, with
//!   parallel/sequential/proposer replay, paid state and rollback checks.
//! - [`recorder`]: per-workflow exact cost records (native plan A0); see
//!   `crates/contracts-onchain/RECORDER.md` for the API and JSON schema.
//! - [`account`]: EastSeaAccount self batches, added-owner relays, ERC-1271.
//! - [`schema`]: strict check of a record against the JSON schema.
pub mod account;
pub mod harness;
pub mod recorder;
pub mod schema;

pub use harness::Harness;
pub use recorder::{StepRecord, WorkflowRecord};
