//! Core data formats (docs/design/02-types.md).
//!
//! Every consensus-relevant structure has a canonical byte encoding
//! (`encode_canonical`) used for hashing and signing. JSON (serde) is only for
//! RPC and manifests.

#![no_std]
extern crate alloc;

mod bal;
mod block;
mod envelope;
mod manifest;
mod proof;

pub use alloy_primitives::{Address, Bytes, B256, U256};
pub use bal::{AccountAccess, BalBuilder, BalError, BlockAccessList, StorageKey};
pub use block::{Block, BlockBody, BlockHeader, CertKind, Certificate, DaRef, ValidatorId};
pub use envelope::{FeeVector, GasVector, SignerScheme, TxEnvelope, TxHeader, TxPayload};
pub use manifest::{Manifest, ManifestKind, Mirror};
pub use proof::{chunks_chain, BlockProof, ChunkIo, ChunkProof, ProofSystemId, MAX_PROOF_BYTES};

pub type Hash = B256;
pub type TxIndex = u32;
pub type TxHash = B256;

/// Canonical, deterministic byte encoding for hashing and signing.
pub trait Canonical {
    fn encode_canonical(&self, out: &mut alloc::vec::Vec<u8>);

    fn to_canonical_bytes(&self) -> alloc::vec::Vec<u8> {
        let mut out = alloc::vec::Vec::new();
        self.encode_canonical(&mut out);
        out
    }
}

pub(crate) fn put_u64(out: &mut alloc::vec::Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub(crate) fn put_bytes(out: &mut alloc::vec::Vec<u8>, b: &[u8]) {
    put_u64(out, b.len() as u64);
    out.extend_from_slice(b);
}
