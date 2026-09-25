//! Light-client checks against real artifacts captured from a 4-validator devnet.

use aether_light::{from_hex, verify_account, verify_finalized, LightError, ValidatorSet};
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
