//! The read-only view behind `aether_rewardStatus`
//! (docs/design/15-node-rewards.md, "16분의 1인지 어떻게 아나" · "초기 참여자가
//! 알게 하기"): N operators of the last epoch, an operator's Macs, warm-up and
//! shares — on synthetic states, and on a real chain where the view's expected
//! share is what `distribute` actually paid. The count runs through the same
//! `rewards::operator_weights` `distribute` uses, so the view cannot disagree
//! with the chain (asserted over randomized states below).

mod common;

use aether_execution::registry::{self, Candidate, Params};
use aether_execution::WorldState;
use aether_node::rewards_view;
use aether_rewards as rewards;
use aether_rewards::beacons::{self, Beacon};
use aether_types::{Address, U256};
use alloy_primitives::B256;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha20Rng;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use common::{Net, Opts};

/// Epochs of ten blocks (short, as the rewards unit tests use).
const EB: u64 = 10;

/// A mask answering every slot of an epoch.
const EVERY_SLOT: u64 = (1 << rewards::SLOTS) - 1;

fn network() -> WorldState {
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
    rewards::enable(&mut s);
    s
}

fn operator(i: u64) -> Address {
    Address::from_word(U256::from(0x1000 + i).into())
}

/// One Mac: candidate `index` of `op`, warm-up `level`, its beacon record
/// showing `mask` slots answered of `epoch` and `attested` as its last
/// re-attestation period (registration covers only the first day's periods).
fn add_mac(s: &mut WorldState, index: u64, op: Address, level: u64, epoch: u64, mask: u64, attested: Option<u64>) {
    let c = Candidate {
        index,
        operator: op,
        validator_key: U256::from(index + 1).to_be_bytes(),
        node_id: [0; 32],
        beaconer: op,
        registered_epoch: 0,
        last_epoch: epoch,
        streak: 1,
        missed: 0,
    };
    rewards::put_candidate(s, &c);
    rewards::put_mac(s, index, rewards::Mac { level, ..Default::default() });
    beacons::put_beacon(s, index, Beacon { epoch, mask, attested });
}

/// `status` of `state` at `height`, pinned under the distribution block's
/// `root` (fixed within an epoch, as the RPC passes it; each test's own chain
/// and root keep the global pin from crossing tests).
fn status(s: &WorldState, chain: u64, height: u64, root: [u8; 32], operator: Option<Address>) -> Value {
    rewards_view::status(chain, s, height, Some(B256::from(root)), operator, None)
}

#[test]
fn one_or_four_operators_each_get_a_sixteenth_of_the_pool() {
    for (n, chain) in [(1u64, 0x92u64), (4, 0x93)] {
        let mut s = network();
        for i in 0..n {
            add_mac(&mut s, i, operator(i), rewards::WARMUP_STEPS, 0, EVERY_SLOT, None);
        }
        // The first block of epoch 1 (height EB) distributed epoch 0; no epoch-1
        // answer has landed at it, so the pin sees exactly what it paid.
        let v = status(&s, chain, EB, [chain as u8; 32], None);
        assert_eq!(v["enabled"], json!(true));
        assert_eq!(v["epoch"], json!(1));
        assert_eq!(v["epoch_blocks"], json!(EB));
        assert_eq!(v["slots_per_epoch"], json!(rewards::SLOTS));
        assert_eq!(v["operators_online_last_epoch"], json!(n));
        assert_eq!(v["max_share"], json!(rewards::MAX_SHARE));
        let pool = rewards::node_pool(0, EB);
        assert_eq!(v["node_pool_last_epoch"], json!(pool.to_string()));
        assert_eq!(v["issuance_per_block_now"], json!(rewards::issuance(EB).to_string()));

        let v = status(&s, chain, EB, [chain as u8; 32], Some(operator(0)));
        assert_eq!(v["operator"]["weight_last_epoch"], json!(rewards::FULL));
        assert_eq!(v["operator"]["expected_share_last_epoch"], json!((pool / U256::from(16u8)).to_string()));
        assert_eq!(v["operator"]["capped"], json!(true));
        let macs = v["operator"]["macs"].as_array().unwrap();
        assert_eq!(macs.len(), 1);
        assert_eq!(macs[0]["index"], json!(0));
        assert_eq!(macs[0]["answered_slots_last_epoch"], json!(rewards::SLOTS));
        assert_eq!(macs[0]["answered_slots_this_epoch"], json!(0));
        assert_eq!(macs[0]["warmup_level"], json!(rewards::WARMUP_STEPS));
        assert_eq!(macs[0]["warmup_percent"], json!(100));
    }
}

