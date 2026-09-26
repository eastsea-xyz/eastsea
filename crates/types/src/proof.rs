use crate::{put_u64, Canonical, GasVector, Hash};
use alloy_primitives::Bytes;
use serde::{Deserialize, Serialize};

/// Proof size budget (EF real-time proving target, docs/design/00 D17).
pub const MAX_PROOF_BYTES: usize = 300 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofSystemId {
    pub family: [u8; 8],
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkIo {
    pub height: u64,
    pub pre_root: Hash,
    pub post_root: Hash,
    pub tx_range: (u32, u32),
    pub gas: GasVector,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkProof {
    pub chunk: u16,
    pub of: u16,
    pub io: ChunkIo,
    pub system: ProofSystemId,
    pub proof_bytes: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockProof {
    pub height: u64,
    pub pre_state_root: Hash,
    pub post_state_root: Hash,
    pub bal_root: Hash,
    pub system: ProofSystemId,
    pub proof_bytes: Bytes,
}

impl BlockProof {
    pub fn within_budget(&self) -> bool {
        self.proof_bytes.len() <= MAX_PROOF_BYTES
    }
}

impl Canonical for ChunkIo {
    fn encode_canonical(&self, out: &mut alloc::vec::Vec<u8>) {
        out.extend_from_slice(b"aether/chunk-io/v1");
        put_u64(out, self.height);
        out.extend_from_slice(self.pre_root.as_slice());
        out.extend_from_slice(self.post_root.as_slice());
        out.extend_from_slice(&self.tx_range.0.to_be_bytes());
        out.extend_from_slice(&self.tx_range.1.to_be_bytes());
        self.gas.encode_canonical(out);
    }
}

/// Consecutive chunk proofs must chain state roots and tx ranges exactly.
pub fn chunks_chain(ios: &[ChunkIo]) -> bool {
    ios.windows(2).all(|w| w[0].height == w[1].height && w[0].post_root == w[1].pre_root && w[0].tx_range.1 == w[1].tx_range.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;

    fn io(pre: u8, post: u8, a: u32, b: u32) -> ChunkIo {
        ChunkIo { height: 5, pre_root: B256::repeat_byte(pre), post_root: B256::repeat_byte(post), tx_range: (a, b), gas: GasVector::default() }
    }

    #[test]
    fn chunk_chaining() {
        assert!(chunks_chain(&[io(1, 2, 0, 10), io(2, 3, 10, 20)]));
        assert!(!chunks_chain(&[io(1, 2, 0, 10), io(9, 3, 10, 20)]), "root gap");
        assert!(!chunks_chain(&[io(1, 2, 0, 10), io(2, 3, 11, 20)]), "tx gap");
    }
}
