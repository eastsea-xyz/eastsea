//! FOCIL-style inclusion lists (docs/design/07-consensus.md). A rotating
//! committee publishes tx hashes the next block must include unless the block
//! is full; no cryptography needed, first step of censorship resistance.

use crate::OrderingPolicy;
use aether_types::{TxEnvelope, TxHash};
use std::collections::BTreeSet;

/// Inclusion-list txs first (list order), then the rest by exec max fee
/// (desc), ties by hash. Deterministic in its inputs.
pub struct FifoWithInclusion;

impl OrderingPolicy for FifoWithInclusion {
    fn order(&self, mut txs: Vec<(TxHash, TxEnvelope)>, inclusion_list: &[TxHash]) -> Vec<(TxHash, TxEnvelope)> {
        let rank = |h: &TxHash| inclusion_list.iter().position(|x| x == h);
        txs.sort_by(|(ha, a), (hb, b)| match (rank(ha), rank(hb)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => core::cmp::Ordering::Less,
            (None, Some(_)) => core::cmp::Ordering::Greater,
            (None, None) => b.header.tip.min(b.header.max_fee.exec).cmp(&a.header.tip.min(a.header.max_fee.exec)).then(ha.cmp(hb)),
        });
        txs.dedup_by(|a, b| a.0 == b.0);
        txs
    }
}

/// Inclusion-list entries absent from the block. Empty when the block is full
/// (the list is then advisory) or every listed tx is present.
pub fn missing_inclusions(block_txs: &[TxHash], inclusion_list: &[TxHash], block_full: bool) -> Vec<TxHash> {
    if block_full {
        return Vec::new();
    }
    let present: BTreeSet<&TxHash> = block_txs.iter().collect();
    let mut missing: Vec<TxHash> = inclusion_list.iter().filter(|h| !present.contains(h)).copied().collect();
    missing.sort();
    missing.dedup();
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_types::{Address, Bytes, FeeVector, GasVector, SignerScheme, TxHeader, TxPayload, B256};

    fn tx(fee: u128) -> TxEnvelope {
        TxEnvelope {
            header: TxHeader {
                chain_id: 1,
                sender: Address::ZERO,
                nonce: 0,
                gas: GasVector::default(),
                max_fee: FeeVector { exec: fee, ..Default::default() },
                tip: fee,
                payload_commitment: B256::ZERO,
                scheme: SignerScheme::P256,
            },
            payload: TxPayload::Plain(Bytes::new()),
            signature: Bytes::new(),
        }
    }

    #[test]
    fn inclusion_first_then_fee() {
        let (a, b, c) = (B256::repeat_byte(1), B256::repeat_byte(2), B256::repeat_byte(3));
        let out = FifoWithInclusion.order(vec![(a, tx(10)), (b, tx(99)), (c, tx(1))], &[c]);
        assert_eq!(out.iter().map(|x| x.0).collect::<Vec<_>>(), vec![c, b, a]);
    }

    #[test]
    fn missing_detection() {
        let (a, b) = (B256::repeat_byte(1), B256::repeat_byte(2));
        assert_eq!(missing_inclusions(&[a], &[a, b], false), vec![b]);
        assert!(missing_inclusions(&[a], &[a, b], true).is_empty());
        assert!(missing_inclusions(&[a, b], &[b, a, b], false).is_empty());
    }
}
