//! Voting-node registrations in blocks (docs/design/22-gas-pool.md 2층):
//! signing on the Mac, checking in validators — beacon answers' pattern. A
//! registration rides the block's payload as a fee-free item instead of the
//! registry contract's paid `register`, so a new Mac with a zero balance
//! joins even while the base fee is above 0.
//!
//! Every check is a pure signature or chain-state check (deterministic, the
//! Apple side stays at the registrar): the registrar's attestation is the same
//! one the contract path takes, and the operator wallet's domain-separated
//! relay signature — over the chain, the registry, the whole registered
//! content, a one-shot nonce and an expiry — means nobody relays another
//! wallet's attestation. A block with an invalid item is invalid; proposers
//! include only items they checked against the same parent state.

use aether_execution::registry;
use aether_execution::WorldState;
use aether_light::block::NodeRegistration;
use aether_types::{SignerScheme, TxHash};

/// The item's id, what its pseudo-receipt and pending-status lookups use
/// (keccak of the canonical serialization, like a tx hash).
pub fn id(r: &NodeRegistration) -> TxHash {
    alloy_primitives::keccak256(serde_json::to_vec(r).expect("registration serializes"))
}

/// Check `r` for the block at `height` built on `state`: pure signature and
/// state checks, no writes. Deterministic — the same item decides the same way
/// on every node.
pub fn verify(state: &WorldState, chain_id: u64, height: u64, r: &NodeRegistration) -> Result<(), String> {
    if !aether_rewards::enabled(state) {
        return Err("no free registration lane on this network".into());
    }
    let (att, sig) = (r.attestation.as_ref(), r.signature.as_ref());
    if att.len() != 64 || sig.len() != 64 || r.operator_key.len() != 33 {
        return Err("registration fields are not the fixed size".into());
    }
    if height > r.expiry {
        return Err(format!("registration expired at height {}", r.expiry));
    }
    let wallet = aether_crypto::PublicKey { scheme: SignerScheme::P256, bytes: r.operator_key.to_vec() };
    if aether_crypto::address_of(&wallet).map_err(|e| format!("operator key: {e:?}"))? != r.operator {
        return Err("the operator key does not derive the operator address".into());
    }
    let relay = registry::relay_message(chain_id, r.operator, &r.validator_key.0, &r.node_id.0, r.beaconer, att, r.nonce, r.expiry);
    aether_crypto::verify(&wallet, &relay, sig).map_err(|e| format!("not signed by the operator wallet: {e:?}"))?;
    // The registrar's key lives in the registry (slots 0 and 1): the same key
    // the contract path checks attestation with, so the two paths accept the
    // same attestations.
    let x = state.storage(&registry::REGISTRY, aether_types::U256::ZERO).to_be_bytes::<32>();
    let y = state.storage(&registry::REGISTRY, aether_types::U256::from(1u8)).to_be_bytes::<32>();
    let registrar = aether_crypto::PublicKey { scheme: SignerScheme::P256, bytes: [&[4u8][..], &x, &y].concat() };
    let attested = registry::attestation_message(chain_id, r.operator, r.validator_key.0, r.node_id.0, r.beaconer);
    aether_crypto::verify(&registrar, &attested, att).map_err(|e| format!("the registrar's attestation does not verify: {e:?}"))?;
    if registry::index_of(state, &r.validator_key.0) != 0 {
        return Err("this voting key is already registered".into());
    }
    if registry::lane_nonce(state, &r.operator) != r.nonce {
        return Err(format!("registration nonce {} is not the next one", r.nonce));
    }
    Ok(())
}

