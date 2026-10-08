//! Founder reserves stay available through four independent operators. These
//! checks drive the state-derived roster and its real, signed handoff.

mod common;

use aether_node::chain::Reserve;
use aether_node::roster::{Member, NetworkFile, ReserveFile};
use aether_node::rotation;
use aether_rewards::{self as rewards, beacons, DAY_EPOCHS};
use aether_types::Address;
use common::{Mac, Net, Opts};
use commonware_codec::Encode as _;
use commonware_cryptography::{ed25519, Signer as _};

const E: u64 = 128;

fn reserve() -> (Reserve, Vec<ed25519::PrivateKey>) {
    let keys: Vec<_> = (101..=103).map(ed25519::PrivateKey::from_seed).collect();
    let members = keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let node = aether_net::SecretKey::from_bytes(&[0x71 + i as u8; 32]).public();
            (hex::encode(k.public_key().as_ref()), node.to_string())
        })
        .collect();
    (
        Reserve {
            operator: Address::repeat_byte(0xf0),
            members,
        },
        keys,
    )
}

fn net(macs: u8, committee: usize) -> (Net, Reserve, Vec<ed25519::PrivateKey>) {
    let (reserve, keys) = reserve();
    let n = Net::new(Opts {
        chain_id: 7_799,
        node_rewards: true,
        epoch_blocks: E,
        macs,
        min_streak: Some(0),
        history_v2: true,
        protocol: 3,
        reserve: Some(reserve.clone()),
        fees: false,
        committee: Some((0..committee).map(common::mac_entry).collect()),
    });
    (n, reserve, keys)
}

fn independents(n: &Net) -> usize {
    let r = Reserve::of(&n.parent.state).unwrap();
    let pool = rotation::eligible(&n.parent.state, n.parent.height / E, 0);
    let ops = rotation::operators(&n.parent.state);
    rotation::independent(&pool, |k| ops.get(k).cloned(), &r)
}

fn land(n: &mut Net, members: &[(String, String)], keys: &[ed25519::PrivateKey]) {
    let seats = common::seat_of(n, members, keys);
    let (committee, handoff) = n.committee.handoff_to(n.chain_id, 1, &seats);
    n.committee = committee;
    let carried = n.step_handoff(handoff);
    n.run_to(carried.height + aether_node::handoff::DELAY);
    assert_eq!(rewards::committee(&n.parent.state), members);
}

#[test]
fn four_operators_leave_every_reserve_registered_on_standby() {
    let (mut n, reserve, _) = net(4, 4);
    let regs = (0..4).map(|i| n.register(i)).collect();
    n.step(regs, None, vec![]);
    n.run_to(8 * E);
    assert_eq!(independents(&n), 4);
    assert_eq!(rewards::seated(&n.parent.state), (0, 0));
    assert!(rewards::next_roster(&n.parent.state).is_none());
    assert_eq!(
        Reserve::of(&n.parent.state).unwrap().members,
        reserve.members
    );
    assert_eq!(rewards::overdue(&n.parent.state), (8, 0));
}

#[test]
fn a_seated_reserve_network_file_keeps_the_original_genesis_and_standby_set() {
    let (n, reserve, keys) = net(4, 4);
    let member = |(key, node): (String, String)| Member { key, node };
    let genesis: Vec<_> = (0..4).map(common::mac_entry).map(member).collect();
    let mut file: NetworkFile = serde_json::from_value(serde_json::json!({
        "chain_id": 7_799,
        "validators": genesis,
        "genesis_validators": genesis,
        "history": 2,
        "protocol": 3,
        "node_rewards": true,
        "reserve": ReserveFile {
            operator: reserve.operator,
            validators: reserve.members.iter().cloned().map(member).collect(),
        },
    }))
    .unwrap();
    let original = file.genesis().unwrap();
    file.validators[3] = member(reserve.members[0].clone());
    let roster: Vec<_> = file
        .validators
        .iter()
        .map(|m| (m.key.clone(), m.node.clone()))
        .collect();
    let (_, handoff) = n
        .committee
        .handoff_to(n.chain_id, 1, &common::seat_of(&n, &roster, &keys));
    file.identity = Some(hex::encode(n.committee.identity().encode()));
    file.output = Some(handoff.output);
    file.round = 1;
    assert_eq!(
        file.genesis()
            .expect("a reserve handoff changes the current roster, not genesis"),
        original
    );

    let mut initial = file.clone();
    initial.round = 0;
    assert!(
        initial
            .genesis()
            .unwrap_err()
            .contains("its key is also a reserve key"),
        "a clean frozen list cannot hide an initial reserve seat"
    );
    initial.validators[3].key = genesis[3].key.clone();
    assert!(initial
        .genesis()
        .unwrap_err()
        .contains("its node id is also a reserve key's"));

    for (identity, output) in [
        (None, file.output.clone()),
        (file.identity.clone(), None),
        (None, None),
    ] {
        let mut retry = file.clone();
        retry.round = 7;
        retry.identity = identity;
        retry.output = output;
        assert!(
            retry
                .genesis()
                .unwrap_err()
                .contains("its key is also a reserve key"),
            "an incomplete initial DKG retry cannot hide a reserve seat"
        );
    }

    // Initial duplicate keys still fail, including an explicit genesis list.
    let mut bad = file.clone();
    bad.genesis_validators = Some(bad.validators.clone());
    assert!(bad
        .genesis()
        .unwrap_err()
        .contains("its key is also a reserve key"));
    bad.genesis_validators = None;
    assert!(bad
        .genesis()
        .unwrap_err()
        .contains("its key is also a reserve key"));
}

