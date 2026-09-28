//! Which voting set comes next (docs/design/07-consensus.md, "open voting
//! nodes"). No person decides: at the first block of every registry epoch,
//! each node computes the proposed set from the registry state that block
//! builds on. The running set then reshares its key to it in the background and
//! hands over with a signed handoff (`handoff.rs`); the chain never stops for it.
//!
//! At most a third of the seats change per epoch, so a newcomer group (even one
//! owner with many Macs and wallets) cannot take the whole set at once: it has
//! to stay live for several epochs, while the running set keeps a quorum.

use aether_consensus::committee::MIN_OPEN_COMMITTEE;
use aether_execution::registry;
use aether_execution::WorldState;

/// In a validator's data dir: files a background reshare stages for a handoff.
pub const STAGED_THRESHOLD: &str = "threshold-next.json";
pub const STAGED_NETWORK: &str = "network-next.json";

/// In a voting node's data dir: the block (and finalization) it verified last
/// as a follower, which it starts from when it has no validator history.
pub const ANCHOR_FILE: &str = "anchor.json";

/// Largest voting set (the DKG cost grows with its square).
pub const MAX_VOTING_NODES: usize = MAX_DRAWN;

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

/// Largest voting set drawn (the DKG cost grows with its square).
pub const MAX_DRAWN: usize = 128;

/// Macs that can be drawn at registry epoch `epoch` (from the state its first
/// block builds on): registered with a valid iroh node id, alive in the previous
/// epoch, with at least `min_streak` epochs of unbroken liveness. One ticket
/// each: a wallet or owner with many addresses gains nothing, only more Macs.
pub fn eligible(state: &WorldState, epoch: u64, min_streak: u64) -> Vec<(String, String)> {
    if epoch == 0 {
        return vec![];
    }
    registry::candidates(state)
        .into_iter()
        // Up at least 95% of the time during the streak (missed * 20 <= streak).
        .filter(|c| c.last_epoch == epoch - 1 && c.streak >= min_streak && c.missed.saturating_mul(20) <= c.streak)
        .filter_map(|c| aether_net::EndpointId::from_bytes(&c.node_id).ok().map(|n| (hex::encode(c.validator_key), n.to_string())))
        .collect()
}

/// Registered voting keys (hex) and their operators (the address that registered them).
pub fn operators(state: &WorldState) -> std::collections::HashMap<String, String> {
    registry::candidates(state).iter().map(|c| (hex::encode(c.validator_key), format!("{:#x}", c.operator))).collect()
}

/// A Mac's place in the draw: H(seed ‖ voting key).
pub fn ticket(seed: &[u8], key: &str) -> Vec<u8> {
    use commonware_cryptography::Hasher as _;
    let mut h = commonware_cryptography::Sha256::default();
    h.update(seed);
    h.update(key.as_bytes());
    h.finalize().1.as_ref().to_vec()
}

/// Voting-set size for a pool: a quarter of it (a sample, not the whole
/// pool), as 3f+1, between 4 and `MAX_DRAWN`; small pools seat everyone.
pub fn target_size(pool: usize) -> usize {
    let n = (pool / 4).clamp(MIN_OPEN_COMMITTEE, MAX_DRAWN).min(pool);
    3 * ((n.max(1) - 1) / 3) + 1
}

/// The next voting set: the pool ordered by H(seed ‖ key), the first
/// `target_size` drawn with at most f of 3f+1 seats per operator (one owner
/// never holds a third); fewer than a third of the running seats change per
/// draw (at least one), members that are not candidates leave first. None when
/// the pool is too small or nothing changes. `candidate(key)` is the operator
/// of a registered voting key (None: not registered).
///
/// The operator is the address that registered the key, so the cap binds an
/// account, not a person: one owner using many addresses is limited only by
/// what makes each voting key cost something (one per Mac via DeviceCheck, at
/// most 16 new per epoch, a day of unbroken liveness before being drawn).
pub fn draw(pool: &[(String, String)], seed: &[u8], candidate: impl Fn(&str) -> Option<String>, running: &Committee) -> Option<Vec<(String, String)>> {
    draw_sized(pool, seed, candidate, running, target_size(pool.len()), MIN_OPEN_COMMITTEE)
}

