//! Canonical receipt commitment and ordered Merkle proofs using the chain's
//! keyed BLAKE3 `hash_bytes` backend.
//!
//! Every field of an execution receipt is committed, including return data and
//! logs. Lengths and domains make each encoding unambiguous. An empty block
//! commits the zero root; its payload still carries `Some(ZERO)` on new chains.

use crate::Receipt;
use aether_hash::{Blake3, Hasher};
use aether_types::B256;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptProof {
    pub count: u32,
    /// Siblings from the receipt leaf upward; an odd last node repeats itself.
    pub siblings: Vec<B256>,
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Stable wire-independent encoding of every receipt field.
pub fn receipt_canonical_bytes(receipt: &Receipt) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"aether/receipt/v1");
    out.extend_from_slice(receipt.tx_hash.as_slice());
    out.push(u8::from(receipt.success));
    for gas in [receipt.gas_used, receipt.prove_gas, receipt.state_gas] {
        out.extend_from_slice(&gas.to_be_bytes());
    }
    out.extend_from_slice(&receipt.state_fee.to_be_bytes::<32>());
    match receipt.contract_address {
        Some(address) => {
            out.push(1);
            out.extend_from_slice(address.as_slice());
        }
        None => out.push(0),
    }
    out.extend_from_slice(&receipt.logs.to_be_bytes());
    put_bytes(&mut out, &receipt.output);
    out.extend_from_slice(&(receipt.events.len() as u64).to_be_bytes());
    for event in &receipt.events {
        out.extend_from_slice(event.address.as_slice());
        out.extend_from_slice(&(event.topics.len() as u64).to_be_bytes());
        for topic in &event.topics {
            out.extend_from_slice(topic.as_slice());
        }
        put_bytes(&mut out, &event.data);
    }
    out
}

fn leaf(index: usize, receipt: &Receipt) -> B256 {
    let bytes = receipt_canonical_bytes(receipt);
    let mut input = Vec::with_capacity(32 + bytes.len());
    input.extend_from_slice(b"aether/receipt-leaf/v1");
    input.extend_from_slice(&(index as u64).to_be_bytes());
    put_bytes(&mut input, &bytes);
    B256::from(Blake3.hash_bytes(&input))
}

fn parent(left: B256, right: B256) -> B256 {
    let mut input = Vec::with_capacity(88);
    input.extend_from_slice(b"aether/receipt-node/v1");
    input.extend_from_slice(left.as_slice());
    input.extend_from_slice(right.as_slice());
    B256::from(Blake3.hash_bytes(&input))
}

fn finish(count: usize, tree: B256) -> B256 {
    if count == 0 {
        return B256::ZERO;
    }
    let mut input = Vec::with_capacity(96);
    input.extend_from_slice(b"aether/receipts-root/v1");
    input.extend_from_slice(&(count as u64).to_be_bytes());
    input.extend_from_slice(tree.as_slice());
    B256::from(Blake3.hash_bytes(&input))
}

/// Root of receipts in exactly the order of the block's transactions.
pub fn receipt_root(receipts: &[Receipt]) -> B256 {
    let mut level: Vec<_> = receipts
        .iter()
        .enumerate()
        .map(|(i, r)| leaf(i, r))
        .collect();
    if level.is_empty() {
        return B256::ZERO;
    }
    let count = level.len();
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|pair| parent(pair[0], *pair.get(1).unwrap_or(&pair[0])))
            .collect();
    }
    finish(count, level[0])
}

/// Inclusion path for one receipt; returns `None` for an absent index.
pub fn receipt_proof(receipts: &[Receipt], index: usize) -> Option<ReceiptProof> {
    if index >= receipts.len() || receipts.len() > u32::MAX as usize {
        return None;
    }
    let mut level: Vec<_> = receipts
        .iter()
        .enumerate()
        .map(|(i, r)| leaf(i, r))
        .collect();
    let mut pos = index;
    let mut siblings = Vec::new();
    while level.len() > 1 {
        siblings.push(*level.get(pos ^ 1).unwrap_or(&level[pos]));
        level = level
            .chunks(2)
            .map(|pair| parent(pair[0], *pair.get(1).unwrap_or(&pair[0])))
            .collect();
        pos >>= 1;
    }
    Some(ReceiptProof {
        count: receipts.len() as u32,
        siblings,
    })
}

