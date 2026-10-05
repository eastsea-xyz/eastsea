//! Light-client checks against real artifacts captured from a 4-validator devnet.

use aether_light::{from_hex, verify_account, verify_finalized, verify_finalized_chain, LightError, ValidatorSet, MAX_LINKS};
use aether_state::Proof;
use aether_types::{Address, B256, U256};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/devnet4.json")).unwrap()
}

fn bytes(v: &Value, k: &str) -> Vec<u8> {
    from_hex(v[k].as_str().unwrap()).unwrap()
}

fn proof(v: &Value, k: &str) -> Proof {
    serde_json::from_value(v[k].clone()).unwrap()
}

#[test]
fn genuine_certificate_and_proof_verify() {
    let f = fixture();
    let set = ValidatorSet::devnet(4);
    let anchor = verify_finalized(&set, &bytes(&f, "anchor_block"), &bytes(&f, "anchor_finalization")).unwrap();
    assert_eq!(anchor.height, f["height"].as_u64().unwrap() + 1);
    let root: B256 = serde_json::from_value(f["state_root"].clone()).unwrap();
    assert_eq!(anchor.parent_state_root, root, "child header commits the reported root");
    let addr: Address = serde_json::from_value(f["address"].clone()).unwrap();
    let acct = verify_account(&anchor, &addr, &proof(&f, "proof")).unwrap().unwrap();
    let balance: U256 = serde_json::from_value(f["balance"].clone()).unwrap();
    assert_eq!(U256::from(acct.balance), balance);
}

#[test]
fn wrong_validator_set_rejects() {
    let f = fixture();
    for set in [ValidatorSet::devnet(3), ValidatorSet::devnet(5)] {
        assert!(verify_finalized(&set, &bytes(&f, "anchor_block"), &bytes(&f, "anchor_finalization")).is_err());
    }
}

#[test]
fn tampered_certificate_rejects() {
    let f = fixture();
    let set = ValidatorSet::devnet(4);
    let fin = bytes(&f, "anchor_finalization");
    // Flip one bit in each of several positions (proposal and signatures).
    for pos in [fin.len() / 4, fin.len() / 2, fin.len() - 1] {
        let mut t = fin.clone();
        t[pos] ^= 0x01;
        assert!(verify_finalized(&set, &bytes(&f, "anchor_block"), &t).is_err(), "bit flip at {pos} accepted");
    }
}

#[test]
fn certificate_for_another_block_rejects() {
    let f = fixture();
    let set = ValidatorSet::devnet(4);
    let r = verify_finalized(&set, &bytes(&f, "next_block"), &bytes(&f, "anchor_finalization"));
    assert_eq!(r, Err(LightError::CertificateForDifferentBlock));
}

#[test]
fn tampered_block_rejects() {
    let f = fixture();
    let set = ValidatorSet::devnet(4);
    let mut b = bytes(&f, "anchor_block");
    let n = b.len();
    b[n - 3] ^= 0x01; // inside the payload: changes the digest
    assert!(verify_finalized(&set, &b, &bytes(&f, "anchor_finalization")).is_err());
}

#[test]
fn proof_for_another_address_rejects() {
    let f = fixture();
    let set = ValidatorSet::devnet(4);
    let anchor = verify_finalized(&set, &bytes(&f, "anchor_block"), &bytes(&f, "anchor_finalization")).unwrap();
    let addr: Address = serde_json::from_value(f["address"].clone()).unwrap();
    // A valid proof, but for someone else's account.
    assert_eq!(verify_account(&anchor, &addr, &proof(&f, "other_proof")), Err(LightError::WrongKey));
}

#[test]
fn proof_under_uncommitted_root_rejects() {
    let f = fixture();
    let set = ValidatorSet::devnet(4);
    // Anchor to the *next* block: its parent_state_root is a later root, so an
    // honest proof for the earlier state does not verify against it unless nothing changed.
    let later = verify_finalized(&set, &bytes(&f, "next_block"), &bytes(&f, "next_finalization")).unwrap();
    let addr: Address = serde_json::from_value(f["address"].clone()).unwrap();
    let r = verify_account(&later, &addr, &proof(&f, "proof"));
    let fixture_root: B256 = serde_json::from_value(f["state_root"].clone()).unwrap();
    if later.parent_state_root != fixture_root {
        assert!(matches!(r, Err(LightError::ProofInvalid(_))));
    }
}

