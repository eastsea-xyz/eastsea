//! Consensus boundaries (docs/design/03-traits.md, 07-consensus.md).
//!
//! The BFT engine (Commonware simplex) sits behind these traits in
//! `engine_simplex` (next step). This crate already implements the
//! engine-independent rules: committee selection and FOCIL-style inclusion lists.

pub mod committee;
pub mod inclusion;

use aether_types::{Block, BlockHeader, Certificate, Hash, TxEnvelope, TxHash};

pub use committee::{select_committee, CandidateInfo};
pub use inclusion::{missing_inclusions, FifoWithInclusion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    BadParent,
    BadHeight,
    BalMismatch,
    GasExceeded,
    MissingInclusions(Vec<TxHash>),
    InvalidTx(String),
}

pub trait Mempool {
    fn pending(&self, max: usize) -> Vec<(TxHash, TxEnvelope)>;
}

pub trait OrderingPolicy {
    /// Order candidate txs for a block. Must be deterministic in its inputs.
    fn order(&self, txs: Vec<(TxHash, TxEnvelope)>, inclusion_list: &[TxHash]) -> Vec<(TxHash, TxEnvelope)>;
}

pub trait BlockProducer {
    fn propose(&mut self, parent: &BlockHeader, mempool: &dyn Mempool) -> Block;
}

pub trait BlockValidator {
    /// Full validity: header links, BAL equality after execution, gas, inclusion lists.
    fn validate(&self, block: &Block, parent: &BlockHeader) -> Result<(), ValidationError>;
}

pub trait ForkChoice {
    fn head(&self) -> Hash;
    fn on_certificate(&mut self, cert: &Certificate);
}
