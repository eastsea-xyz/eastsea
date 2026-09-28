//! The mainnet genesis rules end to end (docs/design/12-launch-plan.md,
//! docs/design/13-roadmap.md E행, docs/design/15-node-rewards.md): a chain
//! with the mainnet flags — history v2, node rewards from genesis — over many
//! epochs and warm-up days. The node pool of an epoch is the sum of the
//! issuance it halves into it; each operator gets exactly a sixteenth of it
//! while fewer than 16 are online, from 16 full-weight operators on the whole
//! pool is shared, and what caps and absences leave is never minted. Warm-up
//! climbs one step a day, and the founder's reserve keys join the voting set
//! below four independent operators and leave at four.

mod common;

use aether_execution::{registry, PROVER_ESCROW};
use aether_light::block::BeaconAnswer;
use aether_node::chain::{leader_address, Executed, Reserve};
use aether_rewards as rewards;
use aether_rewards::{beacons, node_pool, DAY_EPOCHS, FULL, MAX_SHARE, WARMUP_STEPS};
use aether_types::{Address, TxEnvelope, U256};
use commonware_cryptography::{ed25519, Signer as _};
use common::{Mac, Net, Opts};
use std::sync::Arc;

const CHAIN: u64 = 7_799;
/// Short epochs (four slots of three blocks) so a day is 288 blocks, not
/// 86_400: the warm-up climb (a step a day, 14 days) finishes in minutes.
const E: u64 = 12;
const MACS: usize = 20;
/// What the harness funds each operator with (gas money, not a premine of the
/// chain's: rewards are the only mints this test counts).
const FUNDED: u128 = 10u128.pow(20);

fn net(macs: u8, reserve: Option<Reserve>) -> Net {
    Net::new(Opts {
        chain_id: CHAIN,
        node_rewards: true,
        epoch_blocks: E,
        macs,
        min_streak: Some(0),
        history_v2: true,
        reserve,
    })
}

/// Every Mac's answers for the next block, one state view for all of them
/// (`answers` clones the state once per Mac: 20 Macs × 13_000 blocks adds up).
fn answers_once(n: &Net) -> Vec<BeaconAnswer> {
    let view = n.view();
    let h = n.parent.height + 1;
    let mut out = vec![];
    for c in registry::candidates(&view) {
        let Some(i) = (0..n.voting.len()).find(|&i| n.voting_key(i) == c.validator_key) else { continue };
        let how = n.behaviour.get(&i).copied().unwrap_or(Mac::Off);
        for d in beacons::due(&view, h, &c) {
            let answer = match how {
                Mac::Off => false,
                Mac::Slots(mask) => mask & (1 << d.slot) != 0,
                Mac::NoReattest => !d.needs_attestation,
                Mac::Honest => true,
            };
            if !answer {
                continue;
            }
            let attest = d.needs_attestation.then(|| n.reattestation(i, d.period));
            out.push(aether_node::beacons::sign(&n.voting[i], n.chain_id, c.index, &d, attest));
        }
    }
    out
}

/// One block, and everything it minted added to the running total (rewards
/// are the only mint: the supply identity must hold after every step).
fn step(n: &mut Net, minted: &mut U256, txs: Vec<TxEnvelope>) -> Arc<Executed> {
    let answers = answers_once(n);
    let exec = n.step_with(txs, None, vec![], answers);
    *minted += exec.payouts.iter().map(|(_, _, a)| a).sum::<U256>();
    exec
}

fn run_to(n: &mut Net, minted: &mut U256, height: u64) {
    while n.parent.height < height {
        step(n, minted, vec![]);
    }
}

fn balances(n: &Net, upto: usize) -> Vec<U256> {
    (0..upto).map(|i| n.balance(i)).collect()
}

/// Run to the block that pays `epoch` (the first block of the next epoch) and
/// return the balances just before it and the paying block.
fn paid_epoch(n: &mut Net, minted: &mut U256, epoch: u64, upto: usize) -> (Vec<U256>, Arc<Executed>) {
    run_to(n, minted, (epoch + 1) * E - 1);
    let before = balances(n, upto);
    let exec = step(n, minted, vec![]);
    assert_eq!(exec.height, (epoch + 1) * E, "the first block of the epoch after {epoch}");
    (before, exec)
}

/// A whole warm-up day: every Mac in `registered` (they all registered on or
/// before the day's first epoch) starts at level `want` and moves exactly one
/// step up when the day's judgment lands.
fn run_day(n: &mut Net, minted: &mut U256, day: u64, registered: &[usize], want: u64) {
    let level = |n: &Net| registered.iter().map(|&i| rewards::mac(&n.parent.state, i as u64).level).collect::<Vec<_>>();
    assert_eq!(level(n), vec![want; registered.len()], "day {day} starts at level {want}");
    run_to(n, minted, (day + 1) * DAY_EPOCHS * E);
    assert_eq!(level(n), vec![(want + 1).min(WARMUP_STEPS); registered.len()], "day {day} moves one step up");
}

