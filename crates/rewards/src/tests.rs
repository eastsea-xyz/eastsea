use super::*;
use aether_execution::PROVER_ESCROW;
use aether_execution::registry::{Candidate, Params};

const EB: u64 = 10;

/// A mask answering every slot of an epoch.
const EVERY_SLOT: u64 = (1 << SLOTS) - 1;
/// A mask answering the first half of an epoch's slots.
const HALF_SLOTS: u64 = (1 << (SLOTS / 2)) - 1;

fn network() -> WorldState {
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
    enable(&mut s);
    s
}

fn operator(i: u64) -> Address {
    Address::from_word(U256::from(0x1000 + i).into())
}

/// Register Mac `index` for `op` in epoch `epoch`, answering all twelve slots of that epoch.
fn register(s: &mut WorldState, index: u64, op: Address, epoch: u64) {
    enroll(s, index, op, epoch);
    beacon(s, index, epoch);
}

/// Register Mac `index` for `op` in epoch `epoch`, answering nothing (asleep).
fn enroll(s: &mut WorldState, index: u64, op: Address, epoch: u64) {
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

/// Mac `index` answered all twelve slots of `epoch`.
fn beacon(s: &mut WorldState, index: u64, epoch: u64) {
    answer(s, index, epoch, EVERY_SLOT);
}

/// Mac `index` answered the slots in `mask` of `epoch`.
fn answer(s: &mut WorldState, index: u64, epoch: u64, mask: u64) {
    let b = beacons::beacon(s, index);
    beacons::put_beacon(s, index, beacons::Beacon { epoch, mask, ..b });
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
    // Whole schedule before the floor: ~31.5M × (1 − 0.1) / 0.1625 ≈ 175M DBLN,
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

#[test]
fn a_mac_that_answered_half_the_slots_gets_half_the_share() {
    let mut s = network();
    for i in 0..3 {
        register(&mut s, i, operator(i), 0);
        warm(&mut s, i, WARMUP_STEPS);
    }
    distribute(&mut s, EB).unwrap();
    answer(&mut s, 0, 1, EVERY_SLOT);
    answer(&mut s, 1, 1, HALF_SLOTS);
    answer(&mut s, 2, 1, (1 << (SLOTS / 4)) - 1);
    let before: Vec<U256> = (0..3).map(|i| s.balance(&operator(i))).collect();
    let d = distribute(&mut s, 2 * EB).unwrap();
    let got = |i: u64| s.balance(&operator(i)) - before[i as usize];
    assert_eq!(got(0), d.pool / U256::from(16u8));
    assert_eq!(got(1), d.pool / U256::from(32u8), "6 of 12 slots: half");
    assert_eq!(got(2), d.pool / U256::from(64u8), "3 of 12 slots: a quarter");
    // Answers of an earlier epoch count for nothing now.
    let d = distribute(&mut s, 3 * EB).unwrap();
    assert!(d.paid.is_empty());
}

/// Epochs long enough for real slots (twelve segments of 8 blocks).
const BEB: u64 = 100;

fn slot_network() -> WorldState {
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: BEB, min_streak: 0, draw_epochs: 1 }).unwrap();
    enable(&mut s);
    s
}

#[test]
fn each_slot_is_drawn_from_the_block_of_the_slot_before_it() {
    let (segment, window, span) = beacons::layout(BEB).unwrap();
    assert_eq!((segment, window, span), (8, 4, 3));
    assert_eq!(beacons::layout(3_600), Some((300, 90, 209)), "an hour: a 90-block answer window");
    assert!(beacons::layout(35).is_none(), "too short for twelve slots");
    assert_eq!(beacons::layout(36), Some((3, 1, 1)), "the shortest twelve slots fit");
    for seed in [[1u8; 32], [2; 32], [0xfe; 32]] {
        let mut prev = seed;
        let mut last = 7 * BEB - 1; // the block before epoch 7: slot 0's seed
        for k in 0..SLOTS {
            let h = beacons::slot_height(&prev, 7, k, BEB).unwrap();
            assert!(h > 7 * BEB + k * segment && h + window < 7 * BEB + (k + 1) * segment, "slot {k}: {h}");
            // Slot 0 lands at best two blocks after the epoch's last block, and
            // every later slot a full answer window after the slot before it:
            // nobody could have known this height before the seed block landed.
            assert!(h >= last + if k == 0 { 2 } else { window + 2 }, "slot {k} at {h}");
            last = h;
            prev = hash_of(h);
        }
    }
    assert_ne!(beacons::slot_height(&[1; 32], 7, 3, BEB), beacons::slot_height(&[2; 32], 7, 3, BEB), "the hash decides");
    assert_eq!(beacons::slot_height(&[1; 32], 7, 3, BEB), beacons::slot_height(&[1; 32], 7, 3, BEB), "the same for everyone");
}

fn hash_of(h: u64) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&h.to_be_bytes());
    b[31] = 1;
    b
}

