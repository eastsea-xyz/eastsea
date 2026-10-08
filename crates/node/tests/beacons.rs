//! Beacon slots, re-attestation and founder reserve keys on a node-rewards
//! chain (docs/design/15-node-rewards.md, docs/design/12-launch-plan.md).
//! Five Macs: two answer every slot, one answers half of them, one tries to
//! fake answers, one never re-attests. Founder reserve keys fill only the
//! seats a committee is short of four and return to registered standby when
//! qualified Macs cover those seats.

mod common;

use aether_execution::registry;
use aether_node::chain::Reserve;
use aether_rewards::{beacons, node_pool, DAY_EPOCHS, FULL, MAX_SHARE, WARMUP_STEPS};
use aether_types::U256;
use commonware_cryptography::{ed25519, Signer as _};
use common::{answered, Mac, Net, Opts};

/// 48-block epochs (segments of four, a two-block answer window): the shortest
/// that leave a walkable gap between twelve slots.
const E: u64 = 48;

fn net(macs: u8, reserve: Option<Reserve>, committee: Option<Vec<(String, String)>>) -> Net {
    Net::new(Opts { chain_id: 7_792, node_rewards: true, epoch_blocks: E, macs, min_streak: Some(0), history_v2: false, protocol: 1, fees: false, reserve, committee })
}

/// What operator `i`'s Mac earns for `epoch` by the rule (one Mac per operator).
fn expected(n: &Net, index: u64, epoch: u64, total_weight: u64) -> U256 {
    let level = aether_rewards::mac(&n.parent.state, index).level;
    let w = answered(&n.parent.state, index, epoch) * (WARMUP_STEPS + level);
    node_pool(epoch, E) * U256::from(w) / U256::from(total_weight.max(MAX_SHARE * FULL))
}

