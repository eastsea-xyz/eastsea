//! Data availability boundary (docs/design/03-traits.md, 08-network.md).
//! `LocalDa` is a complete in-process implementation used by tests and
//! single-machine nets; `CelestiaDa` (Lumina) and `EthBlobDa` implement the same trait.

use aether_hash::{Blake3, Hasher};
use aether_types::{DaRef, B256};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaError {
    TooLarge { size: usize, max: usize },
    NotFound,
    CommitmentMismatch,
}

pub trait DaLayer: Send + Sync {
    fn post(&self, blob: &[u8]) -> Result<DaRef, DaError>;
    fn get(&self, r: &DaRef) -> Result<Vec<u8>, DaError>;
    fn max_blob(&self) -> usize;
}

/// In-process DA: content-addressed by BLAKE3, height = insertion order.
pub struct LocalDa {
    namespace: [u8; 29],
    max_blob: usize,
    blobs: Mutex<(u64, HashMap<B256, Vec<u8>>)>,
}

impl LocalDa {
    pub fn new(namespace: [u8; 29], max_blob: usize) -> Self {
        LocalDa { namespace, max_blob, blobs: Mutex::new((0, HashMap::new())) }
    }
}

impl DaLayer for LocalDa {
    fn post(&self, blob: &[u8]) -> Result<DaRef, DaError> {
        if blob.len() > self.max_blob {
            return Err(DaError::TooLarge { size: blob.len(), max: self.max_blob });
        }
        let commitment = B256::from(Blake3.hash_bytes(blob));
        let mut g = self.blobs.lock().expect("da lock");
        g.0 += 1;
        let height = g.0;
        g.1.insert(commitment, blob.to_vec());
        Ok(DaRef { height, namespace: self.namespace, commitment })
    }

    fn get(&self, r: &DaRef) -> Result<Vec<u8>, DaError> {
        let g = self.blobs.lock().expect("da lock");
        let blob = g.1.get(&r.commitment).ok_or(DaError::NotFound)?;
        // Re-check content addressing on every read.
        if B256::from(Blake3.hash_bytes(blob)) != r.commitment {
            return Err(DaError::CommitmentMismatch);
        }
        Ok(blob.clone())
    }

    fn max_blob(&self) -> usize {
        self.max_blob
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_get_round_trip_and_limits() {
        let da = LocalDa::new([1; 29], 8);
        let r = da.post(b"block").unwrap();
        assert_eq!(da.get(&r).unwrap(), b"block");
        assert_eq!(da.post(b"too large!"), Err(DaError::TooLarge { size: 10, max: 8 }));
        let mut bogus = r.clone();
        bogus.commitment = B256::ZERO;
        assert_eq!(da.get(&bogus), Err(DaError::NotFound));
        assert_eq!(da.post(b"x").unwrap().height, 2);
    }
}