/// The system writes of blocks `from..to` (block h's parent hash is `hash_of(h - 1)`).
fn blocks(s: &mut WorldState, from: u64, to: u64) {
    for h in from..to {
        if distributes(s, h) {
            distribute(s, h).unwrap();
        }
        beacons::on_block(s, h, hash_of(h - 1));
    }
}

#[test]
fn a_slot_opens_after_its_block_for_a_window_and_is_answered_once() {
    let mut s = slot_network();
    register(&mut s, 0, operator(0), 0);
    let c = registry::candidates(&s)[0].clone();
    let (_, window, _) = beacons::layout(BEB).unwrap();
    blocks(&mut s, 1, BEB + 1);
    let s0 = beacons::slot(&s, 0).expect("the epoch's first block drew it");
    assert_eq!(s0, beacons::slot_height(&hash_of(BEB - 1), 1, 0, BEB).unwrap(), "from the previous epoch's last block");
    assert_eq!(beacons::slot(&s, 1), None, "slot 1 waits for slot 0's block");
    blocks(&mut s, BEB + 1, s0 + 1);
    assert!(beacons::check(&s, s0 + 1, &c, 0).is_err(), "the slot's hash is recorded by the next block");
    blocks(&mut s, s0 + 1, s0 + 2);
    assert_eq!(beacons::slot_hash(&s, 0), Some(hash_of(s0)));
    assert_eq!(beacons::slot(&s, 1).map(|h| h >= s0 + 1 + window), Some(true), "drawn from slot 0's hash, after its window");
    let due = beacons::check(&s, s0 + 1, &c, 0).unwrap();
    assert_eq!((due.epoch, due.slot, due.hash, due.needs_attestation), (1, 0, hash_of(s0), false));
    assert_eq!(beacons::due(&s, s0 + 1, &c).len(), 1, "only the open slot");
    assert!(beacons::check(&s, s0 + 1 + window, &c, 0).is_err(), "the window closed");
    assert!(beacons::check(&s, s0 + 1, &c, 1).is_err(), "slot 1 opens later in the epoch");
    beacons::record(&mut s, &c, &due, false);
    assert!(beacons::check(&s, s0 + 2, &c, 0).unwrap_err().contains("already"));
    assert_eq!(beacons::beacon(&s, 0).answered(1), 1);
    // The registry liveness moved as the contract's beacon() would, without a transaction.
    let c1 = registry::candidates(&s)[0].clone();
    assert_eq!((c1.last_epoch, c1.streak, c1.missed), (1, 2, 0));
    // The next epoch: new slots, the old hashes gone.
    blocks(&mut s, s0 + 2, 2 * BEB + 1);
    assert!(beacons::slot_hash(&s, 0).is_none());
    assert_eq!(beacons::beacon(&s, 0).answered(2), 0);
}

/// The 2026-10-04 rehearsal ran 40-block epochs: `layout` gives each slot an
/// answer window of a single block, and a busy candidate Mac (a few hundred
/// milliseconds behind the validators' proposals) loses almost every answer
/// through no fault of its own — two Macs answered 11-12 of 12 slots an epoch
/// for twenty epochs and were paid for four. The mainnet's hour-long epochs
/// open a ninety-block window, so this is the rehearsal's shape, not the
/// network's; the test pins what the loss is: no catch-up, ever.
#[test]
fn an_answer_that_missed_its_window_pays_nothing_and_no_epoch_revisits_it() {
    // (segment, window, span): forty blocks fit twelve slots only just.
    assert_eq!(beacons::layout(40), Some((3, 1, 1)), "the rehearsal's one-block window");
    assert_eq!(beacons::layout(3_600), Some((300, 90, 209)), "the mainnet's ninety-block one");
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: 40, min_streak: 0, draw_epochs: 1 }).unwrap();
    enable(&mut s);
    enroll(&mut s, 0, operator(0), 0);
    let c = registry::candidates(&s)[0].clone();
    blocks(&mut s, 1, 41); // h 40: epoch 1 opens, slot 0 drawn at 41 (span 1: deterministic)
    blocks(&mut s, 41, 43); // h 42 records slot 0's hash and draws slot 1 at 44
    // The one in-time answer: block 42 is slot 0's whole window.
    let due = beacons::check(&s, 42, &c, 0).unwrap();
    beacons::record(&mut s, &c, &due, false);
    // Slot 1's window is block 45 alone; a Mac a block late finds it shut —
    // the exact refusal the rehearsal's candidate logs carry.
    blocks(&mut s, 43, 46);
    assert_eq!(beacons::check(&s, 46, &c, 1).unwrap_err(), "slot 1 is not open at height 46");
    // The epoch ends with ten more slots answered on time by nobody.
    blocks(&mut s, 46, 80);
    let d = distribute(&mut s, 80).unwrap();
    assert_eq!(d.epoch, 1);
    assert_eq!(beacons::beacon(&s, 0).answered(1), 1);
    let one_slot = d.pool * U256::from(WARMUP_STEPS) / U256::from(MAX_SHARE * FULL);
    assert_eq!(d.paid, vec![(operator(0), one_slot)], "one of twelve slots: pool·14/5376");
    assert_eq!(s.balance(&operator(0)), one_slot);
    assert_eq!(d.unminted, d.pool - one_slot, "the eleven missed slots' share is never minted");
    // The next epoch pays only its own answers: nothing comes back for epoch 1.
    blocks(&mut s, 81, 120);
    let d2 = distribute(&mut s, 120).unwrap();
    assert!(d2.paid.is_empty(), "no epoch revisits a missed one");
    assert_eq!(s.balance(&operator(0)), one_slot, "the balance never moves again");
}

