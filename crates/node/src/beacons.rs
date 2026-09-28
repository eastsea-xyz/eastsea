//! Beacon answers (docs/design/15-node-rewards.md, A): signing on the Mac,
//! checking in validators. The rules (slots, windows, periods, what the state
//! keeps) are `aether_rewards::beacons`; this module adds the signatures.
//!
//! - An answer is signed by the voting key (ed25519) the registry binds to the
//!   Mac: the same key its registration proved it holds.
//! - A re-attestation is the registrar's P-256 signature, checked against the
//!   registrar key in the registry (slots 0 and 1), exactly the key the
//!   registry contract checks registrations with. The DeviceCheck call to Apple
//!   happens at the registrar before it signs, never in block validation.
//! - A block with an invalid answer is invalid: proposers only include answers
//!   they checked against the same parent state.

use aether_execution::registry::{self, Candidate};
use aether_execution::WorldState;
use aether_light::block::{BeaconAnswer, Reattestation};
use aether_rewards::beacons::{self, Due};
use aether_types::{SignerScheme, U256};
use commonware_codec::{DecodeExt as _, Encode as _};
use commonware_cryptography::{ed25519, Signer as _, Verifier as _};

/// Namespace of the voting key's beacon signatures.
pub const NAMESPACE: &[u8] = b"aether-beacon";

/// A checked answer, ready to record.
#[derive(Debug)]
pub struct Checked {
    pub candidate: Candidate,
    pub due: Due,
    pub attested: bool,
}

/// The voting key's answer to `due`.
pub fn sign(key: &ed25519::PrivateKey, chain_id: u64, index: u64, due: &Due, attest: Option<Reattestation>) -> BeaconAnswer {
    let msg = beacons::message(chain_id, due.epoch, due.slot, &due.hash);
    BeaconAnswer { index, slot: due.slot, signature: hex::encode(key.sign(NAMESPACE, &msg).encode()), attest }
}

/// The registrar's P-256 key as the registry holds it (SEC1, uncompressed).
fn registrar_key(state: &WorldState) -> aether_crypto::PublicKey {
    let x = state.storage(&registry::REGISTRY, U256::ZERO).to_be_bytes::<32>();
    let y = state.storage(&registry::REGISTRY, U256::from(1u8)).to_be_bytes::<32>();
    aether_crypto::PublicKey { scheme: SignerScheme::P256, bytes: [&[4u8][..], &x, &y].concat() }
}

/// Whether `r` is the registrar's re-attestation of `validator_key` for `period`.
pub fn verify_reattestation(state: &WorldState, chain_id: u64, validator_key: &[u8; 32], period: u64, r: &Reattestation) -> Result<(), String> {
    if r.period != period {
        return Err(format!("re-attestation for period {}, due {period}", r.period));
    }
    let sig = [hex::decode(&r.r).map_err(|_| "r is not hex")?, hex::decode(&r.s).map_err(|_| "s is not hex")?].concat();
    let msg = beacons::reattest_message(chain_id, validator_key, period);
    aether_crypto::verify(&registrar_key(state), &msg, &sig).map_err(|e| format!("re-attestation does not verify: {e:?}"))
}

/// Check `a` for the block at `height` built on `state`.
pub fn verify(state: &WorldState, chain_id: u64, height: u64, a: &BeaconAnswer) -> Result<Checked, String> {
    let candidate = beacons::candidate(state, a.index).ok_or_else(|| format!("no candidate {}", a.index))?;
    let due = beacons::check(state, height, &candidate, a.slot)?;
    let key = ed25519::PublicKey::decode(candidate.validator_key.as_slice()).map_err(|_| "registered key is not ed25519")?;
    let sig = hex::decode(&a.signature).ok().and_then(|b| ed25519::Signature::decode(b.as_slice()).ok()).ok_or("signature is not an ed25519 signature")?;
    if !key.verify(NAMESPACE, &beacons::message(chain_id, due.epoch, due.slot, &due.hash), &sig) {
        return Err(format!("answer of candidate {} is not signed by its voting key", a.index));
    }
    let attested = match &a.attest {
        Some(r) => {
            verify_reattestation(state, chain_id, &candidate.validator_key, due.period, r)?;
            true
        }
        None if due.needs_attestation => return Err(format!("candidate {} must re-attest for period {}", a.index, due.period)),
        None => false,
    };
    Ok(Checked { candidate, due, attested })
}

/// The state the block at `height` (child of the block hashed `parent_hash`)
/// checks answers against: the parent's, with that block's reward and slot
/// writes (a slot's hash lands in the block after it).
pub fn next_view(state: &WorldState, height: u64, parent_hash: [u8; 32]) -> WorldState {
    let mut view = state.clone();
    if beacons::touches(&view, height) {
        if aether_rewards::distributes(&view, height) {
            let _ = aether_rewards::distribute(&mut view, height);
        }
        beacons::on_block(&mut view, height, parent_hash);
    }
    view
}

