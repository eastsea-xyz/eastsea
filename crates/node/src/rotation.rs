//! Which voting set comes next (docs/design/07-consensus.md, "open voting
//! nodes"). No person decides: at the first block of every registry epoch,
//! each node computes the proposed set from the registry state that block
//! builds on. The running set then reshares its key to it in the background and
//! hands over with a signed handoff (`handoff.rs`); the chain never stops for it.
//!
//! At most a third of the seats change per epoch, so a newcomer group (even one
//! owner with many Macs and wallets) cannot take the whole set at once: it has
//! to stay live for several epochs, while the running set keeps a quorum.

use aether_consensus::committee::{select_open_committee, OpenCandidate, MIN_OPEN_COMMITTEE};
use aether_execution::registry;
use aether_execution::WorldState;

/// In a validator's data dir: files a background reshare stages for a handoff.
pub const STAGED_THRESHOLD: &str = "threshold-next.json";
pub const STAGED_NETWORK: &str = "network-next.json";

/// In a voting node's data dir: the block (and finalization) it verified last
/// as a follower, which it starts from when it has no validator history.
pub const ANCHOR_FILE: &str = "anchor.json";

/// Largest voting set (the DKG cost grows with its square).
pub const MAX_VOTING_NODES: usize = 32;

/// The running voting set, in roster order: (ed25519 key hex, iroh node id).
/// Empty on followers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Committee {
    pub members: Vec<(String, String)>,
}

impl Committee {
    fn has(&self, key: &str) -> bool {
        self.members.iter().any(|(k, _)| k == key)
    }
}

/// The voting set proposed for registry epoch `epoch`, from `state` (what its
/// first block builds on), or None when the running set stays.
pub fn next_set(state: &WorldState, epoch: u64, running: &Committee) -> Option<Vec<(String, String)>> {
    if running.members.is_empty() || epoch == 0 {
        return None;
    }
    // Only candidates reachable at a valid iroh node id can hold a seat.
    let all: Vec<registry::Candidate> = registry::candidates(state).into_iter().filter(|c| aether_net::EndpointId::from_bytes(&c.node_id).is_ok()).collect();
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
    let picked = select_open_committee(&open, epoch - 1, MAX_VOTING_NODES);
    // Below four voting nodes BFT tolerates no fault: keep the running set.
    if picked.len() < MIN_OPEN_COMMITTEE {
        return None;
    }
    let selected: Vec<(String, String)> = picked
        .iter()
        .map(|p| {
            let c = all.iter().find(|c| c.index == p.index).expect("picked from all");
            let node = aether_net::EndpointId::from_bytes(&c.node_id).map(|n| n.to_string()).unwrap_or_default();
            (hex::encode(c.validator_key), node)
        })
        .collect();
    let chosen = |k: &str| selected.iter().any(|(s, _)| s == k);
    let incoming: Vec<&(String, String)> = selected.iter().filter(|(k, _)| !running.has(k)).collect();
    // Leave first: members that are not candidates at all (e.g. the genesis set), then unselected ones.
    let registered = |k: &str| all.iter().any(|c| hex::encode(c.validator_key) == k);
    let mut outgoing: Vec<&(String, String)> = running.members.iter().filter(|(k, _)| !chosen(k)).collect();
    outgoing.sort_by_key(|(k, _)| registered(k));
    // Fewer than a third of the seats change per epoch (at least one).
    let n = running.members.len();
    let budget = (n.saturating_sub(1) / 3).max(1);
    let swaps = budget.min(incoming.len()).min(outgoing.len());
    let grow = budget.saturating_sub(swaps).min(incoming.len() - swaps).min(selected.len().saturating_sub(n)).min(MAX_VOTING_NODES.saturating_sub(n));
    let shrink = budget.saturating_sub(swaps).min(outgoing.len() - swaps).min(n.saturating_sub(selected.len().max(MIN_OPEN_COMMITTEE)));
    let leaving: Vec<&String> = outgoing.iter().take(swaps + shrink).map(|(k, _)| k).collect();
    let mut next: Vec<(String, String)> = running.members.iter().filter(|(k, _)| !leaving.contains(&k)).cloned().collect();
    next.extend(incoming.iter().take(swaps + grow).map(|m| (*m).clone()));
    let changed = next.len() != n || next.iter().any(|(k, _)| !running.has(k));
    (next.len() >= MIN_OPEN_COMMITTEE && changed).then_some(next)
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
            let sig = registrar.sign(&attestation_message(7, a, [i; 32], node(i), a)).unwrap();
            s = run(&s, &op, 0, encode_register([i; 32], node(i), a, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()), 1);
            s = run(&s, &op, 1, encode_beacon([i; 32]), e * E + 1);
        }
        s
    }

    fn node(i: u8) -> [u8; 32] {
        *aether_net::SecretKey::from_bytes(&[i; 32]).public().as_bytes()
    }

    fn key(i: u8) -> String {
        hex::encode([i; 32])
    }

    fn running(keys: &[u8]) -> Committee {
        Committee { members: keys.iter().map(|k| (key(*k), format!("node{k}"))).collect() }
    }

    fn keys(set: &[(String, String)]) -> Vec<String> {
        let mut k: Vec<String> = set.iter().map(|(k, _)| k.clone()).collect();
        k.sort();
        k
    }

    #[test]
    fn a_third_of_the_seats_change_per_epoch_starting_with_non_candidates() {
        let s = registry_with(5, 3);
        let genesis_set = running(&[0xa1, 0xa2, 0xa3, 0xa4]);
        // Four seats: one changes per epoch; a genesis (non-candidate) member leaves first.
        let next = next_set(&s, 4, &genesis_set).expect("live candidates replace the genesis set");
        assert_eq!(next.len(), 4);
        assert_eq!(next.iter().filter(|(k, _)| k.starts_with("a1") || k.starts_with("a2") || k.starts_with("a3") || k.starts_with("a4")).count(), 3);
        assert!(next_set(&s, 5, &genesis_set).is_none(), "nobody beaconed in epoch 4");
        let all_candidates = running(&[1, 2, 3, 4, 5]);
        assert!(next_set(&s, 4, &all_candidates).is_none(), "already running");
        assert!(next_set(&s, 4, &Committee::default()).is_none(), "followers propose nothing");
        // Repeated epochs converge on the selected set: four swaps, then one more seat.
        let mut set = genesis_set;
        for _ in 0..5 {
            if let Some(n) = next_set(&s, 4, &set) {
                set = Committee { members: n };
            }
        }
        assert_eq!(keys(&set.members), keys(&all_candidates.members));
    }

    #[test]
    fn too_few_candidates_keep_the_running_set() {
        let s = registry_with(3, 3);
        assert!(next_set(&s, 4, &running(&[0xa1, 0xa2, 0xa3, 0xa4])).is_none());
    }
}