#[test]
fn liveness_from_answers_follows_the_contracts_grace_rule() {
    let mut s = slot_network();
    register(&mut s, 0, operator(0), 0);
    let c = registry::candidates(&s)[0].clone();
    let due = |epoch| beacons::Due { epoch, slot: 0, hash: [1; 32], period: 0, needs_attestation: false };
    beacons::record(&mut s, &c, &due(3), false);
    let c1 = registry::candidates(&s)[0].clone();
    assert_eq!((c1.last_epoch, c1.streak, c1.missed), (3, 2, 2), "two epochs missed within the grace");
    beacons::record(&mut s, &c, &due(3), false);
    assert_eq!(registry::candidates(&s)[0].streak, 2, "once per epoch");
    beacons::record(&mut s, &c, &due(3 + beacons::GRACE_EPOCHS + 1), false);
    let c2 = registry::candidates(&s)[0].clone();
    assert_eq!((c2.streak, c2.missed), (1, 0), "past the grace the streak restarts");
}

#[test]
fn re_attestation_is_due_from_the_days_random_slot_after_the_registration_day() {
    let mut s = slot_network();
    register(&mut s, 0, operator(0), 0);
    let c = registry::candidates(&s)[0].clone();
    blocks(&mut s, 1, 2);
    let (_, re_e, re_s) = beacons::day(&s).unwrap();
    assert_eq!(beacons::reattest_slot(&hash_of(0)), (re_e, re_s));
    // Day 0 is periods 0 and 1, both covered by the registration.
    assert_eq!(beacons::period(&s, re_e, re_s), 1);
    // Day 1.
    let day1 = DAY_EPOCHS * BEB;
    blocks(&mut s, 2, day1 + 1);
    let (d, e1, k1) = beacons::day(&s).unwrap();
    assert_eq!(d, 1);
    let e = DAY_EPOCHS + e1;
    assert_eq!(beacons::period(&s, e, k1), 2);
    // Walk to day 1's re-attestation slot: from there an answer needs a re-attestation.
    // Each slot's height is only drawn once the block after the slot before it
    // lands, so the walk steps until slot `k1` is out.
    let mut h = e * BEB + 1;
    blocks(&mut s, day1 + 1, h); // epoch e's first block draws slot 0
    while beacons::slot(&s, k1).is_none() {
        blocks(&mut s, h, h + 1);
        h += 1;
    }
    let at = beacons::slot(&s, k1).unwrap();
    blocks(&mut s, h, at + 2);
    let due = beacons::check(&s, at + 1, &c, k1).unwrap();
    assert_eq!(due.period, 2);
    assert!(due.needs_attestation, "registered on day 0: covered through period 1 only");
    // An earlier slot of day 1 (if any) was still period 1: covered.
    if (e1, k1) > (0, 0) {
        assert_eq!(beacons::period(&s, DAY_EPOCHS, 0), 1);
    }
    beacons::record(&mut s, &c, &due, true);
    assert_eq!(beacons::beacon(&s, 0).attested, Some(2));
    // Re-attested: the rest of the period needs nothing more.
    if k1 + 1 < SLOTS {
        let h = beacons::slot(&s, k1 + 1).expect("drawn with slot k1's hash");
        blocks(&mut s, at + 2, h + 2);
        assert!(!beacons::check(&s, h + 1, &c, k1 + 1).unwrap().needs_attestation);
    }
}

#[test]
fn reserve_keys_are_a_genesis_parameter_of_at_most_three_and_earn_nothing() {
    let mut s = network();
    assert_eq!(reserve(&s), None);
    let founder = Address::repeat_byte(0xf0);
    let keys: Vec<([u8; 32], [u8; 32])> = (1..=3u8).map(|i| ([i; 32], [i + 10; 32])).collect();
    assert!(set_reserve(&mut s, founder, &[keys.clone(), keys[..1].to_vec()].concat()).is_err(), "four is too many");
    assert!(set_reserve(&mut s, founder, &[]).is_err());
    set_reserve(&mut s, founder, &keys).unwrap();
    assert_eq!(reserve(&s), Some((founder, keys.clone())));
    // Not registry candidates: no beacons, no node rewards.
    assert!(registry::candidates(&s).is_empty());
    let d = distribute(&mut s, EB).unwrap();
    assert!(d.paid.is_empty() && s.balance(&founder).is_zero());
    let mut off = WorldState::default();
    registry::predeploy(&mut off, ([1; 32], [2; 32]), Params::default()).unwrap();
    assert!(set_reserve(&mut off, founder, &keys).is_err(), "behind the node-rewards genesis flag");
}