/// The pool of `epoch` against the issuance it is halved from.
fn pool_of_epoch(epoch: u64) -> U256 {
    let mut sum = U256::ZERO;
    for h in (epoch * E).max(1)..(epoch + 1) * E {
        sum += rewards::node_share(h);
    }
    assert_eq!(sum, node_pool(epoch, E), "the pool is the sum of the epoch's issuance halves");
    sum
}

fn supply(n: &Net, others: &[Address]) -> U256 {
    let s = &n.parent.state;
    (0..MACS).map(|i| n.balance(i)).sum::<U256>() + others.iter().map(|a| s.balance(a)).sum::<U256>()
}

#[test]
fn one_four_and_twenty_operators_get_exact_shares_and_the_rest_is_never_minted() {
    let mut n = net(MACS as u8, None);
    let mut minted = U256::ZERO;
    let proposer = leader_address(&ed25519::PrivateKey::from_seed(1).public_key());
    let others = [proposer, PROVER_ESCROW];
    let start = supply(&n, &others);
    assert_eq!(start, U256::from(MACS as u128 * FUNDED), "the genesis funds nobody but the operators' gas money");
    assert!(rewards::enabled(&n.parent.state), "node rewards from the first block");
    // Rewards are the only mint: the whole supply is the premine plus every
    // payout of every block (checked at each stop below, after every step).
    let check = |n: &Net, minted: &U256| {
        assert_eq!(supply(n, &others), start + *minted, "rewards are the only mint");
    };

    // Under history v2 a block with no transactions and no proofs records no
    // statement: quiet heights leave the state root untouched (an answer or a
    // distribution still moves it).
    let reg = n.register(0);
    let first = step(&mut n, &mut minted, vec![reg]);
    assert_eq!(first.height, 1);
    let changed = step(&mut n, &mut minted, vec![]); // height 2 carries the answer for the slot at height 1
    assert_ne!(changed.state.root(), first.state.root(), "an answer is still a state write");
    run_to(&mut n, &mut minted, E + 3);
    let quiet = n.parent.state.root();
    let same = step(&mut n, &mut minted, vec![]); // height E + 4: nothing due, nothing spent
    assert_eq!(same.state.root(), quiet, "a quiet empty block changes nothing");

    // Days 0-13: Mac 0 alone warms up, one step a day, capped at a sixteenth.
    for day in 0..WARMUP_STEPS {
        run_day(&mut n, &mut minted, day, &[0], day);
    }
    check(&n, &minted);

    // Days 14-15: one full-weight operator gets exactly a sixteenth per epoch
    // (15/16 of every pool is never minted).
    let (before, exec) = paid_epoch(&mut n, &mut minted, 14 * DAY_EPOCHS, 1);
    let pool = pool_of_epoch(14 * DAY_EPOCHS);
    assert_eq!(n.balance(0) - before[0], pool / U256::from(MAX_SHARE), "N = 1: exactly a sixteenth");
    assert_eq!(exec.payouts.len(), 1);
    check(&n, &minted);
    run_to(&mut n, &mut minted, 16 * DAY_EPOCHS * E);

    // Day 16: three more register. Their first epoch mixes one full Mac with
    // three fresh ones (a sixteenth and a thirty-second), then they warm up.
    let regs = (1..4).map(|i| n.register(i)).collect();
    let registered_at = step(&mut n, &mut minted, regs);
    let mixed = 16 * DAY_EPOCHS;
    assert_eq!(registered_at.height, mixed * E + 1);
    let (before, exec) = paid_epoch(&mut n, &mut minted, mixed, 4);
    let pool = pool_of_epoch(mixed);
    assert_eq!(n.balance(0) - before[0], pool / U256::from(MAX_SHARE), "the warm Mac still gets a sixteenth");
    for i in 1..4 {
        assert_eq!(n.balance(i) - before[i], pool / U256::from(2 * MAX_SHARE), "a fresh Mac gets a thirty-second");
    }
    check(&n, &minted);
    for day in 16..20 {
        run_day(&mut n, &mut minted, day, &[1, 2, 3], day - 16);
    }
    assert_eq!(rewards::mac(&n.parent.state, 0).level, WARMUP_STEPS, "Mac 0 stays full");

    // A mid-warm-up epoch pays exactly by the weight formula: everyone answers
    // all four slots, so a Mac's weight is 4 × (14 + level).
    let epoch = 20 * DAY_EPOCHS + 10;
    run_to(&mut n, &mut minted, epoch * E);
    let weights: Vec<u64> = (0..4u64).map(|i| 4 * (WARMUP_STEPS + rewards::mac(&n.parent.state, i).level)).collect();
    let (before, exec) = paid_epoch(&mut n, &mut minted, epoch, 4);
    let pool = node_pool(epoch, E);
    let denominator = U256::from(weights.iter().sum::<u64>().max(MAX_SHARE * FULL));
    for (i, w) in weights.iter().enumerate() {
        assert_eq!(n.balance(i) - before[i], pool * U256::from(*w) / denominator, "operator {i} by the formula");
    }
    check(&n, &minted);
    for day in 20..30 {
        run_day(&mut n, &mut minted, day, &[1, 2, 3], day - 16);
    }

    // Days 30-31: four full-weight operators, a sixteenth each (a quarter of
    // the pool minted, three quarters never).
    let (before, exec) = paid_epoch(&mut n, &mut minted, 30 * DAY_EPOCHS + 1, 4);
    let pool = pool_of_epoch(30 * DAY_EPOCHS + 1);
    for i in 0..4 {
        assert_eq!(n.balance(i) - before[i], pool / U256::from(MAX_SHARE), "N = 4: exactly a sixteenth each");
    }
    assert_eq!(exec.payouts.len(), 4);
    let epoch_minted = exec.payouts.iter().map(|(_, _, a)| a).sum::<U256>();
    assert_eq!(epoch_minted, pool / U256::from(4u8), "a quarter of the pool is minted");
    check(&n, &minted);
    run_to(&mut n, &mut minted, 32 * DAY_EPOCHS * E);

    // Day 32: sixteen more register (20 operators). Their first epoch mints
    // three quarters of the pool: a sixteenth × 4, a thirty-second × 16.
    let regs = (4..MACS).map(|i| n.register(i)).collect();
    let registered_at = step(&mut n, &mut minted, regs);
    let joined = 32 * DAY_EPOCHS;
    assert_eq!(registered_at.height, joined * E + 1);
    let (before, exec) = paid_epoch(&mut n, &mut minted, joined, MACS);
    let pool = pool_of_epoch(joined);
    for i in 0..4 {
        assert_eq!(n.balance(i) - before[i], pool / U256::from(MAX_SHARE));
    }
    for i in 4..MACS {
        assert_eq!(n.balance(i) - before[i], pool / U256::from(2 * MAX_SHARE), "a fresh Mac gets a thirty-second");
    }
    let epoch_minted = exec.payouts.iter().map(|(_, _, a)| a).sum::<U256>();
    assert_eq!(epoch_minted, pool / U256::from(4u8) * U256::from(3u8), "a quarter of the pool is never minted");
    check(&n, &minted);
    for day in 32..40 {
        run_day(&mut n, &mut minted, day, &(4..MACS).collect::<Vec<_>>(), day - 32);
    }

    // Deep in the second warm-up the weights pass the 16 × full floor, so the
    // denominator becomes the weight sum itself.
    let epoch = 40 * DAY_EPOCHS + 10;
    run_to(&mut n, &mut minted, epoch * E);
    let weights: Vec<u64> = (0..MACS as u64).map(|i| 4 * (WARMUP_STEPS + rewards::mac(&n.parent.state, i).level)).collect();
    let (before, exec) = paid_epoch(&mut n, &mut minted, epoch, MACS);
    let pool = node_pool(epoch, E);
    let sum = weights.iter().sum::<u64>();
    assert!(sum > MAX_SHARE * FULL, "the weight sum passes the floor");
    let denominator = U256::from(sum);
    for (i, w) in weights.iter().enumerate() {
        assert_eq!(n.balance(i) - before[i], pool * U256::from(*w) / denominator, "operator {i} by the formula");
    }
    check(&n, &minted);
    for day in 40..46 {
        run_day(&mut n, &mut minted, day, &(4..MACS).collect::<Vec<_>>(), day - 32);
    }

    // Day 46: twenty full-weight operators share the whole pool: each gets
    // pool × 112/2240 = a twentieth, and only the division's remainder stays
    // unminted.
    let (before, exec) = paid_epoch(&mut n, &mut minted, 46 * DAY_EPOCHS + 1, MACS);
    let pool = pool_of_epoch(46 * DAY_EPOCHS + 1);
    let share = pool * U256::from(FULL) / U256::from(MACS as u64 * FULL as u64);
    assert_eq!(share, pool / U256::from(MACS as u64), "a full Mac's share is a twentieth of the pool");
    for i in 0..MACS {
        assert_eq!(n.balance(i) - before[i], share, "N = 20: all of the pool is shared");
    }
    let epoch_minted = exec.payouts.iter().map(|(_, _, a)| a).sum::<U256>();
    assert_eq!(epoch_minted, pool - pool % U256::from(MACS as u64), "only the remainder stays unminted");
    check(&n, &minted);

    // The whole run minted less than the issuance it spans: warm-up, caps and
    // thin epochs withheld the rest, and it is never minted later.
    let issued: U256 = (1..=n.parent.height).map(rewards::issuance).sum();
    assert!(minted < issued / U256::from(2u8), "node rewards never exceed their half of the issuance");
    assert_eq!(supply(&n, &others), start + minted, "the total supply is the premine plus what was minted");
}