/// `draw` with its target size and the size it never shrinks below given.
fn draw_sized(
    pool: &[(String, String)],
    seed: &[u8],
    candidate: impl Fn(&str) -> Option<String>,
    running: &Committee,
    target: usize,
    floor: usize,
) -> Option<Vec<(String, String)>> {
    if running.members.is_empty() || pool.len() < MIN_OPEN_COMMITTEE {
        return None;
    }
    let mut order: Vec<&(String, String)> = pool.iter().collect();
    order.sort_by_cached_key(|(k, _)| ticket(seed, k));
    let cap = ((target - 1) / 3).max(1);
    let mut seats: std::collections::HashMap<String, usize> = Default::default();
    let selected: Vec<(String, String)> = order
        .into_iter()
        .filter(|(k, _)| {
            let taken = seats.entry(candidate(k).unwrap_or_else(|| k.clone())).or_default();
            *taken += 1;
            *taken <= cap
        })
        .take(target)
        .cloned()
        .collect();
    let chosen = |k: &str| selected.iter().any(|(s, _)| s == k);
    let incoming: Vec<&(String, String)> = selected.iter().filter(|(k, _)| !running.has(k)).collect();
    let mut outgoing: Vec<&(String, String)> = running.members.iter().filter(|(k, _)| !chosen(k)).collect();
    outgoing.sort_by_key(|(k, _)| candidate(k).is_some());
    let n = running.members.len();
    let budget = (n.saturating_sub(1) / 3).max(1);
    let swaps = budget.min(incoming.len()).min(outgoing.len());
    let grow = budget.saturating_sub(swaps).min(incoming.len() - swaps).min(selected.len().saturating_sub(n)).min(MAX_VOTING_NODES.saturating_sub(n));
    let shrink = budget.saturating_sub(swaps).min(outgoing.len() - swaps).min(n.saturating_sub(selected.len().max(floor)));
    let leaving: Vec<&String> = outgoing.iter().take(swaps + shrink).map(|(k, _)| k).collect();
    let mut next: Vec<(String, String)> = running.members.iter().filter(|(k, _)| !leaving.contains(&k)).cloned().collect();
    // The cap counts the seats an operator keeps too: an incoming key is added
    // only while its operator holds fewer than `cap` seats in the new set.
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut held: std::collections::HashMap<String, usize> = Default::default();
    for (k, _) in &next {
        *held.entry(op(k)).or_default() += 1;
    }
    for m in incoming.iter().take(swaps + grow) {
        let n = held.entry(op(&m.0)).or_default();
        if *n < cap {
            *n += 1;
            next.push((*m).clone());
        }
    }
    let changed = next.len() != n || next.iter().any(|(k, _)| !running.has(k));
    (next.len() >= MIN_OPEN_COMMITTEE && changed).then_some(next)
}

/// Protocol 3: up to this many seats, a Mac that qualifies joins the voting set
/// instead of replacing a member (the same count that later turns mainnet
/// issuance on). From here on, draws swap seats as in `draw`.
pub const GROW_UNTIL: usize = 16;