#[test]
fn from_sixteen_operators_on_the_pool_shares_by_weight() {
    let pool = rewards::node_pool(0, EB);
    // Twenty full weights: each a twentieth of the pool, nobody capped.
    let mut s = network();
    for i in 0..20 {
        add_mac(&mut s, i, operator(i), rewards::WARMUP_STEPS, 0, EVERY_SLOT, None);
    }
    let v = status(&s, 0x94, EB, [0x94; 32], Some(operator(0)));
    assert_eq!(v["operators_online_last_epoch"], json!(20));
    assert_eq!(v["operator"]["expected_share_last_epoch"], json!((pool / U256::from(20u8)).to_string()));
    assert_eq!(v["operator"]["capped"], json!(false));

    // Sixteen full weights: the floor 16 × FULL is exactly met, each gets 1/16
    // and the cap no longer binds. A half-warmed seventeenth Mac shifts the
    // split to pure proportions — its share, and everyone's, follows weights.
    let mut s = network();
    for i in 0..16 {
        add_mac(&mut s, i, operator(i), rewards::WARMUP_STEPS, 0, EVERY_SLOT, None);
    }
    add_mac(&mut s, 16, operator(16), 0, 0, EVERY_SLOT, None);
    let weights = rewards_view::epoch_weights(&s, 0);
    let total: u64 = weights.values().sum();
    assert_eq!(total, 16 * rewards::FULL + rewards::SLOTS * rewards::WARMUP_STEPS);
    for op in [operator(0), operator(16)] {
        let v = status(&s, 0x95, EB, [0x95; 32], Some(op));
        assert_eq!(v["operators_online_last_epoch"], json!(17));
        let w = weights[&op];
        let expect = pool * U256::from(w) / U256::from(total);
        assert_eq!(v["operator"]["expected_share_last_epoch"], json!(expect.to_string()), "{op}");
        assert_eq!(v["operator"]["capped"], json!(false), "{op}");
    }
    // The half-warmed Mac earns half of a full one's share, modulo the floor.
    let full = pool * U256::from(rewards::FULL) / U256::from(total);
    let half = pool * U256::from(rewards::SLOTS * rewards::WARMUP_STEPS) / U256::from(total);
    assert!(full - half * U256::from(2u8) <= U256::from(1u8));
}

#[test]
fn two_macs_of_one_operator_count_once() {
    let mut s = network();
    let op = operator(0);
    // One Mac answers half the slots; the operator still counts once, at its
    // better Mac's weight — extra Macs do not raise the 1/16 cap.
    add_mac(&mut s, 0, op, rewards::WARMUP_STEPS, 0, (1 << (rewards::SLOTS / 2)) - 1, None);
    add_mac(&mut s, 1, op, rewards::WARMUP_STEPS, 0, EVERY_SLOT, None);
    let pool = rewards::node_pool(0, EB);
    let v = status(&s, 0x96, EB, [0x96; 32], Some(op));
    assert_eq!(v["operators_online_last_epoch"], json!(1));
    assert_eq!(v["operator"]["weight_last_epoch"], json!(rewards::FULL));
    assert_eq!(v["operator"]["expected_share_last_epoch"], json!((pool / U256::from(16u8)).to_string()));
    assert_eq!(v["operator"]["capped"], json!(true));
    assert_eq!(v["operator"]["macs"].as_array().unwrap().len(), 2);
}