/// Check and record a block's registrations (a system write, like beacon
/// answers): the per-block bound first, then each item in order on the state
/// the ones before it already changed (a block may register two Macs of one
/// operator only with consecutive nonces, in payload order).
pub fn apply(state: &mut WorldState, chain_id: u64, height: u64, items: &[NodeRegistration]) -> Result<(), String> {
    if items.len() > registry::MAX_FREE_PER_BLOCK {
        return Err(format!("more than {} free registrations", registry::MAX_FREE_PER_BLOCK));
    }
    for r in items {
        verify(state, chain_id, height, r)?;
        registry::register_system(state, height, r.operator, r.validator_key.0, r.node_id.0, r.beaconer)
            .map_err(|e| format!("registering {}'s voting key: {e}", r.operator.to_checksum(None)))?;
        registry::bump_lane_nonce(state, &r.operator);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{P256Signer, Signer as _};
    use aether_execution::registry::Params;
    use alloy_primitives::keccak256;

    const CHAIN: u64 = 9;
    const EB: u64 = 100;

    fn registrar() -> P256Signer {
        P256Signer::from_seed(&[7; 32]).unwrap()
    }

    /// A rewards-enabled state with the registry predeployed, at genesis.
    fn state() -> WorldState {
        let mut s = WorldState::default();
        let xy = aether_crypto::p256_xy(&registrar().public_key().bytes).unwrap();
        registry::predeploy(&mut s, xy, Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
        aether_rewards::enable(&mut s);
        s
    }

    /// A registration `op` signed for (relayed by) itself, nonce 0, height 10.
    fn item(op: &P256Signer, key: [u8; 32]) -> NodeRegistration {
        item_at(op, key, 0, 10)
    }

    fn item_at(op: &P256Signer, key: [u8; 32], nonce: u64, expiry: u64) -> NodeRegistration {
        let operator = aether_crypto::address_of(&op.public_key()).unwrap();
        let node = keccak256(key).0;
        let att = registrar()
            .sign(&registry::attestation_message(CHAIN, operator, key, node, operator))
            .unwrap();
        let msg = registry::relay_message(CHAIN, operator, &key, &node, operator, &att, nonce, expiry);
        NodeRegistration {
            operator,
            validator_key: key.into(),
            node_id: node.into(),
            beaconer: operator,
            attestation: att.into(),
            signature: op.sign(&msg).unwrap().into(),
            operator_key: op.public_key().bytes.clone().into(),
            nonce,
            expiry,
        }
    }

    fn wallet(seed: u8) -> P256Signer {
        P256Signer::from_seed(&[seed; 32]).unwrap()
    }

    #[test]
    fn a_signed_item_registers_and_the_words_match_the_contract_path() {
        let mut s = state();
        let op = wallet(1);
        let r = item_at(&op, [3; 32], 0, 99);
        apply(&mut s, CHAIN, 10, &[r.clone()]).unwrap();
        let c = &registry::candidates(&s)[0];
        assert_eq!(c.operator, r.operator);
        assert_eq!(c.validator_key, r.validator_key.0);
        assert_eq!(c.node_id, r.node_id.0);
        assert_eq!(c.beaconer, r.beaconer);
        assert_eq!((c.registered_epoch, c.streak), (0, 1));
        assert_eq!(registry::index_of(&s, &r.validator_key.0), 1);
        assert_eq!(registry::lane_nonce(&s, &r.operator), 1);
        // Consumed: the same item (or its nonce) cannot register again.
        assert!(apply(&mut s, CHAIN, 11, &[r.clone()]).unwrap_err().contains("already registered"));
        let next = item_at(&op, [4; 32], 1, 20);
        apply(&mut s, CHAIN, 11, &[next]).unwrap();
        assert_eq!(registry::candidates(&s).len(), 2);
    }

    #[test]
    fn only_the_operator_wallet_may_relay_an_attestation() {
        let s = state();
        let op = wallet(1);
        let mut r = item(&op, [3; 32]);
        // A stranger's signature over the relay message: refused.
        r.signature = wallet(2).sign(&registry::relay_message(CHAIN, r.operator, &r.validator_key.0, &r.node_id.0, r.beaconer, &r.attestation, r.nonce, r.expiry)).unwrap().into();
        assert!(verify(&s, CHAIN, 10, &r).unwrap_err().contains("operator wallet"));
        // A key that does not derive the operator address: refused.
        let mut forged = item(&wallet(2), [3; 32]);
        forged.operator = r.operator;
        assert!(verify(&s, CHAIN, 10, &forged).unwrap_err().contains("derive"));
        // Another chain: refused (domain separation).
        let mut other = item(&op, [3; 32]);
        let att = registrar().sign(&registry::attestation_message(CHAIN + 1, other.operator, [3; 32], other.node_id.0, other.beaconer)).unwrap();
        other.attestation = att.clone().into();
        other.signature = op.sign(&registry::relay_message(CHAIN + 1, other.operator, &[3; 32], &other.node_id.0, other.beaconer, &att, 0, 10)).unwrap().into();
        assert!(verify(&s, CHAIN, 10, &other).is_err());
    }

    #[test]
    fn the_registrars_attestation_is_checked_like_the_contract_path() {
        let s = state();
        let op = wallet(1);
        // Attested by someone else (not the registrar key in slots 0, 1).
        let mut stranger = item(&op, [3; 32]);
        let operator = stranger.operator;
        let bad = wallet(9).sign(&registry::attestation_message(CHAIN, operator, [3; 32], stranger.node_id.0, stranger.beaconer)).unwrap();
        stranger.attestation = bad.clone().into();
        stranger.signature = op.sign(&registry::relay_message(CHAIN, operator, &[3; 32], &stranger.node_id.0, stranger.beaconer, &bad, 0, 10)).unwrap().into();
        assert!(verify(&s, CHAIN, 10, &stranger).unwrap_err().contains("attestation"));
        // Wrong nonce (already spent or not yet due): refused.
        let mut nonce = item(&op, [3; 32]);
        nonce.nonce = 1;
        let att = nonce.attestation.clone();
        nonce.signature = op.sign(&registry::relay_message(CHAIN, operator, &[3; 32], &nonce.node_id.0, nonce.beaconer, &att, 1, 10)).unwrap().into();
        assert!(verify(&s, CHAIN, 10, &nonce).unwrap_err().contains("nonce"));
        // Expired: refused (the expiry is signed, so it cannot be stretched).
        let old = item_at(&op, [3; 32], 0, 9);
        assert!(verify(&s, CHAIN, 10, &old).unwrap_err().contains("expired"));
        let edge = item_at(&op, [3; 32], 0, 10);
        assert!(verify(&s, CHAIN, 10, &edge).is_ok(), "valid in the block its expiry names");
    }

    #[test]
    fn no_lane_without_node_rewards_and_blocks_are_bounded() {
        let mut plain = WorldState::default();
        let xy = aether_crypto::p256_xy(&registrar().public_key().bytes).unwrap();
        registry::predeploy(&mut plain, xy, Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
        let op = wallet(1);
        assert!(verify(&plain, CHAIN, 10, &item(&op, [3; 32])).unwrap_err().contains("lane"));
        let mut s = state();
        let many: Vec<_> = (0..=registry::MAX_FREE_PER_BLOCK as u8).map(|i| item(&wallet(i + 10), [0x10 + i; 32])).collect();
        assert!(apply(&mut s, CHAIN, 10, &many).unwrap_err().contains("more than"));
    }
}