/// Protocol-3 draw. Below `GROW_UNTIL` seats, qualifying Macs are added (in
/// ticket order, fewer than a third of the running seats per draw, at least
/// one) and nobody leaves, so every Mac that keeps a day of unbroken uptime
/// becomes a voter; one operator still never holds a third of the new set.
/// At `GROW_UNTIL` seats and above it is `draw`.
pub fn draw_v3(pool: &[(String, String)], seed: &[u8], candidate: impl Fn(&str) -> Option<String>, running: &Committee) -> Option<Vec<(String, String)>> {
    let n = running.members.len();
    if n >= GROW_UNTIL {
        // Swap seats from here on, but never below sixteen.
        let target = target_size(pool.len()).max(GROW_UNTIL);
        return draw_sized(pool, seed, candidate, running, target, GROW_UNTIL);
    }
    if n == 0 {
        return None;
    }
    let mut incoming: Vec<&(String, String)> = pool.iter().filter(|(k, _)| !running.has(k)).collect();
    incoming.sort_by_cached_key(|(k, _)| ticket(seed, k));
    let budget = (n.saturating_sub(1) / 3).max(1).min(GROW_UNTIL - n).min(incoming.len());
    if budget == 0 {
        return None;
    }
    let cap = ((n + budget - 1) / 3).max(1);
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut held: std::collections::HashMap<String, usize> = Default::default();
    for (k, _) in &running.members {
        *held.entry(op(k)).or_default() += 1;
    }
    let mut next = running.members.clone();
    for m in incoming {
        if next.len() - n == budget {
            break;
        }
        let h = held.entry(op(&m.0)).or_default();
        if *h < cap {
            *h += 1;
            next.push(m.clone());
        }
    }
    (next.len() > n).then_some(next)
}

/// Founder reserve keys (docs/design/12-launch-plan.md, "창업자 Mac 안전망"): the
/// founder's one extra right is running up to three voting keys on one Mac.
/// They are not registry candidates (no beacons, no rewards) and are seated
/// only while fewer than `MIN_OPEN_COMMITTEE` independent operators qualify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reserve {
    /// The founder's operator address (lowercase 0x hex): its own Macs are not independent.
    pub operator: String,
    /// (ed25519 key hex, iroh node id).
    pub members: Vec<(String, String)>,
}

impl Reserve {
    fn has(&self, key: &str) -> bool {
        self.members.iter().any(|(k, _)| k == key)
    }
}