/// Verify the count, index, shape, sibling path, and root.
pub fn verify_receipt(root: B256, index: usize, receipt: &Receipt, proof: &ReceiptProof) -> bool {
    let mut width = proof.count as usize;
    if width == 0 || index >= width {
        return false;
    }
    let mut pos = index;
    let mut node = leaf(index, receipt);
    let mut path = proof.siblings.iter();
    while width > 1 {
        let Some(&sibling) = path.next() else {
            return false;
        };
        if (pos ^ 1) >= width && sibling != node {
            return false;
        }
        node = if pos & 1 == 0 {
            parent(node, sibling)
        } else {
            parent(sibling, node)
        };
        pos >>= 1;
        width = width.div_ceil(2);
    }
    path.next().is_none() && finish(proof.count as usize, node) == root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Event;
    use aether_types::{Address, Bytes, U256};

    fn receipt(n: u8) -> Receipt {
        Receipt {
            tx_hash: B256::repeat_byte(n),
            success: true,
            gas_used: 21_000,
            prove_gas: 7,
            state_gas: 2,
            state_fee: U256::from(3),
            contract_address: None,
            logs: 1,
            output: Bytes::from(vec![n]),
            events: vec![Event {
                address: Address::repeat_byte(n),
                topics: vec![B256::repeat_byte(n)],
                data: Bytes::from(vec![n, 1]),
            }],
        }
    }

    #[test]
    fn deterministic_ordered_root_and_all_fields() {
        let receipts = vec![receipt(1), receipt(2), receipt(3)];
        let root = receipt_root(&receipts);
        assert_eq!(root, receipt_root(&receipts));
        assert_ne!(root, receipt_root(&[receipt(2), receipt(1), receipt(3)]));
        assert_eq!(receipt_root(&[]), B256::ZERO);
        let mut altered = receipts[0].clone();
        let original = receipt_root(&[altered.clone()]);
        altered.success = false;
        assert_ne!(receipt_root(&[altered.clone()]), original);
        altered = receipts[0].clone();
        altered.gas_used += 1;
        assert_ne!(receipt_root(&[altered.clone()]), original);
        altered = receipts[0].clone();
        altered.prove_gas += 1;
        assert_ne!(receipt_root(&[altered.clone()]), original);
        altered = receipts[0].clone();
        altered.state_gas += 1;
        assert_ne!(receipt_root(&[altered.clone()]), original);
        altered = receipts[0].clone();
        altered.state_fee += U256::from(1);
        assert_ne!(receipt_root(&[altered.clone()]), original);
        altered = receipts[0].clone();
        altered.contract_address = Some(Address::repeat_byte(9));
        assert_ne!(receipt_root(&[altered.clone()]), original);
        altered = receipts[0].clone();
        altered.tx_hash = B256::repeat_byte(9);
        assert_ne!(receipt_root(&[altered]), original);
        altered = receipts[0].clone();
        altered.logs += 1;
        assert_ne!(receipt_root(&[altered]), original);
        altered = receipts[0].clone();
        altered.output = Bytes::from_static(b"changed return data");
        assert_ne!(receipt_root(&[altered]), original);
        altered = receipts[0].clone();
        altered.events[0].address = Address::repeat_byte(9);
        assert_ne!(receipt_root(&[altered.clone()]), original);
        altered = receipts[0].clone();
        altered.events[0].topics[0] = B256::repeat_byte(9);
        assert_ne!(receipt_root(&[altered]), original);
        altered = receipts[0].clone();
        altered.events[0].data = Bytes::from_static(b"changed log");
        assert_ne!(receipt_root(&[altered]), original);
        altered = receipts[0].clone();
        altered.events.clear();
        assert_ne!(receipt_root(&[altered]), original);
        for i in 0..receipts.len() {
            let proof = receipt_proof(&receipts, i).unwrap();
            assert!(verify_receipt(root, i, &receipts[i], &proof));
            let mut forged = receipts[i].clone();
            forged.events[0].data = Bytes::from_static(b"forged");
            assert!(!verify_receipt(root, i, &forged, &proof));
        }
    }

    #[test]
    fn wrong_shape_and_count_fail() {
        let receipts = vec![receipt(1), receipt(2), receipt(3)];
        let root = receipt_root(&receipts);
        let mut proof = receipt_proof(&receipts, 2).unwrap();
        proof.siblings[0] = B256::ZERO;
        assert!(!verify_receipt(root, 2, &receipts[2], &proof));
        let mut proof = receipt_proof(&receipts, 2).unwrap();
        proof.count = 2;
        assert!(!verify_receipt(root, 2, &receipts[2], &proof));
        let mut proof = receipt_proof(&receipts, 0).unwrap();
        proof.count = 4;
        assert!(!verify_receipt(root, 0, &receipts[0], &proof));
    }
}
