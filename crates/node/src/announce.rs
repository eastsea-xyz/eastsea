//! Wallet-server announcements (docs/design/08-network.md, capacity review
//! 2026-09-29; red-team 2026-09-29 §3). A follower Mac serving wallet reads
//! tells the validators where, signed by its voting key — the key of a
//! registered candidate — over the endpoint id it serves on. A validator
//! lists the announcement only if its own finalized registry state knows the
//! key, so an unregistered Mac cannot fill the list with endpoints that never
//! answer; a wallet that pins a server itself still uses it either way.

use crate::candidate::CandidateKeys;
use crate::chain::Chain;
use aether_execution::registry::{self, REGISTRY};
use aether_execution::WorldState;
use aether_net::{EndpointId, RegisteredCandidate, WALLET_SERVER_NAMESPACE};
use aether_types::U256;
use commonware_cryptography::Signer as _;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The `aether_announceWalletServer` params: `[validatorKey (hex),
/// signature (hex)]`, the voting key's signature over this endpoint's own id.
pub fn signed(keys: &CandidateKeys, endpoint: &EndpointId) -> Vec<String> {
    let sig = keys.keys.signer.sign(WALLET_SERVER_NAMESPACE, endpoint.as_bytes());
    vec![hex::encode(keys.validator_key()), hex::encode(commonware_codec::Encode::encode(&sig))]
}

/// The finalized registry indexed for announcements: candidate key → operator.
/// Candidates only ever append, so the count (one storage read) says whether
/// the index is current.
#[derive(Default)]
struct RegistryIndex {
    count: u64,
    by_key: HashMap<[u8; 32], [u8; 20]>,
}

impl RegistryIndex {
    fn of(state: &WorldState) -> Self {
        let candidates = registry::candidates(state);
        let count = candidates.len() as u64;
        RegistryIndex { count, by_key: candidates.into_iter().map(|c| (c.validator_key, c.operator.into())).collect() }
    }
}

/// The answer for an announcing key: the operator of the registered candidate
/// that key belongs to, from the finalized registry state (rebuilt when a
/// candidate registered since the last look).
fn lookup(index: &mut RegistryIndex, state: &WorldState, key: &[u8; 32]) -> Option<[u8; 20]> {
    let count = state.storage(&REGISTRY, U256::from(2u64)).to::<u64>();
    if count != index.count {
        *index = RegistryIndex::of(state);
    }
    index.by_key.get(key).copied()
}

/// The registry check a validator's public endpoint hands to `aether-net`:
/// an announcement is listed only when the finalized registry state has its
/// key, answered with the operator that registered it (what samples of the
/// list spread across).
pub fn checker(chain: Chain) -> RegisteredCandidate {
    let index = Arc::new(Mutex::new(RegistryIndex::default()));
    Arc::new(move |key: &[u8; 32]| {
        let g = chain.lock();
        let state = &g.finalized.state;
        lookup(&mut index.lock().expect("registry index"), state, key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::keccak256;

    /// A registered candidate written straight into the registry slots (the
    /// layout `registry::candidates` reads; the contract that writes it is
    /// tested in aether-execution).
    fn register(state: &mut WorldState, i: u64, key: [u8; 32], operator: [u8; 20]) {
        let base = U256::from_be_bytes(keccak256(U256::from(2u64).to_be_bytes::<32>()).0);
        let n = state.storage(&REGISTRY, U256::from(2u64)).to::<u64>().max(i);
        let slot = |k: u64| base + U256::from(5 * i + k);
        // An address is its 20 bytes right-aligned in the slot's word.
        let mut word = [0u8; 32];
        word[12..].copy_from_slice(&operator);
        state.set_storage(REGISTRY, slot(0), U256::from_be_bytes(word));
        state.set_storage(REGISTRY, slot(1), U256::from_be_bytes(key));
        state.set_storage(REGISTRY, U256::from(2u64), U256::from(n + 1));
    }

    fn state_with(key: [u8; 32], operator: [u8; 20]) -> WorldState {
        let mut s = WorldState::default();
        registry::predeploy(&mut s, ([1; 32], [2; 32]), registry::Params::default()).unwrap();
        register(&mut s, 0, key, operator);
        s
    }

    /// The signature the follower sends is the candidate key's over its own
    /// endpoint id (what `aether-net` verifies): no other key, endpoint or
    /// namespace verifies.
    #[test]
    fn a_signed_announcement_binds_the_key_to_the_endpoint() {
        use commonware_codec::{DecodeExt, Encode as _};
        use commonware_cryptography::{ed25519, Signer as _, Verifier as _};
        let keys = test_keys();
        let endpoint = aether_net::SecretKey::generate().public();
        let params = signed(&keys, &endpoint);
        let (key, sig) = (
            hex::decode(&params[0]).unwrap(),
            hex::decode(&params[1]).unwrap(),
        );
        let pk = ed25519::PublicKey::decode(key.as_slice()).unwrap();
        assert_eq!(key.len(), 32, "the voting key");
        assert!(pk.verify(WALLET_SERVER_NAMESPACE, endpoint.as_bytes(), &ed25519::Signature::decode(sig.as_slice()).unwrap()));

        // Not over another endpoint, and not under another namespace.
        let other = aether_net::SecretKey::generate().public();
        assert!(!pk.verify(WALLET_SERVER_NAMESPACE, other.as_bytes(), &ed25519::Signature::decode(sig.as_slice()).unwrap()));
        assert!(!pk.verify(b"aether-candidate-ownership", endpoint.as_bytes(), &ed25519::Signature::decode(sig.as_slice()).unwrap()));
    }

    /// A fresh candidate's key on a temp dir (never overwriting one that
    /// exists there).
    fn test_keys() -> CandidateKeys {
        let dir = std::env::temp_dir().join(format!("aether-announce-test-{}", std::process::id()));
        CandidateKeys::load_or_create(&dir).expect("candidate keys")
    }

    /// The lookup answers with the operator of a registered candidate only,
    /// and follows the registry as candidates register.
    #[test]
    fn the_registry_lookup_lists_registered_candidates_only() {
        let (mine, theirs) = ([0x11; 32], [0x22; 32]);
        let (my_op, their_op) = ([3u8; 20], [4u8; 20]);
        let mut state = state_with(mine, my_op);
        let mut index = RegistryIndex::default();
        assert_eq!(lookup(&mut index, &state, &mine), Some(my_op));
        assert_eq!(lookup(&mut index, &state, &theirs), None, "not a registered candidate");

        // A later registration is picked up (the count moved).
        register(&mut state, 1, theirs, their_op);
        assert_eq!(lookup(&mut index, &state, &theirs), Some(their_op));
        assert_eq!(lookup(&mut index, &state, &mine), Some(my_op));
    }

    /// Candidates registered by two operators both answer, with their own
    /// operator: what samples of the list spread across.
    #[test]
    fn the_lookup_keeps_operators_apart() {
        let mut state = state_with([0x11; 32], [1u8; 20]);
        register(&mut state, 1, [0x22; 32], [2u8; 20]);
        let mut index = RegistryIndex::of(&state);
        assert_eq!(lookup(&mut index, &state, &[0x11; 32]), Some([1u8; 20]));
        assert_eq!(lookup(&mut index, &state, &[0x22; 32]), Some([2u8; 20]));
    }
}