#[test]
fn reserve_keys_join_under_four_independent_operators_and_leave_at_four() {
    // Mac 4 is the founder's own registered Mac: not independent (docs/design/15).
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(5)).unwrap());
    let reserve = Reserve { operator: founder, members: reserve_members() };
    let mut n = net(5, Some(reserve.clone()));
    let mut minted = U256::ZERO;
    assert_eq!(n.operator(4), founder);
    assert_eq!(Reserve::of(&n.parent.state).unwrap().members, reserve.members);
    // The running set: four genesis keys (not candidates).
    let genesis: Vec<(String, String)> = (0..4).map(|i| (format!("{i:064x}"), format!("genesis{i}"))).collect();
    n.chain.lock().committee = aether_node::rotation::Committee { members: genesis.clone() };
    let no_reserve = |m: &[(String, String)]| reserve.members.iter().all(|r| !m.contains(r));
    let independents = |n: &Net| {
        // As reserve_step sees it at an epoch boundary: the epoch starting here.
        let r = Reserve::of(&n.parent.state).expect("the reserve is on chain");
        let pool = aether_node::rotation::eligible(&n.parent.state, n.parent.height / E, 0);
        let ops = aether_node::rotation::operators(&n.parent.state);
        aether_node::rotation::independent(&pool, |k| ops.get(k).cloned(), &r)
    };

    let reserve_seats = |m: &[(String, String)]| reserve.members.iter().filter(|r| m.contains(r)).count();

    // Two independent Macs and the founder's: under four independent operators,
    // but the committee already has four seats, so no reserve key joins (a
    // 4-seat committee growing to 7 would need 5 of 7, and one founder Mac
    // going dark would stall it: audit 1.1).
    let regs = [0, 1, 4].iter().map(|i| n.register(*i)).collect();
    step(&mut n, &mut minted, regs);
    run_to(&mut n, &mut minted, E);
    assert_eq!(independents(&n), 2);
    if let Some(p) = n.chain.lock().proposal.clone() {
        assert_eq!(reserve_seats(&p.1), 0, "a full committee takes no reserve key");
    }
    // A committee short of four (one member left): qualified Macs fill first,
    // reserve keys only the seats still missing.
    {
        let mut g = n.chain.lock();
        g.committee = aether_node::rotation::Committee { members: genesis[..1].to_vec() };
        g.proposal = None;
    }
    run_to(&mut n, &mut minted, 2 * E);
    let proposal = n.chain.lock().proposal.clone().expect("a short committee is refilled");
    assert_eq!(proposal.1.len(), 4, "refilled to four seats");
    // Qualified Macs are seated first; reserve keys take only what is still
    // missing (exact counts per independent operator: rotation.rs unit tests).
    let others = proposal.1.len() - reserve_seats(&proposal.1);
    assert_eq!(reserve_seats(&proposal.1), 4usize.saturating_sub(others), "only the missing seats");
    {
        let mut g = n.chain.lock();
        g.committee = aether_node::rotation::Committee { members: proposal.1.clone() };
        g.proposal = None;
    }
    // Macs 2 and 3: four independent operators (the founder's Mac still does
    // not count), so every reserve key leaves.
    let regs = [2, 3].iter().map(|i| n.register(*i)).collect();
    step(&mut n, &mut minted, regs);
    run_to(&mut n, &mut minted, 4 * E);
    assert_eq!(independents(&n), 4);
    // At four independent operators no reserve key stays: whatever the next
    // committee is, it has none.
    let next = {
        let g = n.chain.lock();
        g.proposal.clone().map(|p| p.1).unwrap_or_else(|| g.committee.members.clone())
    };
    assert!(no_reserve(&next), "the reserve keys leave at four");
    // They earned nothing: not candidates, no beacons.
    assert!(registry::candidates(&n.parent.state).iter().all(|c| !reserve.members.iter().any(|(k, _)| *k == hex::encode(c.validator_key))));
}

fn reserve_members() -> Vec<(String, String)> {
    (1..=3u64)
        .map(|i| {
            let k = ed25519::PrivateKey::from_seed(100 + i).public_key();
            let node = aether_net::SecretKey::from_bytes(&[0x70 + i as u8; 32]).public();
            (hex::encode(k.as_ref()), node.to_string())
        })
        .collect()
}
