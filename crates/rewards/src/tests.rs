use super::*;
use aether_execution::PROVER_ESCROW;
use aether_execution::registry::{Candidate, Params};

const EB: u64 = 10;

fn network() -> WorldState {
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
    enable(&mut s);
    s
}

fn operator(i: u64) -> Address {
    Address::from_word(U256::from(0x1000 + i).into())
}

/// Register Mac `index` for `op` in epoch `epoch` (a registration counts as that epoch's beacon).
fn register(s: &mut WorldState, index: u64, op: Address, epoch: u64) {
    let c = Candidate {
        index,
        operator: op,
        validator_key: U256::from(index + 1).to_be_bytes(),
        node_id: [0; 32],
        beaconer: op,
        registered_epoch: epoch,
        last_epoch: epoch,
        streak: 1,
        missed: 0,
    };
    put_candidate(s, &c);
}

fn beacon(s: &mut WorldState, index: u64, epoch: u64) {
    let mut c = registry::candidates(s)[index as usize].clone();
    c.last_epoch = epoch;
    put_candidate(s, &c);
}

fn warm(s: &mut WorldState, index: u64, level: u64) {
    let m = Mac { level, ..mac(s, index) };
    set_mac(s, index, m);
}

fn supply(s: &WorldState, n: u64) -> U256 {
    (0..n).map(|i| s.balance(&operator(i))).sum()
}

/// `n` warmed-up operators, one Mac each, all beaconing in epoch 0; epoch 0 paid at height EB.
fn full_weight(n: u64) -> (WorldState, Distribution) {
    let mut s = network();
    for i in 0..n {
        register(&mut s, i, operator(i), 0);
        warm(&mut s, i, WARMUP_STEPS);
    }
    assert!(distributes(&s, EB));
    let d = distribute(&mut s, EB).unwrap();
    (s, d)
}

#[test]
fn issuance_splits_into_a_node_and_a_proof_share() {
    const YEAR: u64 = 365 * DAY_BLOCKS;
    for h in [1, DAY_BLOCKS - 1, DAY_BLOCKS, YEAR, 18 * YEAR + 3] {
        assert_eq!(node_share(h) + proof_share(h), issuance(h));
    }
    assert_eq!(node_share(1), U256::from(proofs::ISSUE_0 / 2));
    // Epoch 0 starts at the genesis, which issues nothing.
    assert_eq!(node_pool(0, EB), node_share(1) * U256::from(EB - 1));
    assert_eq!(node_pool(3, EB), node_share(1) * U256::from(EB));
    // 7-block epochs: one of them straddles the first daily step.
    let straddle = DAY_BLOCKS / 7;
    let expect = (straddle * 7..straddle * 7 + 7).map(node_share).sum::<U256>();
    assert_eq!(node_pool(straddle, 7), expect);
    assert!(node_share(DAY_BLOCKS) < node_share(DAY_BLOCKS - 1));
}

#[test]
fn issuance_falls_15_percent_a_year_to_a_floor_of_a_tenth() {
    const YEAR: u64 = 365 * DAY_BLOCKS;
    let aeth = |h: u64| issuance(h).to::<u128>() as f64 / 1e18;
    assert_eq!(issuance(1), U256::from(proofs::ISSUE_0));
    // Constant within a day, lower the next.
    assert_eq!(issuance(DAY_BLOCKS - 1), issuance(1));
    assert!(issuance(DAY_BLOCKS) < issuance(DAY_BLOCKS - 1));
    for (years, expect) in [(1, 0.85), (2, 0.7225), (4, 0.522), (10, 0.1969)] {
        let got = aeth(years * YEAR);
        assert!((got - expect).abs() < 0.001, "year {years}: {got} vs {expect}");
    }
    // Never below the floor, and it stays there.
    assert_eq!(issuance(15 * YEAR), U256::from(TAIL));
    assert_eq!(issuance(100 * YEAR), U256::from(TAIL));
    assert_eq!(issuance(u64::MAX), U256::from(TAIL));
    // Monotone, including where the exact power meets the floor and the shortcut.
    let mut last = issuance(1);
    for day in (0..6_000).step_by(7) {
        let now = issuance(day * DAY_BLOCKS);
        assert!(now <= last, "day {day}");
        last = now;
    }
    // Whole schedule before the floor: ~31.5M × (1 − 0.1) / 0.1625 ≈ 175M AETH,
    // the first year about 29M of it.
    let first_year: f64 = (0..365).map(|d| aeth(d * DAY_BLOCKS + 1) * DAY_BLOCKS as f64).sum();
    assert!((28.0e6..30.0e6).contains(&first_year), "{first_year}");
}