#[test]
fn warmup_percent_and_reattest_show_per_mac() {
    let mut s = network();
    let op = operator(0);
    // Day 2 (epoch 48): registration covers only the first day's periods, so a
    // re-attestation for period 1 no longer passes; epoch 48's answers have
    // begun, eating one Mac's epoch-47 record.
    add_mac(&mut s, 0, op, 0, 47, EVERY_SLOT, Some(1));
    add_mac(&mut s, 1, op, 7, 48, 0b0010, Some(2));
    add_mac(&mut s, 2, op, rewards::WARMUP_STEPS, 47, EVERY_SLOT, Some(2));
    let pool = rewards::node_pool(47, EB);
    let v = status(&s, 0x97, 48 * EB, [0x97; 32], Some(op));
    let macs = v["operator"]["macs"].as_array().unwrap();
    assert_eq!(macs[0]["warmup_level"], json!(0));
    assert_eq!(macs[0]["warmup_percent"], json!(50));
    assert_eq!(macs[1]["warmup_percent"], json!(75));
    assert_eq!(macs[2]["warmup_percent"], json!(100));
    assert_eq!(macs[0]["attested_period"], json!(1));
    assert_eq!(macs[0]["reattest_ok"], json!(false));
    assert_eq!(macs[1]["reattest_ok"], json!(true));
    assert_eq!(macs[2]["reattest_ok"], json!(true));
    // This epoch against the last, and a record the new epoch already ate.
    assert_eq!(macs[0]["answered_slots_this_epoch"], json!(0));
    assert_eq!(macs[0]["answered_slots_last_epoch"], json!(rewards::SLOTS));
    assert_eq!(macs[1]["answered_slots_this_epoch"], json!(1));
    assert_eq!(macs[1]["answered_slots_last_epoch"], json!(null));
    // The operator's weight is the max over its Macs: the two full-slot ones.
    assert_eq!(v["operator"]["weight_last_epoch"], json!(rewards::FULL));
    assert_eq!(v["operator"]["expected_share_last_epoch"], json!((pool / U256::from(16u8)).to_string()));
}

#[test]
fn a_network_without_node_rewards_answers_enabled_false_only() {
    let mut s = WorldState::default();
    registry::predeploy(&mut s, ([1; 32], [2; 32]), Params { epoch_blocks: EB, min_streak: 0, draw_epochs: 1 }).unwrap();
    let v = rewards_view::status(0x98, &s, 5 * EB, None, Some(operator(0)), Some(U256::from(1u8)));
    assert_eq!(v, json!({ "enabled": false }));
}

#[test]
fn the_first_sight_of_an_epoch_stays_pinned() {
    let mut s = network();
    for i in 0..4 {
        add_mac(&mut s, i, operator(i), rewards::WARMUP_STEPS, 0, EVERY_SLOT, None);
    }
    let root = [0x99; 32];
    let v = status(&s, 0x99, EB, root, None);
    assert_eq!(v["operators_online_last_epoch"], json!(4));
    // The new epoch's answers erase epoch 0's records — the pin keeps the count
    // the distribution used; without it every Mac would now read zero.
    for i in 0..4 {
        add_mac(&mut s, i, operator(i), rewards::WARMUP_STEPS, 1, 0, None);
    }
    let v = status(&s, 0x99, EB + 3, root, Some(operator(0)));
    assert_eq!(v["operators_online_last_epoch"], json!(4));
    assert_eq!(v["operator"]["weight_last_epoch"], json!(rewards::FULL));
    assert_eq!(v["operator"]["expected_share_last_epoch"], json!((rewards::node_pool(0, EB) / U256::from(16u8)).to_string()));
    // The live parts still move with the chain: this epoch, nothing yet.
    assert_eq!(v["operator"]["macs"][0]["answered_slots_this_epoch"], json!(0));
    assert_eq!(v["operator"]["macs"][0]["answered_slots_last_epoch"], json!(rewards::SLOTS));
}

/// The view's weights are `distribute`'s payments, over randomized states:
/// random Mac counts over random operator counts, random warm-up levels,
/// answered-slot masks and record epochs.
#[test]
fn the_view_counts_exactly_what_distribute_pays_on_random_states() {
    let mut rng = ChaCha20Rng::seed_from_u64(0xae7e_1500);
    for round in 0..64 {
        let epoch = 1 + round % 5;
        let mut s = network();
        let operators = rng.random_range(1..=7u64);
        for index in 0..rng.random_range(0..=24u64) {
            let op = operator(rng.random_range(0..operators));
            let level = rng.random_range(0..=rewards::WARMUP_STEPS);
            let mask = rng.random_range(0..(1u64 << rewards::SLOTS));
            add_mac(&mut s, index, op, level, epoch, mask, None);
        }
        // What the view says about the epoch, taken before the distribution runs.
        let weights = rewards_view::epoch_weights(&s, epoch);
        let total: u64 = weights.values().sum();
        let d = rewards::distribute(&mut s, (epoch + 1) * EB).unwrap();
        assert_eq!(d.epoch, epoch);
        assert_eq!(d.pool, rewards::node_pool(epoch, EB));
        let paid: BTreeMap<Address, U256> = d.paid.iter().cloned().collect();
        assert_eq!(paid.len(), weights.len(), "round {round}: the same operators");
        for (op, w) in &weights {
            let expect = d.pool * U256::from(*w) / U256::from(total.max(rewards::MAX_SHARE * rewards::FULL));
            assert_eq!(paid.get(op), Some(&expect), "round {round}: {op}");
        }
    }
}