/// A height without its own certificate is proven through its descendants.
#[test]
fn a_block_is_proven_through_the_blocks_built_on_it() {
    let f = fixture();
    let set = ValidatorSet::devnet(4);
    let (anchor, next) = (bytes(&f, "anchor_block"), bytes(&f, "next_block"));
    let vb = verify_finalized_chain(&set, &anchor, &bytes(&f, "next_finalization"), std::slice::from_ref(&next)).unwrap();
    assert_eq!(vb, verify_finalized(&set, &anchor, &bytes(&f, "anchor_finalization")).unwrap());
    // The certificate must be for the last link, and every link must build on the one before.
    assert!(verify_finalized_chain(&set, &anchor, &bytes(&f, "anchor_finalization"), std::slice::from_ref(&next)).is_err());
    assert_eq!(verify_finalized_chain(&set, &next, &bytes(&f, "next_finalization"), std::slice::from_ref(&next)), Err(LightError::BrokenLink));
    let too_many = vec![next.clone(); MAX_LINKS + 1];
    assert!(verify_finalized_chain(&set, &anchor, &bytes(&f, "next_finalization"), &too_many).is_err());
}

/// Groups (13-roadmap.md): a group's certificates verify under its own
/// namespace and its blocks carry its id, so one group's chain never passes as
/// another's — checked against the genuine group-0 fixture certificate.
mod groups {
    use super::*;
    use aether_light::block::Block;
    use aether_light::consensus_namespace_of;
    use commonware_codec::{Decode, Encode};

    #[test]
    fn old_certified_block_round_trips_without_a_group_field() {
        let f = fixture();
        let original = bytes(&f, "anchor_block");
        let block = Block::decode_cfg(original.as_slice(), &Block::codec_config(aether_light::MAX_BLOCK_BYTES)).unwrap();
        assert_eq!(block.payload().unwrap().group, 0);
        assert_eq!(block.encode().to_vec(), original);
    }

    /// The same fixture block (`which`), re-encoded with `group` in its payload
    /// (a different block: the digest commits to the payload).
    fn regrouped(f: &Value, which: &str, group: u16) -> Vec<u8> {
        let b = Block::decode_cfg(bytes(f, which).as_slice(), &Block::codec_config(aether_light::MAX_BLOCK_BYTES)).unwrap();
        let mut p = b.payload().unwrap();
        p.group = group;
        Block::new(b.context.clone(), b.parent, b.height, b.timestamp, p.to_bytes()).encode().to_vec()
    }

    #[test]
    fn namespaces_separate_groups() {
        assert_eq!(consensus_namespace_of(0), aether_light::consensus_namespace(), "group 0 keeps today's namespace");
        for g in [1u16, 2, 300] {
            let ns = consensus_namespace_of(g);
            assert!(ns.starts_with(&consensus_namespace_of(0)), "group {g} extends the base namespace");
            assert!(ns.ends_with(&g.encode()), "group {g} ends with its id");
            assert_ne!(ns, consensus_namespace_of(0));
        }
        assert_ne!(consensus_namespace_of(1), consensus_namespace_of(2));
        assert_eq!(ValidatorSet::devnet(4).group(), 0);
        assert_eq!(ValidatorSet::devnet(4).with_group(3).group(), 3);
    }