#[test]
fn nothing_happens_unless_the_genesis_enabled_it() {
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
    assert!(!enabled(&s) && !distributes(&s, EB));
    assert!(distributes(&network(), EB) && !distributes(&network(), EB + 1) && !distributes(&network(), 0));
}

#[test]
fn fewer_than_16_operators_get_a_sixteenth_each_and_the_rest_is_not_minted() {
    for n in [1u64, 4] {
        let (s, d) = full_weight(n);
        let pool = node_pool(0, EB);
        assert_eq!(d.pool, pool);
        assert_eq!(d.paid.len() as u64, n);
        for i in 0..n {
            assert_eq!(s.balance(&operator(i)), pool / U256::from(16u8), "N={n}");
        }
        assert_eq!(supply(&s, n), pool / U256::from(16u8) * U256::from(n));
        assert_eq!(d.unminted, pool - supply(&s, n));
    }
}

#[test]
fn from_16_operators_the_whole_pool_is_shared() {
    let pool = node_pool(0, EB);
    let (s, d) = full_weight(16);
    for i in 0..16 {
        assert_eq!(s.balance(&operator(i)), pool / U256::from(16u8));
    }
    assert_eq!(d.unminted, pool - pool / U256::from(16u8) * U256::from(16u8));
    let (s, d) = full_weight(20);
    for i in 0..20 {
        assert_eq!(s.balance(&operator(i)), pool / U256::from(20u8));
    }
    // Only rounding dust stays unminted; never more than the pool is paid.
    assert!(d.unminted < U256::from(20u8));
    assert_eq!(supply(&s, 20) + d.unminted, pool);
}

#[test]
fn shares_follow_weights_and_nobody_exceeds_a_sixteenth() {
    // 20 operators: 10 warmed up, 10 new (weight 0.5): W = 15 full weights < 16.
    let mut s = network();
    for i in 0..20 {
        register(&mut s, i, operator(i), 0);
        if i < 10 {
            warm(&mut s, i, WARMUP_STEPS);
        }
    }
    let d = distribute(&mut s, EB).unwrap();
    let sixteenth = d.pool / U256::from(16u8);
    assert_eq!(s.balance(&operator(0)), sixteenth);
    assert_eq!(s.balance(&operator(19)), d.pool / U256::from(32u8));
    // 30 operators, 20 warmed up and 10 new: W = 25 full weights.
    let mut s = network();
    for i in 0..30 {
        register(&mut s, i, operator(i), 0);
        if i < 20 {
            warm(&mut s, i, WARMUP_STEPS);
        }
    }
    let d = distribute(&mut s, EB).unwrap();
    let w = U256::from(25 * FULL);
    assert_eq!(s.balance(&operator(0)), d.pool * U256::from(FULL) / w);
    assert_eq!(s.balance(&operator(29)), d.pool * U256::from(FULL / 2) / w);
    assert!(s.balance(&operator(0)) <= sixteenth);
    assert_eq!(supply(&s, 30) + d.unminted, d.pool);
}

#[test]
fn an_operator_counts_once_however_many_macs_it_has() {
    let mut s = network();
    // Operator 0: three Macs (one warmed up); operator 1: one new Mac; operator 2's Mac is silent.
    register(&mut s, 0, operator(0), 0);
    register(&mut s, 1, operator(0), 0);
    register(&mut s, 2, operator(0), 0);
    warm(&mut s, 2, WARMUP_STEPS);
    register(&mut s, 3, operator(1), 0);
    register(&mut s, 4, operator(2), 0);
    let d = distribute(&mut s, EB).unwrap();
    assert_eq!(d.paid.len(), 3);
    beacon(&mut s, 0, 1);
    beacon(&mut s, 1, 1);
    beacon(&mut s, 2, 1);
    beacon(&mut s, 3, 1);
    let before = (s.balance(&operator(0)), s.balance(&operator(1)), s.balance(&operator(2)));
    let d = distribute(&mut s, 2 * EB).unwrap();
    assert_eq!(d.paid.len(), 2, "the silent operator is not counted");
    assert_eq!(s.balance(&operator(0)) - before.0, d.pool / U256::from(16u8));
    assert_eq!(s.balance(&operator(1)) - before.1, d.pool / U256::from(32u8));
    assert_eq!(s.balance(&operator(2)), before.2);
}

/// Run epochs `from..to` with `up(index, epoch)` deciding which Macs beacon.
fn run(s: &mut WorldState, macs: u64, from: u64, to: u64, up: impl Fn(u64, u64) -> bool) {
    for e in from..to {
        for i in 0..macs {
            if up(i, e) {
                beacon(s, i, e);
            }
        }
        distribute(s, (e + 1) * EB).unwrap();
    }
}