#[test]
fn received_comes_from_the_newest_node_record_of_that_distribution() {
    let record = |kind: &str, height: u64, amount: u64| {
        json!({ "kind": kind, "proven": height, "amount": amount.to_string(), "height": height, "timestamp_ms": 0 })
    };
    // Proof records land after the node one: the newest node record still decides.
    let records = [record("node", 12, 5), record("proof", 13, 1), record("proof", 14, 1)];
    assert_eq!(rewards_view::received_from_records(&records, 12), Some(U256::from(5u8)));
    // Paid nothing at 13: its newest node record is an older distribution's.
    assert_eq!(rewards_view::received_from_records(&records, 13), None);
    // A newer distribution replaces it.
    let records = [record("node", 12, 5), record("node", 24, 7), record("proof", 25, 1)];
    assert_eq!(rewards_view::received_from_records(&records, 24), Some(U256::from(7u8)));
    assert_eq!(rewards_view::received_from_records(&records, 12), None);
    assert_eq!(rewards_view::received_from_records(&[], 24), None);
}

#[test]
fn on_a_real_chain_the_view_shows_the_last_epoch_and_the_actual_payout() {
    // 48-block epochs (segments of four, a two-block answer window) so twelve
    // slots fit: the first two sit at heights 1 and 5, their hashes recorded by
    // blocks 2 and 6.
    const E: u64 = 48;
    let mut net = Net::new(Opts { chain_id: 0x9a, node_rewards: true, epoch_blocks: E, macs: 4, min_streak: None, history_v2: false, protocol: 1, reserve: None, committee: None });
    let regs = (0..4).map(|i| net.register(i)).collect();
    net.step(regs, None, vec![]);
    // Epoch 0, two of its slots in: counted live.
    net.run_to(6);
    let f = &net.parent;
    let v = rewards_view::status(0x9a, &f.state, f.height, Some(f.state.root()), Some(net.operator(0)), None);
    assert_eq!(v["epoch"], json!(0));
    assert_eq!(v["operators_online_last_epoch"], json!(4));
    assert_eq!(v["operator"]["macs"][0]["answered_slots_this_epoch"], json!(2));

    // Block 2E distributes epoch 1: the view's expected share is what it paid.
    net.run_to(2 * E - 1);
    let before = net.balance(0);
    net.step(vec![], None, vec![]);
    assert_eq!(net.parent.height, 2 * E);
    let f = &net.parent;
    let v = rewards_view::status(0x9a, &f.state, f.height, Some(f.state.root()), Some(net.operator(0)), None);
    let pool = rewards::node_pool(1, E);
    assert_eq!(v["epoch"], json!(2));
    assert_eq!(v["operators_online_last_epoch"], json!(4));
    assert_eq!(v["node_pool_last_epoch"], json!(pool.to_string()));
    // New Macs: warm-up 0.5, all twelve slots answered — 1/32 each, held by the cap.
    let paid = net.balance(0) - before;
    assert_eq!(paid, pool / U256::from(32u8));
    assert_eq!(v["operator"]["expected_share_last_epoch"], json!(paid.to_string()));
    assert_eq!(v["operator"]["capped"], json!(true));
    assert_eq!(v["operator"]["received_last_distribution"], json!(null), "a memory-only net keeps no reward records");
    let mac = &v["operator"]["macs"][0];
    assert_eq!(mac["answered_slots_last_epoch"], json!(rewards::SLOTS));
    assert_eq!(mac["answered_slots_this_epoch"], json!(0));
    assert_eq!(mac["warmup_percent"], json!(50));
    assert_eq!(mac["reattest_ok"], json!(true));
}

#[test]
fn the_testnet_rules_answer_enabled_false() {
    let mut net = Net::new(Opts { chain_id: 7780, node_rewards: false, epoch_blocks: 12, macs: 1, min_streak: None, history_v2: false, protocol: 1, reserve: None, committee: None });
    net.step(vec![], None, vec![]);
    let f = &net.parent;
    let v = rewards_view::status(7780, &f.state, f.height, None, None, None);
    assert_eq!(v, json!({ "enabled": false }));
}
