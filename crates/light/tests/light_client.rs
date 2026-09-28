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
        let anchor = VerifiedBlock { height: n, digest: String::new(), timestamp_ms: 0, parent_state_root: B256::ZERO, history_root: B256::from(mmr.root(&h)) };
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