#[test]
fn warm_up_takes_14_good_days_goes_down_a_day_at_a_time_and_stays_in_bounds() {
    let mut s = network();
    register(&mut s, 0, operator(0), 0);
    assert_eq!(mac(&s, 0).warmup(), WARMUP_STEPS, "a new Mac starts at 0.5");
    let day = DAY_EPOCHS;
    for d in 0..WARMUP_STEPS + 3 {
        run(&mut s, 1, d * day, (d + 1) * day, |_, _| true);
        assert_eq!(mac(&s, 0).level, (d + 1).min(WARMUP_STEPS), "day {d}");
    }
    assert_eq!(mac(&s, 0).warmup(), 2 * WARMUP_STEPS, "full weight after 14 good days, no more after");
    let mut d = WARMUP_STEPS + 3;
    // A bad day (asleep for 3 of 24 hours: 87.5% < 90%) costs one step, not everything.
    run(&mut s, 1, d * day, (d + 1) * day, |_, e| e % day >= 3);
    assert_eq!(mac(&s, 0).level, WARMUP_STEPS - 1);
    d += 1;
    // 22 of 24 hours (91.7%) is a good day.
    run(&mut s, 1, d * day, (d + 1) * day, |_, e| e % day >= 2);
    assert_eq!(mac(&s, 0).level, WARMUP_STEPS);
    d += 1;
    // Many bad days: never below 0.5. (A single Mac is the whole network: keep it above 70%.)
    for _ in 0..WARMUP_STEPS + 2 {
        run(&mut s, 1, d * day, (d + 1) * day, |_, e| e % day >= 6);
        d += 1;
    }
    assert_eq!(mac(&s, 0).level, 0);
    // The reward follows: a Mac at 0.5 alone gets 1/32.
    beacon(&mut s, 0, d * day);
    let before = s.balance(&operator(0));
    let dist = distribute(&mut s, (d * day + 1) * EB).unwrap();
    assert_eq!(s.balance(&operator(0)) - before, dist.pool / U256::from(32u8));
}

#[test]
fn a_day_the_whole_network_was_mostly_down_moves_nobody() {
    let mut s = network();
    for i in 0..4 {
        register(&mut s, i, operator(i), 0);
    }
    let day = DAY_EPOCHS;
    // Day 0: everyone up, all move to level 1.
    run(&mut s, 4, 0, day, |_, _| true);
    assert!((0..4).all(|i| mac(&s, i).level == 1));
    // Day 1: an outage keeps everyone down for 12 hours (50% < 70%): neutral.
    run(&mut s, 4, day, 2 * day, |_, e| e % day >= 12);
    assert!((0..4).all(|i| mac(&s, i).level == 1));
    // Day 2: only Mac 3 is down half the day (network 87.5% ≥ 70%): it alone steps down.
    run(&mut s, 4, 2 * day, 3 * day, |i, e| i != 3 || e % day >= 12);
    assert_eq!((0..4).map(|i| mac(&s, i).level).collect::<Vec<_>>(), vec![2, 2, 2, 0]);
}

#[test]
fn long_silent_macs_do_not_make_every_day_neutral() {
    let mut s = network();
    for i in 0..10 {
        register(&mut s, i, operator(i), 0);
    }
    let day = DAY_EPOCHS;
    // Macs 1..10 leave after registration; Mac 0 stays up.
    run(&mut s, 10, 0, 3 * day, |i, _| i == 0);
    // Day 0: 10% of the network answered: neutral. From day 2 the silent Macs no longer count.
    assert_eq!(mac(&s, 0).level, 1);
    assert_eq!(mac(&s, 5).level, 0);
}

#[test]
fn a_mac_registered_mid_day_is_judged_from_its_first_full_day() {
    let mut s = network();
    register(&mut s, 0, operator(0), 0);
    run(&mut s, 1, 0, 5, |_, _| true);
    register(&mut s, 1, operator(1), 5);
    run(&mut s, 2, 5, DAY_EPOCHS, |_, _| true);
    assert_eq!((mac(&s, 0).level, mac(&s, 1).level), (1, 0));
    run(&mut s, 2, DAY_EPOCHS, 2 * DAY_EPOCHS, |_, _| true);
    assert_eq!((mac(&s, 0).level, mac(&s, 1).level), (2, 1));
}

fn proof_setup() -> WorldState {
    let mut s = network();
    register(&mut s, 0, operator(0), 0);
    s.set_balance(PROVER_ESCROW, U256::from(1_000_000u64)).unwrap();
    s
}

