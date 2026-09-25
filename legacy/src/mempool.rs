use crate::crypto::ThresholdScheme;
use crate::types::{EncryptedTx, Transaction};
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct EncryptedMempool {
    queue: Arc<RwLock<VecDeque<EncryptedTx>>>,
}

impl EncryptedMempool {
    pub fn new() -> Self {
        EncryptedMempool {
            queue: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    pub fn submit(&self, enc_tx: EncryptedTx) {
        self.queue.write().push_back(enc_tx);
    }

    pub fn len(&self) -> usize {
        self.queue.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.read().is_empty()
    }

    /// Propose batch of transactions to consensus blindly (without knowing plaintext)
    pub fn drain_batch(&self, max_txs: usize) -> Vec<EncryptedTx> {
        let mut q = self.queue.write();
        let count = std::cmp::min(max_txs, q.len());
        let mut batch = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(tx) = q.pop_front() {
                batch.push(tx);
            }
        }
        batch
    }

    /// Decrypt ordered transactions using threshold shares
    pub fn decrypt_ordered_batch(
        ordered_enc_txs: &[EncryptedTx],
        shares: &[(i128, i128)],
    ) -> Result<Vec<Transaction>, &'static str> {
        let mut decrypted = Vec::with_capacity(ordered_enc_txs.len());
        for enc_tx in ordered_enc_txs {
            let tx = ThresholdScheme::decrypt_with_shares(shares, enc_tx)?;
            decrypted.push(tx);
        }
        Ok(decrypted)
    }
}