    /// A genuine group-0 certificate does not verify under group 1's namespace:
    /// whatever block it is attached to, the signature itself is wrong there.
    #[test]
    fn another_groups_namespace_rejects_the_certificate() {
        let f = fixture();
        let set = ValidatorSet::devnet(4).with_group(1);
        let err = verify_finalized(&set, &bytes(&f, "anchor_block"), &bytes(&f, "anchor_finalization")).unwrap_err();
        assert_eq!(err, LightError::WrongGroup, "the payload check fires first");
        // Same test with the payload claiming group 1: the signature still does
        // not verify under group 1's namespace.
        let err = verify_finalized(&set, &regrouped(&f, "anchor_block", 1), &bytes(&f, "anchor_finalization")).unwrap_err();
        assert_eq!(err, LightError::CertificateInvalid, "the certificate is group 0's");
        // Group 0 keeps verifying its own chain.
        verify_finalized(&ValidatorSet::devnet(4), &bytes(&f, "anchor_block"), &bytes(&f, "anchor_finalization")).unwrap();
        verify_finalized(&ValidatorSet::devnet(4).with_group(0), &bytes(&f, "anchor_block"), &bytes(&f, "anchor_finalization")).unwrap();
    }

    /// A block of another group is refused before its certificate is even
    /// looked at, wherever it sits in the chain (also as a link).
    #[test]
    fn another_groups_block_rejects() {
        let f = fixture();
        let set = ValidatorSet::devnet(4);
        for group in [1u16, 2] {
            assert_eq!(verify_finalized(&set, &regrouped(&f, "anchor_block", group), &bytes(&f, "anchor_finalization")), Err(LightError::WrongGroup));
            // The genuine anchor, then a link claiming another group: the link
            // builds on the anchor (same parent, next height), so the group
            // check is what refuses it.
            let r = verify_finalized_chain(&set, &bytes(&f, "anchor_block"), &bytes(&f, "next_finalization"), &[regrouped(&f, "next_block", group)]);
            assert_eq!(r, Err(LightError::WrongGroup), "a foreign link is refused too");
        }
    }
}

/// History: a certified block's history root proves an old block's full
/// contents and whole eras (roots of 8192-block MMR subtrees).
mod history {
    use aether_hash::{Blake3, Digest as H32};
    use aether_light::block::{Block, Context, Payload, EPOCH};
    use aether_light::{verify_era_root, verify_old_block, LightError, VerifiedBlock};
    use aether_state::mmr::{self, prove, prove_by_eras, EraIndex, Mmr, ERA_BITS, ERA_LEN};
    use aether_types::B256;
    use commonware_codec::Encode;
    use commonware_consensus::types::{Height, Round, View};
    use commonware_cryptography::{ed25519, Digestible, Signer};

    fn digest(b: &Block) -> H32 {
        b.digest().as_ref().try_into().unwrap()
    }

    /// Blocks 0..n with the history roots nodes put in, and the anchor that certifies them all.
    fn chain(n: u64) -> (Vec<Block>, Vec<H32>, VerifiedBlock) {
        let h = Blake3;
        let genesis = Block::genesis(7, B256::repeat_byte(1));
        let (mut blocks, mut mmr) = (vec![genesis.clone()], Mmr::default().append(&h, 0, &digest(&genesis)));
        for height in 1..n {
            let prev = blocks.last().unwrap();
            let leader = ed25519::PrivateKey::from_seed(height % 4).public_key();
            let context = Context { round: Round::new(EPOCH, View::new(height)), leader, parent: (View::new(height - 1), prev.digest()) };
            let payload = Payload { version: 2, history_root: B256::from(mmr.root(&h)), ..Default::default() };
            let b = Block::new(context, prev.digest(), Height::new(height), height * 1000, payload.to_bytes());
            mmr = mmr.append(&h, height, &digest(&b));
            blocks.push(b);
        }
        let leaves = blocks.iter().map(|b| mmr::leaf(&h, b.height.get(), &digest(b))).collect();
        // What `verify_finalized` returns for block n (its certificate is checked elsewhere).
        let anchor = VerifiedBlock { height: n, digest: String::new(), timestamp_ms: 0, parent_state_root: B256::ZERO, receipts_root: None, history_root: B256::from(mmr.root(&h)) };
        (blocks, leaves, anchor)
    }

