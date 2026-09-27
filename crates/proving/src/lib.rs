//! Proving boundary (docs/design/03-traits.md, 06-proving.md).
//!
//! zkVM backends (Lattice Jolt, Stwo, RISC Zero) plug in behind `Prover`; the
//! choice is made from the phase-0.5 spike measurements. `Verifier` must stay
//! `no_std`-portable because the wallet ships it on macOS, iOS and wasm.

pub mod block;
pub mod market;

use aether_state::Proof as StateProof;
use aether_types::{chunks_chain, BlockProof, ChunkIo, ChunkProof, Hash, ProofSystemId, TxEnvelope, MAX_PROOF_BYTES};

/// Everything a guest needs to re-execute a chunk and commit to its result.
#[derive(Clone, Debug)]
pub struct ChunkInput {
    pub height: u64,
    pub chunk: u16,
    pub of: u16,
    pub pre_root: Hash,
    pub tx_range: (u32, u32),
    pub txs: Vec<TxEnvelope>,
    /// Proofs for exactly the keys in this chunk's BAL slice.
    pub witness: Vec<StateProof>,
}

/// Public inputs a verifier checks a block proof against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofIo {
    pub height: u64,
    pub pre_state_root: Hash,
    pub post_state_root: Hash,
    pub bal_root: Hash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvingError {
    UnsupportedSystem(ProofSystemId),
    ChunksDoNotChain,
    OverBudget(usize),
    IoMismatch,
    Invalid(String),
}

pub trait Prover: Send + Sync {
    fn system(&self) -> ProofSystemId;
    fn prove_chunk(&self, input: ChunkInput) -> Result<ChunkProof, ProvingError>;
    fn aggregate(&self, chunks: &[ChunkProof], bal_root: Hash) -> Result<BlockProof, ProvingError>;
    /// Cycle estimate used by the proof scheduler (advisory only).
    fn estimate_cycles(&self, input: &ChunkInput) -> u64;
}

pub trait Verifier: Send + Sync {
    fn system(&self) -> ProofSystemId;
    /// Verify the cryptographic proof only; `check_block` wraps the shared checks.
    fn verify_proof(&self, proof: &BlockProof) -> Result<(), ProvingError>;
}

/// Checks every verifier must apply regardless of backend.
pub fn check_block<V: Verifier + ?Sized>(v: &V, proof: &BlockProof, expected: &ProofIo) -> Result<(), ProvingError> {
    if proof.system != v.system() {
        return Err(ProvingError::UnsupportedSystem(proof.system));
    }
    if !proof.within_budget() {
        return Err(ProvingError::OverBudget(proof.proof_bytes.len()));
    }
    let io = ProofIo { height: proof.height, pre_state_root: proof.pre_state_root, post_state_root: proof.post_state_root, bal_root: proof.bal_root };
    if io != *expected {
        return Err(ProvingError::IoMismatch);
    }
    v.verify_proof(proof)
}

/// Aggregation precondition shared by all backends.
pub fn check_chunks(chunks: &[ChunkProof]) -> Result<(), ProvingError> {
    let ios: Vec<ChunkIo> = chunks.iter().map(|c| c.io.clone()).collect();
    let complete = chunks.iter().enumerate().all(|(i, c)| c.chunk as usize == i && c.of as usize == chunks.len());
    if chunks.is_empty() || !complete || !chunks_chain(&ios) {
        return Err(ProvingError::ChunksDoNotChain);
    }
    Ok(())
}

pub const PROOF_BUDGET: usize = MAX_PROOF_BYTES;

#[cfg(test)]
mod tests {
    use super::*;
    use aether_types::{Bytes, GasVector, B256};

    const SYS: ProofSystemId = ProofSystemId { family: *b"test\0\0\0\0", version: 1 };
    struct AcceptAll;
    impl Verifier for AcceptAll {
        fn system(&self) -> ProofSystemId {
            SYS
        }
        fn verify_proof(&self, _: &BlockProof) -> Result<(), ProvingError> {
            Ok(())
        }
    }

    fn proof(len: usize) -> BlockProof {
        BlockProof {
            height: 1,
            pre_state_root: B256::repeat_byte(1),
            post_state_root: B256::repeat_byte(2),
            bal_root: B256::repeat_byte(3),
            system: SYS,
            proof_bytes: Bytes::from(vec![0u8; len]),
        }
    }

    fn io() -> ProofIo {
        ProofIo { height: 1, pre_state_root: B256::repeat_byte(1), post_state_root: B256::repeat_byte(2), bal_root: B256::repeat_byte(3) }
    }

    #[test]
    fn shared_checks_enforced_before_backend() {
        assert_eq!(check_block(&AcceptAll, &proof(10), &io()), Ok(()));
        assert_eq!(check_block(&AcceptAll, &proof(MAX_PROOF_BYTES + 1), &io()), Err(ProvingError::OverBudget(MAX_PROOF_BYTES + 1)));
        let mut wrong = io();
        wrong.post_state_root = B256::ZERO;
        assert_eq!(check_block(&AcceptAll, &proof(10), &wrong), Err(ProvingError::IoMismatch));
        let mut p = proof(10);
        p.system.version = 2;
        assert!(matches!(check_block(&AcceptAll, &p, &io()), Err(ProvingError::UnsupportedSystem(_))));
    }

    #[test]
    fn chunk_set_must_be_complete_and_chained() {
        let c = |i: u16, pre: u8, post: u8, a: u32, b: u32| ChunkProof {
            chunk: i,
            of: 2,
            io: ChunkIo { height: 1, pre_root: B256::repeat_byte(pre), post_root: B256::repeat_byte(post), tx_range: (a, b), gas: GasVector::default() },
            system: SYS,
            proof_bytes: Bytes::new(),
        };
        assert_eq!(check_chunks(&[c(0, 1, 2, 0, 5), c(1, 2, 3, 5, 9)]), Ok(()));
        assert!(check_chunks(&[c(1, 2, 3, 5, 9), c(0, 1, 2, 0, 5)]).is_err(), "order");
        assert!(check_chunks(&[c(0, 1, 2, 0, 5)]).is_err(), "missing chunk");
        assert!(check_chunks(&[]).is_err());
    }
}