/// A network with the founder's reserve keys set at genesis.
fn reserve_net(min_streak: u64) -> WorldState {
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: EB, min_streak, draw_epochs: 1 }).unwrap();
    enable(&mut s);
    set_reserve(&mut s, FOUNDER, &RESERVE_KEYS).unwrap();
    s
}

/// The founder operator and its genesis reserve keys.
const FOUNDER: Address = Address::new([0xf0; 20]);
const RESERVE_KEYS: [([u8; 32], [u8; 32]); 1] = [([0x51; 32], [0x52; 32])];

/// A committee whose members are the reserve keys (`seated` true) or plain
/// keys, as the carried handoff would name them: (voting key hex, node id).
fn committee(seated: bool) -> Vec<(String, String)> {
    if seated {
        RESERVE_KEYS.iter().map(|(k, n)| (hex::encode(k), hex::encode(n))).collect()
    } else {
        vec![(hex::encode([0x61; 32]), hex::encode([0x62; 32]))]
    }
}

/// The chain applies a committee change at `height` (chain.rs `pre_state_with`),
/// and — when the height opens an epoch — that boundary's overdue write too
/// (finding 6: here the ordinary case, the count at zero).
fn switch(s: &mut WorldState, height: u64, seated: bool) {
    switch_reserve(s, height, &committee(seated));
    if height.is_multiple_of(EB) {
        set_overdue(s, height / EB, 0);
    }
}

#[test]
fn reserve_service_pays_the_founder_as_if_its_mac_answered_every_slot() {
    // Two independent operators stay alive and the founder's warmed-up Mac
    // sleeps; a committee change seats the reserve keys before the epoch being
    // paid. Each served epoch then pays the founder exactly what its Mac would
    // have earned answering all twelve slots — here a sixteenth, counted as one
    // operator among the answering ones.
    let mut s = reserve_net(0);
    for i in 0..2 {
        register(&mut s, i, operator(i), 1);
        warm(&mut s, i, WARMUP_STEPS);
    }
    enroll(&mut s, 2, FOUNDER, 1);
    warm(&mut s, 2, WARMUP_STEPS);
    switch(&mut s, EB, true);
    assert_eq!(seated(&s), (RESERVE_KEYS.len() as u64, EB));
    let d = distribute(&mut s, 2 * EB).unwrap();
    assert_eq!(d.paid.len(), 3, "the founder counts as one operator");
    assert_eq!(s.balance(&FOUNDER), d.pool / U256::from(16u8), "the founder's full share");
    assert_eq!(s.balance(&operator(0)), d.pool / U256::from(16u8));
}

#[test]
fn reserve_service_follows_the_same_weight_rule_past_sixteen_operators() {
    // Twenty operators answer every slot with the reserve keys seated. With
    // the weight sum past 16 × FULL the pool is shared by weight and the
    // founder's credited weight is ruled like anyone's.
    let mut s = reserve_net(24);
    for i in 0..20 {
        register(&mut s, i, operator(i), 1);
        warm(&mut s, i, WARMUP_STEPS);
    }
    enroll(&mut s, 20, FOUNDER, 1);
    warm(&mut s, 20, WARMUP_STEPS);
    switch(&mut s, EB, true);
    let d = distribute(&mut s, 2 * EB).unwrap();
    assert_eq!(d.paid.len(), 21);
    assert!(21 * FULL > MAX_SHARE * FULL, "the weight sum passes the floor");
    let share = d.pool * U256::from(FULL) / U256::from(21 * FULL);
    assert_eq!(share, d.pool / U256::from(21u8));
    assert!(share < d.pool / U256::from(MAX_SHARE), "never past a sixteenth");
    assert_eq!(s.balance(&FOUNDER), share, "the founder by the same weight rule");
    assert_eq!(s.balance(&operator(7)), share);
}

#[test]
fn a_serving_founder_mac_is_not_paid_twice() {
    // The founder's own Macs answer everything while the reserve keys serve:
    // the credit changes nothing — an operator takes the max over its Macs,
    // not the sum, so it is one sixteenth, never two.
    let mut s = reserve_net(0);
    register(&mut s, 0, operator(0), 1);
    warm(&mut s, 0, WARMUP_STEPS);
    register(&mut s, 1, FOUNDER, 1);
    warm(&mut s, 1, WARMUP_STEPS);
    register(&mut s, 2, FOUNDER, 1); // a second, half-warm Mac of the founder
    warm(&mut s, 2, WARMUP_STEPS / 2);
    switch(&mut s, EB, true);
    let d = distribute(&mut s, 2 * EB).unwrap();
    assert_eq!(d.paid.len(), 2, "the founder is one operator");
    assert_eq!(s.balance(&FOUNDER), d.pool / U256::from(16u8), "one share, not two");
}