    #[test]
    fn an_old_block_is_verified_byte_for_byte() {
        let (blocks, leaves, anchor) = chain(40);
        let proof = prove(&Blake3, &leaves, 17).unwrap();
        let (v, payload) = verify_old_block(&anchor, &blocks[17].encode(), &proof).unwrap();
        assert_eq!(v.height, 17);
        assert_eq!(payload.history_root, v.history_root);
        // Another block, or the same block with one payload byte changed, does not pass.
        assert!(verify_old_block(&anchor, &blocks[18].encode(), &proof).is_err());
        let mut p = blocks[17].payload().unwrap();
        p.version = 3;
        let forged = Block::new(blocks[17].context.clone(), blocks[17].parent, blocks[17].height, blocks[17].timestamp, p.to_bytes());
        assert!(matches!(verify_old_block(&anchor, &forged.encode(), &proof), Err(LightError::ProofInvalid(_))));
        assert!(verify_old_block(&anchor, b"not a block", &proof).is_err());
    }

    #[test]
    fn a_whole_era_is_verified_by_its_root() {
        let (_, leaves, anchor) = chain(ERA_LEN + 5);
        let mut idx = EraIndex::default();
        for l in &leaves {
            idx.push(&Blake3, *l);
        }
        let proof = prove_by_eras(&Blake3, anchor.height, 0, ERA_BITS, &idx.eras, |_| Some(idx.open.clone())).unwrap();
        verify_era_root(&anchor, 0, &idx.eras[0], &proof).unwrap();
        assert!(verify_era_root(&anchor, 0, &[9; 32], &proof).is_err(), "another root");
        assert!(verify_era_root(&anchor, 1, &idx.eras[0], &proof).is_err(), "another era");
        // An anchor before the era's last block cannot vouch for it.
        let early = VerifiedBlock { height: ERA_LEN - 1, ..anchor.clone() };
        assert!(verify_era_root(&early, 0, &idx.eras[0], &proof).is_err());
    }
}

#[test]
fn receipt_proofs_require_the_committed_root_index_and_complete_receipt() {
    use aether_execution::{receipt::{receipt_proof, receipt_root}, Receipt};
    use aether_light::verify_receipt;
    let f = fixture();
    let mut anchor = verify_finalized(&ValidatorSet::devnet(4), &bytes(&f, "anchor_block"), &bytes(&f, "anchor_finalization")).unwrap();
    let receipt = Receipt { tx_hash: B256::repeat_byte(1), success: true, gas_used: 21_000,
        prove_gas: 3, state_gas: 0, state_fee: U256::ZERO, contract_address: None,
        logs: 0, output: aether_types::Bytes::new(), events: vec![] };
    let other = Receipt { tx_hash: B256::repeat_byte(2), ..receipt.clone() };
    let receipts = vec![receipt.clone(), other];
    let proof = receipt_proof(&receipts, 0).unwrap();
    assert_eq!(verify_receipt(&anchor, 0, &receipt, &proof), Err(LightError::RootNotCommitted));
    anchor.receipts_root = Some(receipt_root(&receipts));
    verify_receipt(&anchor, 0, &receipt, &proof).unwrap();
    assert!(verify_receipt(&anchor, 1, &receipt, &proof).is_err());
    let forged = Receipt { success: false, ..receipt.clone() };
    assert!(verify_receipt(&anchor, 0, &forged, &proof).is_err());
    let mut truncated = proof.clone();
    truncated.siblings.clear();
    assert!(verify_receipt(&anchor, 0, &receipt, &truncated).is_err());
    anchor.receipts_root = Some(B256::ZERO);
    assert!(verify_receipt(&anchor, 0, &receipt, &proof).is_err());
}

