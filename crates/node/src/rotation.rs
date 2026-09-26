//! Automatic voting-node rotation (docs/design/07-consensus.md, "open voting
//! nodes"). No person decides: at the first block of every registry epoch, each
//! validator computes the next voting set from the registry state the block
//! builds on. If it differs from the running set, the running set refuses to
//! build past the boundary, so every validator stops at the same height; the
//! supervisor (`aether run`) then reshares the committee key to the new set,
//! which continues the same chain under the same identity.

use aether_consensus::committee::{select_open_committee, OpenCandidate, MIN_OPEN_COMMITTEE};
use aether_execution::registry::{self, Candidate};
use aether_execution::WorldState;
use std::collections::BTreeSet;

/// In a validator's data dir: the registry epoch whose rotation it gave up.
pub const DEFERRED_FILE: &str = "rotation-deferred";

/// In a voting node's data dir: the block (and finalization) it verified last
/// as a follower, which it starts from when it has no validator history.
pub const ANCHOR_FILE: &str = "anchor.json";

/// Largest voting set (the DKG cost grows with its square).
pub const MAX_VOTING_NODES: usize = 32;

/// The running voting set and whether this registry epoch's rotation was
/// given up (a failed reshare: the running set keeps the chain going).
#[derive(Clone, Debug, Default)]
pub struct Committee {
    /// Ed25519 voting keys; empty on followers (no gate).
    pub keys: BTreeSet<[u8; 32]>,
    /// Registry epoch whose rotation was deferred.
    pub deferred: Option<u64>,
}

/// The next voting set, if block `height` (built on `parent`) starts a registry
/// epoch and the selection from `parent` differs from the running set.
pub fn due(parent: &WorldState, height: u64, running: &Committee) -> Option<Vec<Candidate>> {
    if running.keys.is_empty() || height == 0 {
        return None;
    }
    let every = registry::epoch_blocks(parent);
    if !height.is_multiple_of(every) || running.deferred == Some(height / every) {
        return None;
    }
    let all = registry::candidates(parent);
    if all.len() < MIN_OPEN_COMMITTEE {
        return None;
    }
    let open: Vec<OpenCandidate> = all
        .iter()
        .map(|c| OpenCandidate {
            index: c.index,
            operator: c.operator.into_array(),
            validator_key: c.validator_key,
            registered_epoch: c.registered_epoch,
            last_epoch: c.last_epoch,
            streak: c.streak,
        })
        .collect();
    let picked = select_open_committee(&open, height / every - 1, MAX_VOTING_NODES);
    // Below four voting nodes BFT tolerates no fault: keep the running set.
    if picked.len() < MIN_OPEN_COMMITTEE {
        return None;
    }
    let next: BTreeSet<[u8; 32]> = picked.iter().map(|c| c.validator_key).collect();
    if next == running.keys {
        return None;
    }
    Some(picked.iter().map(|p| all[p.index as usize].clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{address_of, P256Signer, Signer};
    use aether_execution::registry::{attestation_message, encode_beacon, encode_register, REGISTRY};
    use aether_execution::{execute_block, sign_call, BlockContext, EvmCall};
    use aether_types::{Address, GasVector, U256};

    const E: u64 = 10;

    fn seed(b: u8) -> [u8; 32] {
        let mut s = [0u8; 32];
        s[0] = 0x51;
        s[31] = b;
        s
    }

    fn run(state: &WorldState, from: &P256Signer, nonce: u64, input: aether_types::Bytes, block: u64) -> WorldState {
        let ctx = BlockContext {
            chain_id: 7,
            number: block,
            timestamp: block,
            beneficiary: Address::ZERO,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            fees: None,
        };
        let call = EvmCall { to: Some(REGISTRY), value: U256::ZERO, input, gas_limit: 400_000, delegate: None };
        let out = execute_block(state, &ctx, &[sign_call(from, 7, nonce, 1, &call).unwrap()]).unwrap();
        assert!(out.receipts[0].success);
        out.state
    }

    /// `n` candidates from `n` operators, each beaconing in epoch `e`.
    fn registry_with(n: u8, e: u64) -> WorldState {
        let registrar = P256Signer::from_seed(&seed(0)).unwrap();
        let mut s = WorldState::default();
        registry::predeploy(&mut s, aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap(), E).unwrap();
        for i in 1..=n {
            let op = P256Signer::from_seed(&seed(i)).unwrap();
            let a = address_of(&op.public_key()).unwrap();
            s.set_balance(a, U256::from(10u128.pow(20))).unwrap();
            let sig = registrar.sign(&attestation_message(7, a, [i; 32], [i; 32], a)).unwrap();
            s = run(&s, &op, 0, encode_register([i; 32], [i; 32], a, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()), 1);
            s = run(&s, &op, 1, encode_beacon([i; 32]), e * E + 1);
        }
        s
    }

    fn running(keys: &[u8]) -> Committee {
        Committee { keys: keys.iter().map(|k| [*k; 32]).collect(), deferred: None }
    }

    #[test]
    fn rotates_only_at_epoch_starts_to_a_different_live_set() {
        let s = registry_with(5, 3);
        let genesis_set = running(&[0xa1, 0xa2, 0xa3, 0xa4]);
        let next = due(&s, 4 * E, &genesis_set).expect("five live candidates replace the genesis set");
        assert_eq!(next.len(), 5);
        assert!(due(&s, 4 * E + 1, &genesis_set).is_none(), "not an epoch start");
        assert!(due(&s, 5 * E, &genesis_set).is_none(), "nobody beaconed in epoch 4");
        assert!(due(&s, 4 * E, &running(&[1, 2, 3, 4, 5])).is_none(), "already running");
        assert!(due(&s, 4 * E, &Committee::default()).is_none(), "followers never gate");
        assert!(due(&s, 4 * E, &Committee { deferred: Some(4), ..genesis_set }).is_none(), "deferred after a failed reshare");
    }

    #[test]
    fn too_few_candidates_keep_the_running_set() {
        let s = registry_with(3, 3);
        assert!(due(&s, 4 * E, &running(&[0xa1, 0xa2, 0xa3, 0xa4])).is_none());
    }
}