#[test]
fn the_credit_follows_the_seating_not_the_operator_count() {
    // The reserve keys serve only when the committee actually seats them: a
    // four-seat committee takes none even with two independent operators
    // (`rotation::with_reserve`), so without a seat there is no credit — and
    // with one, the operator count around it changes nothing.
    let mut s = reserve_net(0);
    for i in 0..2 {
        register(&mut s, i, operator(i), 1);
        warm(&mut s, i, WARMUP_STEPS);
    }
    enroll(&mut s, 9, FOUNDER, 1);
    warm(&mut s, 9, WARMUP_STEPS);
    set_overdue(&mut s, 1, 0);
    distribute(&mut s, 2 * EB).unwrap();
    assert!(s.balance(&FOUNDER).is_zero(), "no seat, no credit");
    switch(&mut s, 2 * EB, true);
    distribute(&mut s, 3 * EB).unwrap();
    assert_eq!(s.balance(&FOUNDER), node_pool(2, EB) / U256::from(16u8), "seated: the credit returns");
    // Two more independent operators stand, but the committee keeps its seat
    // for them: the credit stays — only an unseating ends it.
    for i in 2..4 {
        register(&mut s, i, operator(i), 2);
        warm(&mut s, i, WARMUP_STEPS);
    }
    for i in 0..5 {
        beacon(&mut s, if i == 4 { 9 } else { i }, 3);
    }
    set_overdue(&mut s, 3, 0);
    let d = distribute(&mut s, 4 * EB).unwrap();
    assert_eq!(d.paid.len(), 5, "five operators, the founder one of them");
    assert_eq!(s.balance(&FOUNDER) - node_pool(2, EB) / U256::from(16u8), d.pool / U256::from(16u8), "still seated, still credited");
}

#[test]
fn unseating_the_reserve_keys_ends_the_credit() {
    // A committee change that seats no reserve key clears the word, and the
    // next epoch pays the sleeping founder nothing — even one epoch later
    // than the switch, when nothing else changed.
    let mut s = reserve_net(0);
    register(&mut s, 0, operator(0), 1);
    warm(&mut s, 0, WARMUP_STEPS);
    enroll(&mut s, 9, FOUNDER, 1);
    warm(&mut s, 9, WARMUP_STEPS);
    switch(&mut s, EB, true);
    distribute(&mut s, 2 * EB).unwrap();
    let paid = s.balance(&FOUNDER);
    switch(&mut s, 2 * EB, false);
    assert_eq!(seated(&s), (0, 0), "the word is cleared");
    for e in 2..4 {
        beacon(&mut s, 0, e);
        set_overdue(&mut s, e, 0);
        distribute(&mut s, (e + 1) * EB).unwrap();
    }
    assert_eq!(s.balance(&FOUNDER), paid, "unseated: no credit for any epoch after");
}

#[test]
fn a_mid_epoch_seating_serves_only_from_the_next_epoch() {
    // The keys must hold the seat for the whole epoch: a switch landing inside
    // an epoch credits none of it, and the same seating credits the next one.
    // A switch at the epoch's first block is on time for that epoch.
    let mut s = reserve_net(0);
    register(&mut s, 0, operator(0), 0);
    warm(&mut s, 0, WARMUP_STEPS);
    enroll(&mut s, 1, FOUNDER, 0);
    warm(&mut s, 1, WARMUP_STEPS);
    switch(&mut s, 2 * EB + EB / 2, true); // inside epoch 2
    beacon(&mut s, 0, 2);
    set_overdue(&mut s, 2, 0);
    distribute(&mut s, 3 * EB).unwrap();
    assert!(s.balance(&FOUNDER).is_zero(), "epoch 2 was half-seated: no credit");
    beacon(&mut s, 0, 3);
    set_overdue(&mut s, 3, 0);
    distribute(&mut s, 4 * EB).unwrap();
    let after_full = s.balance(&FOUNDER);
    assert_eq!(after_full, node_pool(3, EB) / U256::from(16u8), "epoch 3 fully seated");
    // A break and a re-seating mid-epoch: no credit until a full epoch runs.
    switch(&mut s, 4 * EB + 1, false);
    switch(&mut s, 5 * EB + 1, true); // inside epoch 5
    beacon(&mut s, 0, 5);
    set_overdue(&mut s, 5, 0);
    distribute(&mut s, 6 * EB).unwrap();
    assert_eq!(s.balance(&FOUNDER), after_full, "the re-seating is mid-epoch 5: no credit");
    assert_eq!(seated(&s).1, 5 * EB + 1);
    // A switch that keeps a seat does not restart the height: unbroken service.
    switch(&mut s, 6 * EB, true);
    assert_eq!(seated(&s).1, 5 * EB + 1, "seat kept: the height stands");
}