#[test]
fn adding_a_receipt_root_changes_the_certified_block_digest() {
    use aether_light::block::Block;
    use commonware_codec::{Decode, Encode};
    let f = fixture();
    let original = bytes(&f, "anchor_block");
    let block = Block::decode_cfg(original.as_slice(), &Block::codec_config(aether_light::MAX_BLOCK_BYTES)).unwrap();
    let mut payload = block.payload().unwrap();
    assert_eq!(payload.receipts_root, None);
    assert!(!std::str::from_utf8(&payload.to_bytes()).unwrap().contains("receipts_root"));
    assert_eq!(block.encode().as_ref(), original.as_slice(), "legacy block hash input must stay identical");
    payload.receipts_root = Some(B256::repeat_byte(7));
    let changed = Block::new(block.context.clone(), block.parent, block.height, block.timestamp, payload.to_bytes());
    assert_eq!(changed.payload().unwrap().receipts_root, payload.receipts_root);
    assert_eq!(verify_finalized(&ValidatorSet::devnet(4), &changed.encode(), &bytes(&f, "anchor_finalization")),
        Err(LightError::CertificateForDifferentBlock));
}

#[test]
fn receipt_root_is_extracted_only_after_its_block_is_certified() {
    use aether_execution::{receipt::{receipt_proof, receipt_root}, Receipt};
    use aether_light::{block::Block, consensus_namespace, devnet_threshold, devnet_validator_key, verify_receipt, Scheme};
    use commonware_codec::{Decode, Encode};
    use commonware_consensus::simplex::types::{Finalization, Finalize, Proposal};
    use commonware_cryptography::{Digestible, Signer};
    use commonware_parallel::Sequential;
    use commonware_utils::non_empty;
    let f = fixture();
    let original = bytes(&f, "anchor_block");
    let block = Block::decode_cfg(original.as_slice(), &Block::codec_config(aether_light::MAX_BLOCK_BYTES)).unwrap();
    let tx = aether_types::TxEnvelope {
        header: aether_types::TxHeader {
            chain_id: 7777, sender: Address::ZERO, nonce: 0,
            gas: Default::default(), max_fee: Default::default(), tip: 0,
            payload_commitment: B256::ZERO, scheme: aether_types::SignerScheme::P256, group: None,
        },
        payload: aether_types::TxPayload::Plain(aether_types::Bytes::new()),
        signature: aether_types::Bytes::new(),
    };
    let receipt = Receipt { tx_hash: aether_execution::tx_hash(&tx), success: true, gas_used: 21_000,
        prove_gas: 0, state_gas: 0, state_fee: U256::ZERO, contract_address: None,
        logs: 0, output: aether_types::Bytes::new(), events: vec![] };
    let receipts = vec![receipt.clone()];
    let mut payload = block.payload().unwrap();
    payload.txs = vec![tx];
    payload.receipts_root = Some(receipt_root(&receipts));
    let block = Block::new(block.context.clone(), block.parent, block.height, block.timestamp, payload.to_bytes());
    let (participants, polynomial, shares) = devnet_threshold(4);
    let signers: Vec<_> = (1..=3).map(|i| {
        let me = devnet_validator_key(i).public_key();
        let share = shares.iter().find(|(pk, _)| *pk == me).unwrap().1.clone();
        Scheme::signer(&consensus_namespace(), participants.clone(), polynomial.clone(), share).unwrap()
    }).collect();
    let votes: Vec<_> = signers.iter().map(|signer| Finalize::sign(signer,
        Proposal::new(block.context.round, block.context.parent.0, block.digest())).unwrap()).collect();
    let certificate = Finalization::from_owned_finalizes(&signers[0], non_empty![@votes.into_iter()], &Sequential).unwrap();
    let anchor = verify_finalized(&ValidatorSet::devnet(4), &block.encode(), &certificate.encode()).unwrap();
    assert_eq!(anchor.receipts_root, payload.receipts_root);
    verify_receipt(&anchor, 0, &receipt, &receipt_proof(&receipts, 0).unwrap()).unwrap();
    let expected: Value = serde_json::from_str(include_str!("fixtures/receipt-devnet4.json")).unwrap();
    assert_eq!(expected, serde_json::json!({
        "chain_id": 7777,
        "identity": ValidatorSet::devnet(4).identity_hex(),
        "height": anchor.height,
        "block": aether_light::to_hex(&block.encode()),
        "finalization": aether_light::to_hex(&certificate.encode()),
        "receipt": receipt,
        "proof": receipt_proof(&receipts, 0).unwrap(),
    }), "certified receipt fixture must remain byte-identical");
}