#[test]
fn only_registered_operators_are_paid_issuance_for_proofs_and_escrow_is_always_paid() {
    let mut s = proof_setup();
    proofs::record(&mut s, 11, [1; 32], U256::from(700u64));
    proofs::record(&mut s, 12, [2; 32], U256::from(300u64));
    let stranger = Address::repeat_byte(0x99);
    assert_eq!(pay_proof(&mut s, 11, 13, stranger), Ok((U256::from(700u64), U256::ZERO)));
    assert_eq!(s.balance(&stranger), U256::from(700u64));
    assert_eq!(proofs::prover(&s, 11), Some(stranger), "proven: its issuance is gone for good");
    let (paid, issued) = pay_proof(&mut s, 12, 13, operator(0)).unwrap();
    // 10-block epochs: a sixteenth of the epoch's proof share is less than one block's.
    assert_eq!(issued, proof_pool(1, EB) / U256::from(16u8));
    assert!(issued < proof_share(12));
    assert_eq!(paid, U256::from(300u64) + issued);
    assert_eq!(pay_proof(&mut s, 12, 14, operator(0)), Err(ClaimError::AlreadyProven));
}

#[test]
fn proof_issuance_is_capped_at_a_sixteenth_of_the_epoch_per_operator() {
    // Big epochs so the cap spans several blocks: cap = 1/16 of 160 blocks = 10 blocks' proof share.
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: 160, min_streak: 0, draw_epochs: 1 }).unwrap();
    enable(&mut s);
    register(&mut s, 0, operator(0), 0);
    s.set_balance(PROVER_ESCROW, U256::from(1_000_000u64)).unwrap();
    let cap = proof_pool(1, 160) / U256::from(16u8);
    assert_eq!(cap, proof_share(200) * U256::from(10u8));
    let now = 200; // epoch 1
    let mut minted = U256::ZERO;
    let mut escrow = U256::ZERO;
    for h in 170..185u64 {
        proofs::record(&mut s, h, [h as u8; 32], U256::from(10u64));
        let (paid, issued) = pay_proof(&mut s, h, now, operator(0)).unwrap();
        assert_eq!(paid, U256::from(10u64) + issued, "escrow is paid uncapped");
        minted += issued;
        escrow += U256::from(10u64);
    }
    assert_eq!(minted, cap, "5 proofs past the cap mint nothing");
    assert_eq!(proof_paid(&s, operator(0), 1), cap);
    assert_eq!(s.balance(&operator(0)), cap + escrow);
    // The next epoch has a fresh cap.
    proofs::record(&mut s, 330, [7; 32], U256::ZERO);
    assert_eq!(pay_proof(&mut s, 330, 331, operator(0)).unwrap().1, proof_share(330));
    assert_eq!(proof_paid(&s, operator(0), 1), U256::ZERO);
}

#[test]
fn after_the_first_year_a_proof_mints_this_networks_issuance_not_the_halving() {
    // Year 3: the testnet's halving pays 0.25 AETH, the decay about 0.61.
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: 160, min_streak: 0, draw_epochs: 1 }).unwrap();
    enable(&mut s);
    register(&mut s, 0, operator(0), 0);
    s.set_balance(PROVER_ESCROW, U256::from(1_000_000u64)).unwrap();
    let h = 3 * 365 * DAY_BLOCKS + 5;
    assert!(proof_share(h) > proofs::issuance(h), "the decay pays more than the halving here");
    proofs::record(&mut s, h, [9; 32], U256::from(10u64));
    let before = s.balance(&operator(0));
    let (paid, issued) = pay_proof(&mut s, h, h + 1, operator(0)).unwrap();
    assert_eq!(issued, proof_share(h));
    assert_eq!(paid, U256::from(10u64) + issued);
    assert_eq!(s.balance(&operator(0)), before + paid);
}

#[test]
fn over_many_epochs_no_more_than_the_issuance_is_minted() {
    let mut s = network();
    for i in 0..5 {
        register(&mut s, i, operator(i), 0);
    }
    let epochs = 3 * DAY_EPOCHS;
    let mut unminted = U256::ZERO;
    let mut pools = U256::ZERO;
    for e in 0..epochs {
        for i in 0..5 {
            if (i + e) % 5 != 0 {
                beacon(&mut s, i, e);
            }
        }
        let d = distribute(&mut s, (e + 1) * EB).unwrap();
        unminted += d.unminted;
        pools += d.pool;
    }
    assert_eq!(pools, (1..epochs * EB).map(node_share).sum::<U256>(), "the genesis issues nothing");
    assert_eq!(supply(&s, 5) + unminted, pools);
    // Five operators, at most 1/16 each: at least 11/16 of the node share was never minted.
    assert!(supply(&s, 5) * U256::from(16u8) <= pools * U256::from(5u8));
}