/// Check and record a block's answers (a system write, like proof payouts).
pub fn apply(state: &mut WorldState, chain_id: u64, height: u64, answers: &[BeaconAnswer]) -> Result<(), String> {
    if answers.len() > beacons::MAX_ANSWERS_PER_BLOCK {
        return Err(format!("more than {} beacon answers", beacons::MAX_ANSWERS_PER_BLOCK));
    }
    for a in answers {
        // A second answer to the same slot fails `check` (already answered).
        let c = verify(state, chain_id, height, a)?;
        beacons::record(state, &c.candidate, &c.due, c.attested);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{P256Signer, Signer as _};
    use aether_execution::registry::Params;

    const CHAIN: u64 = 9;
    const EB: u64 = 100;

    fn registrar() -> P256Signer {
        P256Signer::from_seed(&[7; 32]).unwrap()
    }

    /// A node-rewards state with Mac 0 (voting key from seed 1) registered in epoch 0,
    /// run to the first slot of epoch 1 plus one block.
    fn setup() -> (WorldState, ed25519::PrivateKey, u64) {
        let mut s = WorldState::default();
        let xy = aether_crypto::p256_xy(&registrar().public_key().bytes).unwrap();
        registry::predeploy(&mut s, xy, Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
        aether_rewards::enable(&mut s);
        let key = ed25519::PrivateKey::from_seed(1);
        let vk: [u8; 32] = key.public_key().encode().as_ref().try_into().unwrap();
        let c = Candidate {
            index: 0,
            operator: aether_types::Address::repeat_byte(1),
            validator_key: vk,
            node_id: [0; 32],
            beaconer: aether_types::Address::repeat_byte(1),
            registered_epoch: 0,
            last_epoch: 0,
            streak: 1,
            missed: 0,
        };
        aether_rewards::put_candidate(&mut s, &c);
        let mut h = 1;
        loop {
            if aether_rewards::distributes(&s, h) {
                aether_rewards::distribute(&mut s, h).unwrap();
            }
            beacons::on_block(&mut s, h, [h as u8; 32]);
            if h > EB && beacons::slot_hash(&s, 0).is_some() {
                return (s, key, h);
            }
            h += 1;
        }
    }

    #[test]
    fn a_signed_answer_is_recorded_and_a_forged_one_is_refused() {
        let (mut s, key, h) = setup();
        let c = beacons::candidate(&s, 0).unwrap();
        let due = beacons::check(&s, h, &c, 0).unwrap();
        let good = sign(&key, CHAIN, 0, &due, None);
        // Signed by another key: refused, whatever it claims.
        let forged = sign(&ed25519::PrivateKey::from_seed(2), CHAIN, 0, &due, None);
        assert!(verify(&s, CHAIN, h, &forged).unwrap_err().contains("not signed"));
        // Another chain's answer does not replay here.
        assert!(verify(&s, CHAIN + 1, h, &good).is_err());
        // A block with the forged answer next to a good one is invalid as a whole.
        let mut t = s.clone();
        assert!(apply(&mut t, CHAIN, h, &[good.clone(), forged]).is_err());
        apply(&mut s, CHAIN, h, std::slice::from_ref(&good)).unwrap();
        assert_eq!(beacons::beacon(&s, 0).answered(1), 1);
        assert!(apply(&mut s, CHAIN, h, &[good]).is_err(), "answered once");
    }

    #[test]
    fn a_reattestation_must_be_the_registrars_for_this_key_and_period() {
        let (s, key, h) = setup();
        let c = beacons::candidate(&s, 0).unwrap();
        let due = beacons::check(&s, h, &c, 0).unwrap();
        let attest = |signer: &P256Signer, period: u64| {
            let sig = signer.sign(&beacons::reattest_message(CHAIN, &c.validator_key, period)).unwrap();
            Reattestation { period, r: hex::encode(&sig[..32]), s: hex::encode(&sig[32..64]) }
        };
        let ok = sign(&key, CHAIN, 0, &due, Some(attest(&registrar(), due.period)));
        assert!(verify(&s, CHAIN, h, &ok).unwrap().attested);
        let stranger = sign(&key, CHAIN, 0, &due, Some(attest(&P256Signer::from_seed(&[8; 32]).unwrap(), due.period)));
        assert!(verify(&s, CHAIN, h, &stranger).is_err(), "only the registrar's key");
        let stale = sign(&key, CHAIN, 0, &due, Some(attest(&registrar(), due.period + 1)));
        assert!(verify(&s, CHAIN, h, &stale).is_err(), "only this period");
    }
}