#[test]
fn reserve_service_needs_the_founders_registered_mac() {
    // The credit is the founder's participation: without a registered Mac
    // there is nothing to participate with, and nothing is paid.
    let mut s = reserve_net(0);
    for i in 0..2 {
        register(&mut s, i, operator(i), 1);
    }
    switch(&mut s, EB, true);
    let d = distribute(&mut s, 2 * EB).unwrap();
    assert_eq!(d.paid.len(), 2);
    assert!(s.balance(&FOUNDER).is_zero());
}

#[test]
fn reserve_service_moves_no_warm_up() {
    // A served epoch is a weight, not an answer: the founder's day count and
    // warm-up level move only by its Mac's own slots. Half the day answered is
    // a bad day — the credit must not turn it into a good one — and the
    // credited share follows the level it has, never above a sixteenth.
    let mut s = reserve_net(24);
    register(&mut s, 0, operator(0), 0);
    warm(&mut s, 0, WARMUP_STEPS);
    enroll(&mut s, 1, FOUNDER, 0);
    warm(&mut s, 1, 5);
    switch(&mut s, 0, true);
    let mut shares = Vec::new();
    for day in 0..6u64 {
        for e in day * DAY_EPOCHS..(day + 1) * DAY_EPOCHS {
            answer(&mut s, 0, e, EVERY_SLOT);
            answer(&mut s, 1, e, HALF_SLOTS);
            set_overdue(&mut s, e, 0);
            let d = distribute(&mut s, (e + 1) * EB).unwrap();
            let level = 5u64.saturating_sub(day);
            let w = SLOTS * (WARMUP_STEPS + level);
            let got = d.paid.iter().find(|(op, _)| *op == FOUNDER).map(|(_, a)| *a).unwrap();
            assert_eq!(got, d.pool * U256::from(w) / U256::from(MAX_SHARE * FULL), "epoch {e} at level {level}");
            shares.push(got);
        }
        assert_eq!(mac(&s, 1).level, 5u64.saturating_sub(day + 1), "day {day}: a bad day steps down, the credit does not raise it");
        assert_eq!(mac(&s, 0).level, WARMUP_STEPS, "the answering Mac keeps its own good day");
    }
    let pool = node_pool(DAY_EPOCHS, EB);
    assert!(shares.iter().all(|a| *a <= pool / U256::from(16u8)));
    assert_eq!(s.balance(&FOUNDER), shares.into_iter().sum::<U256>(), "every served epoch paid");
}

#[test]
fn hour_profiles_track_when_each_mac_answers() {
    // Two days of history: Mac 0 always answers, Mac 1 only the first four
    // hours of each day. Each hour bucket is then exact — 1.0 or 0.0 —
    // because both EMAs move in lockstep once a full day has passed.
    let mut s = network();
    register(&mut s, 0, operator(0), 0);
    register(&mut s, 1, operator(1), 0);
    for e in 1..=2 * DAY_EPOCHS {
        answer(&mut s, 0, e, EVERY_SLOT);
        answer(&mut s, 1, e, if e % DAY_EPOCHS < 4 { EVERY_SLOT } else { 0 });
        distribute(&mut s, (e + 1) * EB).unwrap();
    }
    let p0 = beacons::profile(&s, 0);
    let p1 = beacons::profile(&s, 1);
    for h in 0..DAY_EPOCHS {
        assert_eq!(p0.at(h), Some(beacons::PROB_SCALE), "Mac 0 hour {h}");
        assert_eq!(p1.at(h), Some(u64::from(h < 4) * beacons::PROB_SCALE), "Mac 1 hour {h}");
    }
    assert_eq!((p0.overall(), p0.worst()), (Some(beacons::PROB_SCALE), Some(beacons::PROB_SCALE)));
    assert_eq!(p1.worst(), Some(0));
    assert_eq!(p1.overall(), Some(beacons::PROB_SCALE / 6));
}

#[test]
fn profiles_recover_from_zero_and_decay_to_it() {
    // One dark day, one always-on day, one dark day: the bucket climbs to a
    // half (a step, not a jump) and then decays — a Mac that comes back is
    // believed slowly, and one that stops is forgotten slowly.
    let mut s = network();
    register(&mut s, 0, operator(0), 0);
    let dark = |e: u64| (e % DAY_EPOCHS) >= 8 && (e % DAY_EPOCHS) < 16;
    for day in 0..3u64 {
        for e in day * DAY_EPOCHS + 1..=(day + 1) * DAY_EPOCHS {
            let up = match day {
                1 => true,        // the recovery day
                _ => !dark(e),    // dark at hours 8..15 otherwise
            };
            answer(&mut s, 0, e, if up { EVERY_SLOT } else { 0 });
            distribute(&mut s, (e + 1) * EB).unwrap();
        }
        let p = beacons::profile(&s, 0);
        assert_eq!(p.at(0), Some(beacons::PROB_SCALE), "an up hour stays exact on day {day}");
        // The dark bucket: 0/5 = 0, then 5/9, then 4/13.
        assert_eq!(p.at(8), Some([0, 5, 4][day as usize] * beacons::PROB_SCALE / [5, 9, 13][day as usize]), "day {day}");
    }
}