#[test]
fn macs_are_paid_by_the_slots_they_answer_and_fakes_count_for_nothing() {
    let mut n = net(5, None, None);
    // 0, 1: every slot. 2: the first half of them. 3: fakes. 4: never re-attests.
    let regs = (0..5).map(|i| n.register(i)).collect();
    n.step_with(regs, None, vec![], vec![]);
    n.behaviour.insert(2, Mac::Slots(0b0000_0011_1111));
    n.behaviour.insert(3, Mac::Off);
    n.behaviour.insert(4, Mac::NoReattest);
    let nonces: Vec<u64> = (0..5).map(|i| n.parent.state.nonce(&n.operator(i))).collect();

    // Epoch 1: Mac 3 forges answers while the slots are open.
    let mut forged = 0;
    while n.parent.height < 2 * E - 1 {
        let honest = n.answers();
        if let Some(a) = n.answers_of(0).first() {
            // Mac 3 claims Mac 0's slot with its own key, and its own slot with a garbage signature.
            let view = n.view();
            let c0 = registry::candidates(&view)[0].clone();
            let due = beacons::check(&view, n.parent.height + 1, &c0, a.slot).unwrap();
            let steal = aether_node::beacons::sign(&n.voting[3], n.chain_id, 0, &due, None);
            let mut garbage = aether_node::beacons::sign(&n.voting[3], n.chain_id, 3, &due, None);
            garbage.signature = hex::encode([7u8; 64]);
            for fake in [steal, garbage] {
                assert!(n.chain.add_beacon(fake.clone()).is_err(), "the pool refuses a fake");
                let mut with_fake = honest.clone();
                with_fake.push(fake);
                assert!(n.build(vec![], None, vec![], with_fake).is_err(), "validators refuse a block with a fake");
                forged += 1;
            }
        }
        n.step_with(vec![], None, vec![], honest);
    }
    assert!(forged >= 8, "fakes were tried at every slot ({forged})");
    let state = n.parent.state.clone();
    assert_eq!((0..5).map(|i| answered(&state, i, 1)).collect::<Vec<_>>(), vec![12, 12, 6, 0, 12]);

    // Block 2E pays epoch 1 by answered slots: 12/12 → 1/32 (warm-up 0.5), 6/12 → 1/64, fake → 0.
    let before: Vec<U256> = (0..5).map(|i| n.balance(i)).collect();
    let paid = n.step(vec![], None, vec![]);
    let pool = node_pool(1, E);
    let got = |n: &Net, i: usize| n.balance(i) - before[i];
    assert_eq!(got(&n, 0), pool / U256::from(32u8));
    assert_eq!(got(&n, 1), pool / U256::from(32u8));
    assert_eq!(got(&n, 2), pool / U256::from(64u8));
    assert_eq!(got(&n, 3), U256::ZERO);
    assert_eq!(got(&n, 4), pool / U256::from(32u8), "covered by its registration until day 1");
    assert_eq!(paid.payouts.len(), 4);
    // Zero fee: answering sent no transaction; balances moved by rewards only.
    assert_eq!((0..5).map(|i| n.parent.state.nonce(&n.operator(i))).collect::<Vec<_>>(), nonces);
    // Answers also keep the registry liveness (no paid beacon() call): the draw pool sees them.
    let c = registry::candidates(&n.parent.state);
    assert_eq!((c[0].last_epoch, c[3].last_epoch), (1, 0));

    // Day 1: from its re-attestation slot, Mac 4 (no DeviceCheck token) cannot answer.
    n.run_to(DAY_EPOCHS * E + 1);
    let (day, re_epoch, re_slot) = beacons::day(&n.parent.state).unwrap();
    assert_eq!(day, 1);
    let re = DAY_EPOCHS + re_epoch;
    // The slot's height is only drawn once the block after the slot before it
    // lands, so walk block by block until it is out.
    n.run_to(re * E + 1);
    while beacons::slot(&n.parent.state, re_slot).is_none() {
        n.step(vec![], None, vec![]);
    }
    let at = beacons::slot(&n.parent.state, re_slot).unwrap();
    n.run_to(at);
    // Mac 4's answer without a re-attestation is not valid; with the registrar's it would be.
    let view = n.view();
    let c4 = registry::candidates(&view)[4].clone();
    let due = beacons::check(&view, at + 1, &c4, re_slot).unwrap();
    assert!(due.needs_attestation && due.period == 2);
    let bare = aether_node::beacons::sign(&n.voting[4], n.chain_id, 4, &due, None);
    assert!(n.chain.add_beacon(bare.clone()).unwrap_err().contains("re-attest"));
    assert!(n.build(vec![], None, vec![], vec![bare]).is_err());
    // An honest Mac re-attests at the same slot (the registrar's P-256 signature).
    let a0 = n.answers_of(0);
    assert!(a0.iter().any(|a| a.attest.as_ref().is_some_and(|r| r.period == 2)));
    // The next whole epoch: Mac 4 earns nothing (weight 0 until it re-attests), Mac 0 does.
    n.run_to((re + 2) * E - 1);
    let state = n.parent.state.clone();
    assert_eq!(answered(&state, 4, re + 1), 0);
    assert_eq!(answered(&state, 0, re + 1), 12);
    let total: u64 = (0..5u64)
        .map(|i| answered(&state, i, re + 1) * (WARMUP_STEPS + aether_rewards::mac(&state, i).level))
        .sum();
    let (b0, b4) = (n.balance(0), n.balance(4));
    let e0 = expected(&n, 0, re + 1, total);
    n.step(vec![], None, vec![]);
    assert_eq!(n.balance(4), b4, "failed re-attestation: weight 0");
    assert_eq!(n.balance(0) - b0, e0);
    assert!(aether_rewards::mac(&n.parent.state, 0).level >= 1, "a good day 0 moved Mac 0 up");
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

#[test]
fn founder_reserve_keys_fill_only_a_short_committee_and_keep_standby_at_four() {
    // Mac 4 is the founder's own registered Mac: not independent.
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(5)).unwrap());
    let reserve = Reserve { operator: founder, members: reserve_members() };
    let rkeys: Vec<ed25519::PrivateKey> = (1..=3u64).map(|i| ed25519::PrivateKey::from_seed(100 + i)).collect();
    let seated = |m: &[(String, String)]| reserve.members.iter().filter(|x| m.contains(x)).count();

    // Two independent Macs and the founder's register, but four seats already
    // stand at genesis: not one reserve key joins, and with no roster committed
    // no committee change may happen at all. Four must never grow to seven.
    {
        let mut n = net(5, Some(reserve.clone()), None);
        assert_eq!(n.operator(4), founder);
        assert_eq!(Reserve::of(&n.parent.state).expect("a genesis parameter").members, reserve.members);
        let regs = [0, 1, 4].iter().map(|i| n.register(*i)).collect();
        n.step_with(regs, None, vec![], vec![]);
        n.run_to(2 * E);
        assert!(aether_rewards::next_roster(&n.parent.state).is_none(), "four seats standing: no reserve key joins");
    }

    // The network opens short of four seats — the bootstrap the reserve keys
    // are for: the founder's Mac fills the third seat, one reserve key the
    // fourth, and only those two, as a committed roster (so only a handoff
    // naming exactly it may carry the committee over; see reserve_hardening.rs).
    let mut n = net(5, Some(reserve.clone()), Some(vec![common::mac_entry(0), common::mac_entry(1)]));
    let regs = [0, 1, 4].iter().map(|i| n.register(*i)).collect();
    n.step_with(regs, None, vec![], vec![]);
    n.run_to(E);
    let (_, roster) = aether_rewards::next_roster(&n.parent.state).expect("the missing seats are filled");
    assert_eq!(seated(&roster), 1, "only as many reserve keys as missing seats: {roster:?}");
    assert_eq!(roster.len(), 4);
    assert!([common::mac_entry(0), common::mac_entry(1), common::mac_entry(4)].iter().all(|m| roster.contains(m)), "the independents stay, the founder's Mac fills one seat");

    // Their handoff goes through; from the switch height that set votes.
    let (_, handoff) = n.committee.handoff_to(n.chain_id, 1, &common::seat_of(&n, &roster, &rkeys));
    let carried = n.step_handoff(handoff);
    let switch = carried.height + aether_node::handoff::DELAY;
    n.run_to(switch);
    assert_eq!(aether_rewards::committee(&n.parent.state), roster, "the switch records the committee");
    assert_eq!(aether_rewards::seated(&n.parent.state).0, 1);
    assert!(aether_rewards::next_roster(&n.parent.state).is_none(), "the roster it committed has served its purpose");

    // A third independent operator qualifies: the committee is at the rule's four seats, nothing changes.
    let reg = n.register(2);
    n.step_with(vec![reg], None, vec![], vec![]);
    n.run_to(5 * E);
    assert!(aether_rewards::next_roster(&n.parent.state).is_none(), "already the rule's seat count");
    // A fourth: qualified Macs cover every seat, so reserves return to standby.
    let reg = n.register(3);
    n.step_with(vec![reg], None, vec![], vec![]);
    n.run_to(6 * E);
    assert_eq!(aether_node::rotation::eligible(&n.parent.state, 6, 0).len(), 5);
    let (_, leave) = aether_rewards::next_roster(&n.parent.state).expect("reserve keys leave");
    assert_eq!(seated(&leave), 0, "every reserve key leaves at once");
    assert_eq!(leave.len(), 4, "qualifying Macs fill the seats");
    assert!([common::mac_entry(0), common::mac_entry(1), common::mac_entry(4)].iter().all(|m| leave.contains(m)));
    assert!(leave.contains(&common::mac_entry(2)) || leave.contains(&common::mac_entry(3)), "a qualifying Mac takes the freed seat");
    assert_eq!(Reserve::of(&n.parent.state).unwrap().members, reserve.members, "every reserve remains registered and eligible");
    // They earned nothing: not candidates, no beacons.
    assert!(registry::candidates(&n.parent.state).iter().all(|c| !reserve.members.iter().any(|(k, _)| *k == hex::encode(c.validator_key))));
}