#[test]
fn a_silent_fourth_seat_is_filled_at_the_drop_boundary_including_draw_freezes() {
    for (off_epoch, with_seed) in [(6, false), (DAY_EPOCHS - 2, false), (DAY_EPOCHS - 2, true)] {
        let (mut n, reserve, keys) = net(4, 4);
        let regs = (0..4).map(|i| n.register(i)).collect();
        n.step(regs, None, vec![]);
        n.run_to(off_epoch * E);
        assert_eq!(independents(&n), 4);
        n.behaviour.insert(3, Mac::Off);
        n.run_to((off_epoch + 1) * E);
        assert!(
            rewards::next_roster(&n.parent.state).is_none(),
            "one silent epoch is insufficient"
        );
        let drop = (off_epoch + 2) * E;
        if with_seed {
            n.run_to(drop - 1);
            let seed = n.committee.sign_seed(n.chain_id, drop / (DAY_EPOCHS * E));
            let (block, exec) = n
                .build_extras(vec![], None, vec![], n.answers(), vec![], None, Some(seed))
                .unwrap();
            n.chain.finalize(&block).unwrap();
            n.parent = exec;
            n.last = block;
        } else {
            n.run_to(drop);
        }
        let recent = beacons::recent(&n.parent.state, 3);
        assert_eq!(recent, Some((off_epoch + 1, 0, 0)));
        let (_, roster) = rewards::next_roster(&n.parent.state)
            .expect("standby fills the silent seat at the next boundary");
        assert_eq!(
            roster.len(),
            4,
            "a reserve replaces a seat, never grows four to seven"
        );
        assert!((0..3).all(|i| roster.contains(&common::mac_entry(i))));
        assert!(!roster.contains(&common::mac_entry(3)));
        assert_eq!(
            reserve
                .members
                .iter()
                .filter(|m| roster.contains(m))
                .count(),
            1
        );
        land(&mut n, &roster, &keys);
        assert!(
            n.parent.height < drop + E,
            "the handoff completes within one epoch"
        );
        n.run_to(drop + 2 * E);
        assert_eq!(rewards::seated(&n.parent.state).0, 1);
        assert_eq!(
            Reserve::of(&n.parent.state).unwrap().members,
            reserve.members
        );
    }
}

#[test]
fn an_announced_fourth_seat_uses_standby_without_waiting_for_silence() {
    let (mut n, reserve, _) = net(4, 4);
    let regs = (0..4).map(|i| n.register(i)).collect();
    n.step(regs, None, vec![]);
    n.run_to(6 * E);
    let leaving = aether_node::beacons::sign_availability(
        &n.voting[3],
        n.chain_id,
        3,
        n.parent.height + 1,
        true,
    );
    let mut answers = n.answers();
    answers.push(leaving);
    n.step_with(vec![], None, vec![], answers);
    n.run_to(7 * E);
    let (_, roster) = rewards::next_roster(&n.parent.state)
        .expect("the announced seat can be filled immediately");
    assert_eq!(roster.len(), 4);
    assert!(!roster.contains(&common::mac_entry(3)));
    assert_eq!(
        reserve
            .members
            .iter()
            .filter(|m| roster.contains(m))
            .count(),
        1
    );
}

#[test]
fn the_credit_expiry_starts_at_five_and_never_erases_the_reserve_set() {
    let (mut n, reserve, keys) = net(5, 3);
    let regs = (0..3).map(|i| n.register(i)).collect();
    n.step(regs, None, vec![]);
    n.run_to(4 * E);
    let (_, roster) = rewards::next_roster(&n.parent.state).unwrap();
    land(&mut n, &roster, &keys);
    assert_eq!(rewards::seated(&n.parent.state).0, 1);

    let reg = n.register(3);
    n.step(vec![reg], None, vec![]);
    let four = n.parent.height / E + 4;
    n.run_to(four * E);
    assert_eq!(independents(&n), 4);
    for epoch in four..four + 5 {
        n.run_to(epoch * E);
        assert_eq!(
            rewards::overdue(&n.parent.state),
            (epoch, 0),
            "four operators cannot expire a reserve seat"
        );
        assert_eq!(
            rewards::reserve_served(&n.parent.state, epoch),
            Some(reserve.operator)
        );
        assert_eq!(
            Reserve::of(&n.parent.state).unwrap().members,
            reserve.members
        );
    }

    let reg = n.register(4);
    n.step(vec![reg], None, vec![]);
    let five = n.parent.height / E + 4;
    n.run_to(five * E);
    assert_eq!(independents(&n), 5);
    for count in 1..=3 {
        let epoch = five + count - 1;
        n.run_to(epoch * E);
        assert_eq!(rewards::overdue(&n.parent.state), (epoch, count));
        assert_eq!(
            rewards::reserve_served(&n.parent.state, epoch).is_some(),
            count <= rewards::RESERVE_GRACE_EPOCHS
        );
        assert_eq!(
            Reserve::of(&n.parent.state).unwrap().members,
            reserve.members
        );
    }

    n.behaviour.insert(4, Mac::Off);
    let back_to_four = n.parent.height / E + 1;
    n.run_to(back_to_four * E);
    assert_eq!(independents(&n), 4);
    assert_eq!(rewards::overdue(&n.parent.state), (back_to_four, 0));
    assert_eq!(
        rewards::reserve_served(&n.parent.state, back_to_four),
        Some(reserve.operator)
    );
    assert_eq!(
        Reserve::of(&n.parent.state).unwrap().members,
        reserve.members
    );
}