#[test]
fn the_genesis_write_parks_the_registry_sentinel_on_every_reserve_key() {
    // Finding 1: `set_reserve` prewrites the deployed registry's `indexOf`
    // sentinel for each reserve key, so its own `register()` — a direct
    // transaction, not one routed through the app — reverts Known().
    let s = reserve_net(0);
    for (key, _) in RESERVE_KEYS {
        let at = U256::from_be_bytes(
            keccak256([key.as_slice(), &U256::from(3u64).to_be_bytes::<32>()].concat()).0,
        );
        assert_eq!(s.storage(&registry::REGISTRY, at), U256::MAX, "the sentinel word");
    }
}

#[test]
fn committee_and_roster_words_round_trip_and_zero_their_tail() {
    // The words that keep the voting set in state (finding 2): a member list
    // reads back as written, and a shorter one zeroes the words a longer list
    // used, so no stale member can survive a rewrite.
    let id = |b: u8| aether_net::SecretKey::from_bytes(&[b; 32]).public().to_string();
    let long = vec![(hex::encode([0x11; 32]), id(1)), (hex::encode([0x22; 32]), id(2)), (hex::encode([0x33; 32]), id(3))];
    let short = vec![(hex::encode([0x44; 32]), id(4))];
    let mut s = reserve_net(0);
    set_committee(&mut s, &long).unwrap();
    assert_eq!(super::committee(&s), long);
    set_committee(&mut s, &short).unwrap();
    assert_eq!(super::committee(&s), short);
    assert_eq!(s.storage(&REWARDS, tagged(TAG_COMMITTEE, U256::from(5u64))), U256::ZERO, "a stale member word is zeroed");
    // Rosters and pools the same, with their draw tags; cleared is None.
    commit_roster(&mut s, 7, &long).unwrap();
    assert_eq!(next_roster(&s), Some((7, long.clone())));
    clear_roster(&mut s);
    assert_eq!(next_roster(&s), None);
    freeze_pool(&mut s, 9, &short).unwrap();
    assert_eq!(draw_pool(&s), Some((9, short.clone())));
    // A member that is not (key hex, node id) is refused; an empty list writes
    // nothing (a network without node rewards).
    assert!(set_committee(&mut s, &[("0".into(), id(4))]).is_err());
    set_committee(&mut s, &[]).unwrap();
    assert_eq!(super::committee(&s), short);
    assert!(super::committee(&network()).is_empty());
}

#[test]
fn same_roster_compares_members_not_order_or_spelling() {
    let id = |b: u8| aether_net::SecretKey::from_bytes(&[b; 32]).public().to_string();
    let a = [(hex::encode([1; 32]), id(1)), (hex::encode([2; 32]), id(2))];
    assert!(same_roster(&a, &[a[1].clone(), a[0].clone()]), "order does not matter");
    assert!(same_roster(&a, &[("0x".to_string() + &hex::encode([1; 32]), id(1)), (hex::encode([2; 32]).to_uppercase(), id(2))]), "nor spelling");
    assert!(!same_roster(&a, &[a[0].clone(), (hex::encode([3; 32]), id(3))]), "a different member");
    // A member that is not (key hex, node id) matches nothing: a handoff
    // carrying one is refused.
    assert!(!same_roster(&a, &[a[0].clone(), (hex::encode([2; 32]), "not-a-node-id".into())]));
    assert!(!same_roster(&a, &[a[0].clone(), ("00".into(), id(2))]));
    assert!(same_roster(&[], &[]));
}

#[test]
fn past_two_overdue_epochs_the_service_credit_stops() {
    // Finding 6: seats nobody needs — four or more independent operators
    // qualify while the reserve keys still sit — pay two more epochs and
    // stop. The word must be the epoch's own, so a stale or missing count
    // pays nothing rather than guessing.
    let mut s = reserve_net(0);
    switch(&mut s, EB, true);
    assert_eq!(overdue(&s), (1, 0));
    for (count, served) in [(0u64, true), (1, true), (2, true), (3, false), (9, false)] {
        set_overdue(&mut s, 2, count);
        assert_eq!(reserve_served(&s, 2).is_some(), served, "count {count}");
    }
    // A stale count — another epoch's word — never serves.
    set_overdue(&mut s, 2, 0);
    assert!(reserve_served(&s, 2).is_some());
    assert!(reserve_served(&s, 3).is_none(), "the word is epoch 2's, not 3's");
    assert_eq!(overdue(&s), (2, 0));
}