/// Distinct operators, other than the founder, with a Mac in the pool.
pub fn independent(pool: &[(String, String)], candidate: impl Fn(&str) -> Option<String>, reserve: &Reserve) -> usize {
    pool.iter()
        .filter(|(k, _)| !reserve.has(k))
        .filter_map(|(k, _)| candidate(k))
        .filter(|op| *op != reserve.operator)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// The next voting set with the founder's reserve keys applied to `drawn`
/// (the draw's result, or None: the running set). While fewer than
/// `MIN_OPEN_COMMITTEE` independent operators qualify, reserve keys fill only
/// the seats the committee is short of `MIN_OPEN_COMMITTEE` — never more: a
/// committee that already stands at four seats takes none, because growing it
/// to seven would put three seats on the founder's one Mac and raise the
/// quorum past what the other validators alone can meet, so that one Mac
/// going offline would stall the chain. A committee short of seats first
/// takes qualifying Macs (ticket order, one seat per operator) and reserve
/// keys make up the rest; keys seated beyond the shortfall step down. From
/// four independent operators on, the reserve keys all leave at once, and if
/// that leaves fewer than `MIN_OPEN_COMMITTEE` seats, qualifying Macs fill
/// them the same way. Reserve keys are exempt from the per-operator seat cap:
/// that is the safety net. None when nothing changes.
pub fn with_reserve(
    drawn: Option<Vec<(String, String)>>,
    pool: &[(String, String)],
    seed: &[u8],
    candidate: impl Fn(&str) -> Option<String>,
    reserve: &Reserve,
    running: &Committee,
) -> Option<Vec<(String, String)>> {
    if running.members.is_empty() {
        return drawn;
    }
    let needed = independent(pool, &candidate, reserve) < MIN_OPEN_COMMITTEE;
    let mut next = drawn.clone().unwrap_or_else(|| running.members.clone());
    // Qualifying Macs fill a committee that is short of seats (ticket order,
    // one seat per operator) — both when the reserve keys leave and while
    // they are seated.
    next.retain(|(k, _)| needed || !reserve.has(k));
    let op = |k: &str| candidate(k).unwrap_or_else(|| k.to_string());
    let mut seated: std::collections::BTreeSet<String> = next.iter().map(|(k, _)| op(k)).collect();
    let mut order: Vec<&(String, String)> = pool.iter().filter(|(k, _)| !next.iter().any(|(n, _)| n == k)).collect();
    order.sort_by_cached_key(|(k, _)| ticket(seed, k));
    for m in order {
        if next.len() >= MIN_OPEN_COMMITTEE {
            break;
        }
        if seated.insert(op(&m.0)) {
            next.push(m.clone());
        }
    }
    if needed {
        // Only as many reserve keys as the committee is still short of
        // MIN_OPEN_COMMITTEE seats, never more; keys seated beyond the
        // shortfall step down. The cap stays three (all of them).
        let short = MIN_OPEN_COMMITTEE.saturating_sub(next.iter().filter(|(k, _)| !reserve.has(k)).count()).min(reserve.members.len());
        let mut seated = 0;
        next.retain(|(k, _)| {
            !reserve.has(k) || {
                seated += 1;
                seated <= short
            }
        });
        for m in &reserve.members {
            if seated >= short {
                break;
            }
            if !next.iter().any(|(k, _)| *k == m.0) {
                next.push(m.clone());
                seated += 1;
            }
        }
    } else if next.len() < MIN_OPEN_COMMITTEE {
        // Dropping the reserve keys would leave the committee short and no
        // qualifying Mac can fill it: they stay seated for now.
        return drawn;
    }
    let same = next.len() == running.members.len() && next.iter().all(|m| running.members.contains(m));
    (!same).then_some(next)
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
        registry::predeploy(
            &mut s,
            aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap(),
            registry::Params { epoch_blocks: E, min_streak: 0, draw_epochs: 1 },
        )
        .unwrap();
        for i in 1..=n {
            let op = P256Signer::from_seed(&seed(i)).unwrap();
            let a = address_of(&op.public_key()).unwrap();
            s.set_balance(a, U256::from(10u128.pow(20))).unwrap();
            let sig = registrar.sign(&attestation_message(7, a, [i; 32], node(i), a)).unwrap();
            s = run(&s, &op, 0, encode_register([i; 32], node(i), a, sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap()), (e - 1) * E + 1);
            // Registered in epoch e-1, beacon in e: no epoch missed.
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

    fn draw_at(s: &WorldState, epoch: u64, seed: &[u8], set: &Committee) -> Option<Vec<(String, String)>> {
        let ops = operators(s);
        draw(&eligible(s, epoch, 0), seed, |k| ops.get(k).cloned(), set)
    }

    #[test]
    fn the_cap_counts_the_seats_an_operator_already_holds() {
        // Running {1(whale), 100, 101, 102}; the pool is mostly the whale's: it may not gain a second seat.
        let pool: Vec<(String, String)> = (1..=16u8).map(|i| (key(i), format!("node{i}"))).collect();
        let op = |k: &str| Some(if k <= key(12).as_str() { "0xwhale".to_string() } else { k.to_string() });
        let set = running(&[1, 100, 101, 102]);
        for seed in [b"a".as_slice(), b"b", b"c", b"d", b"e", b"f", b"g"] {
            if let Some(next) = draw(&pool, seed, op, &set) {
                let whale = next.iter().filter(|(k, _)| op(k).as_deref() == Some("0xwhale")).count();
                assert!(whale <= 1, "{whale} whale seats of {}", next.len());
            }
        }
    }

    #[test]
    fn one_operator_never_holds_a_third_of_the_drawn_seats() {
        // 16 eligible Macs, 12 of one owner: a 4-seat draw takes at most one of theirs.
        let pool: Vec<(String, String)> = (1..=16u8).map(|i| (key(i), format!("node{i}"))).collect();
        let op = |k: &str| Some(if k < key(13).as_str() { "0xwhale".to_string() } else { k.to_string() });
        let set = running(&[100, 101, 102, 103]);
        for seed in [b"a".as_slice(), b"b", b"c", b"d", b"e"] {
            let drawn = draw(&pool, seed, op, &set).unwrap();
            let whale = drawn.iter().filter(|(k, _)| op(k).as_deref() == Some("0xwhale")).count();
            assert!(whale <= 1, "{whale} whale seats of {}", drawn.len());
        }
    }

    #[test]
    fn a_third_of_the_seats_change_per_draw_starting_with_non_candidates() {
        let s = registry_with(5, 3);
        let genesis_set = running(&[0xa1, 0xa2, 0xa3, 0xa4]);
        let next = draw_at(&s, 4, b"seed", &genesis_set).expect("live candidates replace the genesis set");
        assert_eq!(next.len(), 4);
        assert_eq!(next.iter().filter(|(k, _)| k.starts_with("a")).count(), 3, "one seat changes");
        assert!(draw_at(&s, 5, b"seed", &genesis_set).is_none(), "nobody beaconed in epoch 4");
        assert!(draw_at(&s, 4, b"seed", &Committee::default()).is_none(), "followers draw nothing");
        // Repeated draws converge on the drawn set (five eligible: target 4 of them).
        let mut set = genesis_set;
        for _ in 0..6 {
            if let Some(n) = draw_at(&s, 4, b"seed", &set) {
                set = Committee { members: n };
            }
        }
        assert_eq!(set.members.len(), 4);
        assert!(set.members.iter().all(|(k, _)| !k.starts_with("a")), "only candidates remain");
    }

    #[test]
    fn the_seed_decides_who_is_drawn_and_nobody_else_can() {
        // A large pool: the draw is a sample, and different seeds draw different sets.
        let pool: Vec<(String, String)> = (0..64u8).map(|i| (hex::encode([i; 32]), format!("n{i}"))).collect();
        let set = Committee { members: pool[..16].to_vec() };
        assert_eq!(target_size(64), 16);
        let sample = |seed: &[u8]| {
            let mut order: Vec<&(String, String)> = pool.iter().collect();
            order.sort_by_cached_key(|(k, _)| ticket(seed, k));
            order.into_iter().take(16).map(|(k, _)| k.clone()).collect::<Vec<_>>()
        };
        assert_ne!(sample(b"one"), sample(b"two"));
        // Wallet addresses play no part: the draw only sees keys (one per Mac).
        let a = draw(&pool, b"one", |k| Some(k.to_string()), &set).unwrap();
        let b = draw(&pool, b"one", |k| Some(k.to_string()), &set).unwrap();
        assert_eq!(a, b, "deterministic for everyone");
        assert!(a.len() == 16 && a.iter().filter(|m| !set.members.contains(m)).count() <= 5, "fewer than a third change");
        assert_eq!(target_size(4), 4);
        assert_eq!(target_size(1000), 127);
    }

    fn committee(n: u8) -> Committee {
        Committee { members: (0..n).map(|i| (format!("g{i}"), format!("gn{i}"))).collect() }
    }

    fn mac(i: u8) -> (String, String) {
        (format!("m{i}"), format!("mn{i}"))
    }

    #[test]
    fn v3_one_qualifying_mac_joins_four_genesis_seats() {
        let next = draw_v3(&[mac(1)], &seed(1), |k| k.starts_with('m').then(|| format!("op-{k}")), &committee(4)).expect("grows");
        assert_eq!(next.len(), 5);
        assert!(next.iter().any(|(k, _)| k == "m1"));
        assert!(committee(4).members.iter().all(|m| next.contains(m)), "nobody leaves while growing");
    }

    #[test]
    fn v3_grows_by_less_than_a_third_per_draw_and_stops_at_sixteen() {
        let pool: Vec<_> = (1..=20).map(mac).collect();
        let ops = |k: &str| k.starts_with('m').then(|| format!("op-{k}"));
        assert_eq!(draw_v3(&pool, &seed(2), ops, &committee(4)).unwrap().len(), 5, "4 seats: one more");
        assert_eq!(draw_v3(&pool, &seed(2), ops, &committee(10)).unwrap().len(), 13, "10 seats: three more");
        assert_eq!(draw_v3(&pool, &seed(2), ops, &committee(15)).unwrap().len(), 16, "never past sixteen by growing");
        let at_cap = draw_v3(&pool, &seed(2), ops, &committee(16)).unwrap();
        assert_eq!(at_cap.len(), 16, "at sixteen, draws swap seats instead");
    }

    #[test]
    fn v3_one_operator_never_holds_a_third_of_the_grown_set() {
        // Two Macs of one operator; four genesis seats are each their own operator.
        let pool = vec![mac(1), mac(2)];
        let next = draw_v3(&pool, &seed(3), |k| k.starts_with('m').then(|| "same-owner".to_string()), &committee(4)).unwrap();
        assert_eq!(next.len(), 5, "one seat per draw at this size");
        let ten = draw_v3(&pool, &seed(3), |k| k.starts_with('m').then(|| "same-owner".to_string()), &committee(10)).unwrap();
        let theirs = ten.iter().filter(|(k, _)| k.starts_with('m')).count();
        assert!(theirs * 3 < ten.len(), "{theirs} of {} seats", ten.len());
    }

    #[test]
    fn v3_nothing_to_add_keeps_the_set() {
        assert!(draw_v3(&[], &seed(4), |_| None, &committee(4)).is_none());
        let running = committee(4);
        let already: Vec<_> = running.members.clone();
        assert!(draw_v3(&already, &seed(4), |_| None, &running).is_none());
    }

    fn reserve() -> Reserve {
        Reserve { operator: "0xf0".into(), members: (1..=3).map(|i| (format!("r{i}"), format!("rn{i}"))).collect() }
    }

    fn ops_of(k: &str) -> Option<String> {
        // Macs "m<i>" belong to operator "op<i>"; "f<i>" are the founder's own registered Macs.
        if let Some(i) = k.strip_prefix('m') {
            Some(format!("op{i}"))
        } else {
            k.starts_with('f').then(|| "0xf0".to_string())
        }
    }

    #[test]
    fn reserve_keys_fill_only_the_seats_a_short_committee_is_missing() {
        // The committee is the qualifying independents themselves: the reserve
        // keys make up the difference to four seats, and only that difference.
        let r = reserve();
        // No independent operator at all: only the founder's own registered Mac qualifies.
        let founder = ("f1".into(), "fn1".into());
        let next = with_reserve(None, std::slice::from_ref(&founder), &seed(5), ops_of, &r, &Committee { members: vec![founder.clone()] }).expect("reserve joins");
        assert_eq!(next.len(), MIN_OPEN_COMMITTEE);
        assert!(r.members.iter().all(|m| next.contains(m)), "three seats short: every reserve key");
        // One to three independent operators seated: three, two, one reserve keys.
        for i in 1..=3u8 {
            let pool: Vec<_> = (1..=i).map(mac).collect();
            assert_eq!(independent(&pool, ops_of, &r), i as usize, "independents count");
            let next = with_reserve(None, &pool, &seed(5), ops_of, &r, &Committee { members: pool.clone() }).expect("reserve joins");
            let seated = next.iter().filter(|(k, _)| k.starts_with('r')).count();
            assert_eq!((seated, next.len()), (MIN_OPEN_COMMITTEE - i as usize, MIN_OPEN_COMMITTEE), "{i} independent operator(s)");
        }
        // A committee that already stands at four seats takes no reserve key at
        // all, however few independent operators qualify: growing four seats
        // to seven would put three of them on the founder's one Mac.
        let pool = vec![mac(1), ("f1".into(), "fn1".into())];
        assert_eq!(independent(&pool, ops_of, &r), 1);
        assert!(with_reserve(None, &pool, &seed(5), ops_of, &r, &committee(4)).is_none(), "four seats standing: no reserve key joins");
        // Qualifying Macs fill a short committee before any reserve key does.
        let pool = vec![mac(1), mac(2), mac(3), ("f1".into(), "fn1".into())];
        let next = with_reserve(None, &pool, &seed(5), ops_of, &r, &Committee { members: vec![mac(1)] }).expect("qualifying Macs fill the seats");
        assert!(next.iter().all(|(k, _)| !k.starts_with('r')), "the Macs cover the shortfall");
        assert_eq!(next.len(), MIN_OPEN_COMMITTEE);
        assert!(next.contains(&mac(2)) && next.contains(&mac(3)) && next.contains(&founder));
        // Already seated: nothing changes.
        let mut full = vec![mac(1)];
        full.extend(r.members.iter().cloned());
        assert!(with_reserve(None, std::slice::from_ref(&mac(1)), &seed(5), ops_of, &r, &Committee { members: full }).is_none());
        // Many Macs of one operator are still one operator.
        let whale: Vec<_> = (0..10).map(|i| (format!("w{i}"), format!("wn{i}"))).collect();
        assert_eq!(independent(&whale, |_| Some("0xwhale".into()), &r), 1);
    }

    #[test]
    fn seated_reserve_keys_step_down_as_the_committee_fills() {
        // A committee that once carried every reserve key keeps fewer as other
        // seats stand, and none once four seats stand without them.
        let r = reserve();
        // The seven-seat set of the old rule (four genesis keys and every
        // reserve key): trimmed back to four seats.
        let mut legacy = committee(4).members;
        legacy.extend(r.members.iter().cloned());
        let next = with_reserve(None, std::slice::from_ref(&mac(1)), &seed(7), ops_of, &r, &Committee { members: legacy }).expect("steps down");
        assert_eq!(next, committee(4).members, "a full committee keeps no reserve seat");
        // Two independents seated with three reserve keys: two step down.
        let mut seated = vec![mac(1), mac(2)];
        seated.extend(r.members.iter().cloned());
        let next = with_reserve(None, &seated, &seed(7), ops_of, &r, &Committee { members: seated.clone() }).expect("steps down");
        assert_eq!(next, vec![mac(1), mac(2), r.members[0].clone(), r.members[1].clone()]);
        // A third independent operator seated: exactly one reserve key stays.
        let mut three = vec![mac(1), mac(2), mac(3)];
        three.push(r.members[0].clone());
        assert!(with_reserve(None, &three.clone(), &seed(7), ops_of, &r, &Committee { members: three }).is_none(), "already the rule's seat count");
    }

    #[test]
    fn reserve_keys_leave_once_four_independent_operators_qualify() {
        let mut seated = vec![mac(1), mac(2)];
        seated.extend(reserve().members);
        let running = Committee { members: seated };
        let pool: Vec<_> = (1..=4).map(mac).collect();
        let next = with_reserve(None, &pool, &seed(6), ops_of, &reserve(), &running).expect("reserve leaves");
        assert!(next.iter().all(|(k, _)| !k.starts_with('r')), "every reserve key leaves at once");
        assert_eq!(next.len(), MIN_OPEN_COMMITTEE, "qualifying Macs fill the seats");
        assert!(next.contains(&mac(3)) && next.contains(&mac(4)));
        // A draw result gets the same treatment.
        let drawn = vec![mac(1), mac(2), mac(3), mac(4), reserve().members[0].clone()];
        let next = with_reserve(Some(drawn), &pool, &seed(6), ops_of, &reserve(), &running).unwrap();
        assert_eq!(next, vec![mac(1), mac(2), mac(3), mac(4)]);
        // Without reserve keys seated and enough operators: nothing to do.
        let clean = Committee { members: (1..=4).map(mac).collect() };
        assert!(with_reserve(None, &pool, &seed(6), ops_of, &reserve(), &clean).is_none());
    }

    #[test]
    fn too_few_eligible_keep_the_running_set() {
        let s = registry_with(3, 3);
        assert!(draw_at(&s, 4, b"seed", &running(&[0xa1, 0xa2, 0xa3, 0xa4])).is_none());
    }
}
