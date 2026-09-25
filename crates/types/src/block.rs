use crate::{put_u64, BlockAccessList, Canonical, GasVector, Hash, TxEnvelope, TxHash};
use alloc::vec::Vec;
use alloy_primitives::B256;
use serde::{Deserialize, Serialize};

/// Hash of a validator's consensus public key.
pub type ValidatorId = B256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaRef {
    pub height: u64,
    pub namespace: [u8; 29],
    pub commitment: Hash,
}

/// Order-first header: consensus fixes txs + BAL + inclusion list for `height`;
/// `state_root_at` reports execution of the older block `exec_target`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub height: u64,
    pub parent: Hash,
    pub timestamp: u64,
    pub proposer: ValidatorId,
    pub tx_root: Hash,
    pub bal_root: Hash,
    pub inclusion_list_root: Hash,
    pub history_root: Hash,
    pub exec_target: u64,
    pub state_root_at: Hash,
    pub gas_used: GasVector,
    pub da_ref: Option<DaRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockBody {
    pub txs: Vec<TxEnvelope>,
    pub bal: BlockAccessList,
    pub inclusion_list: Vec<TxHash>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub body: BlockBody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertKind {
    Notarized,
    Finalized,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Certificate {
    pub height: u64,
    pub block_hash: Hash,
    pub kind: CertKind,
    pub committee_epoch: u64,
    /// BLS12-381 threshold signature (48 bytes) or future scheme bytes.
    pub signature: alloy_primitives::Bytes,
}

impl Canonical for BlockHeader {
    fn encode_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"aether/header/v1");
        put_u64(out, self.height);
        out.extend_from_slice(self.parent.as_slice());
        put_u64(out, self.timestamp);
        out.extend_from_slice(self.proposer.as_slice());
        out.extend_from_slice(self.tx_root.as_slice());
        out.extend_from_slice(self.bal_root.as_slice());
        out.extend_from_slice(self.inclusion_list_root.as_slice());
        out.extend_from_slice(self.history_root.as_slice());
        put_u64(out, self.exec_target);
        out.extend_from_slice(self.state_root_at.as_slice());
        self.gas_used.encode_canonical(out);
        match &self.da_ref {
            None => out.push(0),
            Some(d) => {
                out.push(1);
                put_u64(out, d.height);
                out.extend_from_slice(&d.namespace);
                out.extend_from_slice(d.commitment.as_slice());
            }
        }
    }
}

impl Canonical for Certificate {
    fn encode_canonical(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"aether/cert/v1");
        put_u64(out, self.height);
        out.extend_from_slice(self.block_hash.as_slice());
        out.push(match self.kind {
            CertKind::Notarized => 0,
            CertKind::Finalized => 1,
        });
        put_u64(out, self.committee_epoch);
    }
}

impl BlockHeader {
    /// Execution lag: height minus the executed height.
    pub fn exec_lag(&self) -> Option<u64> {
        self.height.checked_sub(self.exec_target)
    }
}