#[test]
fn unseating_and_credit_expiry_keep_every_reserve_key_registered() {
    let mut s = reserve_net(0);
    let registered = reserve(&s).unwrap();
    switch(&mut s, EB, true);
    set_overdue(&mut s, 2, RESERVE_GRACE_EPOCHS + 1);
    assert!(reserve_served(&s, 2).is_none(), "expiry stops only service credit");
    assert_eq!(reserve(&s), Some(registered.clone()));
    switch(&mut s, 3 * EB, false);
    assert_eq!(seated(&s), (0, 0));
    assert_eq!(reserve(&s), Some(registered), "unseated keys remain eligible standby");
}

#[test]
fn the_recent_word_keeps_the_last_two_epochs() {
    let mut s = network();
    register(&mut s, 0, operator(0), 0);
    assert_eq!(beacons::recent(&s, 0), None);
    answer(&mut s, 0, 1, EVERY_SLOT);
    distribute(&mut s, 2 * EB).unwrap();
    assert_eq!(beacons::recent(&s, 0), Some((1, 12, beacons::NO_COUNT)));
    answer(&mut s, 0, 2, 0b0001);
    distribute(&mut s, 3 * EB).unwrap();
    assert_eq!(beacons::recent(&s, 0), Some((2, 1, 12)));
    answer(&mut s, 0, 3, 0);
    distribute(&mut s, 4 * EB).unwrap();
    assert_eq!(beacons::recent(&s, 0), Some((3, 0, 1)));
    // An epoch with no distribution leaves a gap: unknown, never read as silence.
    answer(&mut s, 0, 5, EVERY_SLOT);
    distribute(&mut s, 6 * EB).unwrap();
    assert_eq!(beacons::recent(&s, 0), Some((5, 12, beacons::NO_COUNT)));
}

#[test]
fn every_rewards_storage_tag_is_distinct() {
    let mut tags = crate::ALL_TAGS.to_vec();
    tags.sort_unstable();
    tags.dedup();
    assert_eq!(tags.len(), crate::ALL_TAGS.len(), "two records share a storage tag");
}

#[test]
fn announced_night_keeps_warmup_and_recent_stability() {
    let mut s = network();
    registry_v3::genesis(&mut s).unwrap();
    enroll(&mut s, 0, operator(0), 0);
    enroll(&mut s, 1, operator(1), 0);
    warm(&mut s, 0, 3);
    warm(&mut s, 1, 3);
    for e in 1..2 * DAY_EPOCHS {
        let asleep = e % DAY_EPOCHS < 8;
        registry_v3::set_availability(&mut s, 0, e * EB, asleep);
        answer(&mut s, 0, e, if asleep { 0 } else { EVERY_SLOT });
        answer(&mut s, 1, e, EVERY_SLOT);
        distribute(&mut s, (e + 1) * EB).unwrap();
        let (up, no_drop) = beacons::stability(&s, 0, e + 1);
        assert!(no_drop, "announced hour {e} is not an unexpected drop");
        if e == DAY_EPOCHS { assert_eq!(up, 3); }
    }
    assert_eq!(mac(&s, 0).level, 5, "sleep does not lower warm-up");
    assert_eq!(beacons::announced_sleep(&s, 0), 0, "day count resets");

    let mut unannounced = network();
    registry_v3::genesis(&mut unannounced).unwrap();
    enroll(&mut unannounced, 0, operator(0), 0);
    enroll(&mut unannounced, 1, operator(1), 0);
    warm(&mut unannounced, 0, 3);
    for e in 1..2 * DAY_EPOCHS {
        answer(&mut unannounced, 0, e, if e % DAY_EPOCHS < 8 { 0 } else { EVERY_SLOT });
        answer(&mut unannounced, 1, e, EVERY_SLOT);
        distribute(&mut unannounced, (e + 1) * EB).unwrap();
    }
    assert_eq!(mac(&unannounced, 0).level, 1, "unannounced loss still costs warm-up");
}

#[test]
fn announced_long_sleep_preserves_streak_but_a_later_unannounced_gap_does_not() {
    let mut s = network();
    registry_v3::genesis(&mut s).unwrap();
    enroll(&mut s, 0, operator(0), 0);
    let c = beacons::candidate(&s, 0).unwrap();
    let due = |epoch| beacons::Due { epoch, slot: 0, hash: [1; 32], period: 0, needs_attestation: false };
    beacons::record(&mut s, &c, &due(1), false);
    registry_v3::set_availability(&mut s, 0, EB + 1, true);
    registry_v3::set_availability(&mut s, 0, 30 * EB, false);
    beacons::record(&mut s, &c, &due(30), false);
    let resumed = beacons::candidate(&s, 0).unwrap();
    assert_eq!((resumed.streak, resumed.missed), (3, 0));
    beacons::record(&mut s, &c, &due(60), false);
    assert_eq!(beacons::candidate(&s, 0).unwrap().streak, 1);
}
