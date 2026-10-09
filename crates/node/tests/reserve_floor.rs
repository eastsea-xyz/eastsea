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
        protocol: 4,
        reserve: Some(reserve.clone()),
        fees: false,
        committee: Some((0..committee).map(common::mac_entry).collect()),
    });
    (n, reserve, keys)
}

fn independents(n: &Net) -> usize {
    independents_at(n, n.parent.height / E)
}

fn independents_at(n: &Net, epoch: u64) -> usize {
    let r = Reserve::of(&n.parent.state).unwrap();
    let pool = rotation::eligible(&n.parent.state, epoch, 0);
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

// This value must be captured with the production reserve-floor rule reverted,
// then pinned before the old-rules fail-once run of the activation test. The
// transcript covers both reserve-bearing histories through FORK_HEIGHT - 1.
const EXPECTED_PRE_FOUR_RESERVE_TRANSCRIPT: &str = "BASELINE_PENDING";
const RESERVE_FORK_HEIGHT: u64 = 8 * E;

#[derive(Debug, PartialEq, Eq)]
struct ReserveForkFrame {
    encoded: Vec<u8>,
    state_root: aether_types::B256,
    metadata: aether_types::B256,
    committee: Vec<(String, String)>,
    roster: Option<(u64, Vec<(String, String)>)>,
    overdue: (u64, u64),
    seated: (u64, u64),
    payouts: Vec<(u64, Address, aether_types::U256)>,
}

impl ReserveForkFrame {
    fn of(block: &aether_node::block::Block, exec: &aether_node::chain::Executed) -> Self {
        Self {
            encoded: block.encode().to_vec(),
            state_root: exec.state.root(),
            metadata: exec.meta_digest(),
            committee: rewards::committee(&exec.state),
            roster: rewards::next_roster(&exec.state),
            overdue: rewards::overdue(&exec.state),
            seated: rewards::seated(&exec.state),
            payouts: exec.payouts.clone(),
        }
    }

    fn hash_into(&self, transcript: &mut blake3::Hasher) {
        transcript.update(&(self.encoded.len() as u64).to_be_bytes());
        transcript.update(&self.encoded);
        transcript.update(self.state_root.as_slice());
        transcript.update(self.metadata.as_slice());
        let words = serde_json::to_vec(&(
            &self.committee,
            &self.roster,
            self.overdue,
            self.seated,
            &self.payouts,
        ))
        .unwrap();
        transcript.update(&(words.len() as u64).to_be_bytes());
        transcript.update(&words);
    }
}

struct ReserveForkHistory {
    net: Net,
    reserve: Reserve,
    // False: four physical seats, one silent Mac; true: a genuinely seated
    // reserve whose credit expired under protocol 3 with four operators.
    credit: bool,
    running: Vec<(String, String)>,
    frames: Vec<ReserveForkFrame>,
}

impl ReserveForkHistory {
    fn record(&mut self) {
        self.frames
            .push(ReserveForkFrame::of(&self.net.last, &self.net.parent));
    }

    fn run_to(&mut self, height: u64) {
        while self.net.parent.height < height {
            self.net.step(vec![], None, vec![]);
            self.record();
        }
    }
}

fn reserve_protocol_three_history(credit: bool) -> ReserveForkHistory {
    let (reserve, keys) = reserve();
    let genesis: Vec<_> = common::Committee::genesis_members()
        .into_iter()
        .enumerate()
        .map(|(i, (key, _))| (key, common::mac_entry(i).1))
        .collect();
    let seats = if credit { 3 } else { 4 };
    let mut n = Net::new(Opts {
        chain_id: 7_799,
        node_rewards: true,
        epoch_blocks: E,
        macs: 4,
        min_streak: Some(0),
        history_v2: true,
        // Keep this explicit: a new binary does not change an existing chain's
        // protocol until the committee's signed activation reaches its height.
        protocol: 3,
        reserve: Some(reserve.clone()),
        fees: false,
        committee: Some(genesis[..seats].to_vec()),
    });
    // Register the actual genesis voting keys, so the recorded committee,
    // beacon records, and later emergency signers describe the same validators.
    n.voting = n.committee.keys.clone();
    n.chain.lock().protocol = 4;
    n.chain.finalize(&n.last).unwrap();
    let mut h = ReserveForkHistory {
        running: genesis[..seats].to_vec(),
        net: n,
        reserve,
        credit,
        frames: vec![],
    };
    h.record();
    let regs = (0..seats).map(|i| h.net.register(i)).collect();
    h.net.step(regs, None, vec![]);
    h.record();
    if credit {
        h.run_to(E);
        let (_, roster) = rewards::next_roster(&h.net.parent.state)
            .expect("the three-seat history commits its reserve seat");
        let (next, handoff) =
            h.net
                .committee
                .handoff_to(h.net.chain_id, 1, &common::seat_of(&h.net, &roster, &keys));
        h.net.committee = next;
        let carried = h.net.step_handoff(handoff);
        h.record();
        h.run_to(carried.height + aether_node::handoff::DELAY);
        h.running = roster;
        assert_eq!(rewards::committee(&h.net.parent.state), h.running);
        assert_eq!(rewards::seated(&h.net.parent.state).0, 1);
        // Register at the next epoch's first block, so its profile has no
        // partly served hour that would trigger the legacy survival rule.
        h.run_to(2 * E - 1);
        let reg = h.net.register(3);
        h.net.step(vec![reg], None, vec![]);
        h.record();
    }
    h.run_to(if credit { 5 * E } else { 4 * E });
    assert_eq!(independents(&h.net), 4);
    if !credit {
        h.net.behaviour.insert(3, Mac::Off);
    }
    // The upgrade lands at H-E-1: an actual block announces it more than one
    // registry epoch before H. It is not an unsigned schedule injected by the
    // test, and no genesis parameter is changed.
    h.run_to(RESERVE_FORK_HEIGHT - E - 2);
    let upgrade = aether_node::upgrade::Upgrade {
        chain_id: h.net.chain_id,
        protocol: 4,
        activate_at: RESERVE_FORK_HEIGHT,
        emergency: true,
        releases: vec![],
        notes: "reserve-floor activation regression".into(),
        registrar: None,
    };
    let committee = &h.net.committee;
    assert_eq!(committee.keys.len(), 4);
    let output = committee
        .files
        .values()
        .next()
        .unwrap()
        .decode(4)
        .unwrap()
        .0;
    let partials: Vec<_> = committee
        .keys
        .iter()
        .take(3)
        .map(|key| {
            let share = committee.files[&key.public_key()].decode(4).unwrap().1;
            aether_node::upgrade::sign_emergency_partial(&upgrade, &share, key)
        })
        .collect();
    let signed = aether_node::upgrade::combine(output.public(), &partials).unwrap();
    assert_eq!(signed.emergency_approvals.len(), 3);
    aether_node::upgrade::verify(&committee.identity(), &signed).unwrap();
    aether_node::upgrade::verify_emergency(&signed, &rewards::committee(&h.net.parent.state))
        .unwrap();
    h.net.step(vec![], Some(signed), vec![]);
    h.record();
    assert_eq!(h.net.parent.height, RESERVE_FORK_HEIGHT - E - 1);
    h.run_to(RESERVE_FORK_HEIGHT - 1);
    h
}

#[test]
fn reserve_floor_activates_only_at_the_signed_fork_and_replays_the_old_committee() {
    use commonware_codec::Decode as _;

    let mut histories = [
        reserve_protocol_three_history(false),
        reserve_protocol_three_history(true),
    ];
    let mut transcript = blake3::Hasher::new();
    transcript.update(b"aether-reserve-floor-protocol3-v1");
    for h in &histories {
        assert_eq!(h.net.parent.height, RESERVE_FORK_HEIGHT - 1);
        assert_eq!(
            aether_node::upgrade::protocol_at(&h.net.parent.schedule, h.net.parent.height),
            3,
        );
        assert_eq!(h.net.parent.next_protocol(), 4);
        assert_eq!(rewards::committee(&h.net.parent.state), h.running);
        // The range ends mid-epoch, after honest answers advanced last_epoch.
        // Query the upcoming boundary rather than reusing its previous epoch.
        let operators = independents_at(&h.net, RESERVE_FORK_HEIGHT / E);
        if h.credit {
            assert_eq!(operators, 4);
            assert_eq!(rewards::overdue(&h.net.parent.state).0, 7);
            assert!(rewards::overdue(&h.net.parent.state).1 > rewards::RESERVE_GRACE_EPOCHS);
            assert_eq!(rewards::reserve_served(&h.net.parent.state, 7), None);
            let (_, old_exit) = rewards::next_roster(&h.net.parent.state).unwrap();
            assert_eq!(old_exit.len(), 4);
            assert!(old_exit.iter().all(|m| !h.reserve.members.contains(m)));
        } else {
            assert_eq!(operators, 3);
            assert_eq!(rewards::seated(&h.net.parent.state).0, 0);
            assert_eq!(rewards::overdue(&h.net.parent.state), (7, 0));
            assert!(
                rewards::next_roster(&h.net.parent.state).is_none(),
                "protocol 3 keeps its old four-seat committee despite two silent epochs"
            );
        }
        transcript.update(&[u8::from(h.credit)]);
        transcript.update(&(h.frames.len() as u64).to_be_bytes());
        for frame in &h.frames {
            frame.hash_into(&mut transcript);
        }
    }
    let actual = transcript.finalize().to_hex().to_string();
    assert_eq!(
        actual, EXPECTED_PRE_FOUR_RESERVE_TRANSCRIPT,
        "pin this entire pre-4 reserve-bearing block range with the old production rules"
    );

    for h in &mut histories {
        let (replay, genesis) = aether_node::chain::Chain::new(h.net.chain.cfg());
        let mut replay_parent = replay.lock().finalized.clone();
        {
            let mut g = replay.lock();
            g.protocol = 4;
            g.identity = Some(common::Committee::genesis().identity());
        }
        assert_eq!(ReserveForkFrame::of(&genesis, &replay_parent), h.frames[0]);
        replay.finalize(&genesis).unwrap();
        // Decode and replay the original blocks, preserving every old state
        // root, metadata commitment, committee, roster, credit word and payout.
        for (height, expected) in h.frames.iter().enumerate().skip(1) {
            let block = aether_node::block::Block::decode_cfg(
                expected.encoded.as_slice(),
                &aether_node::block::Block::codec_config(8 << 20),
            )
            .unwrap();
            replay_parent = replay.execute(&block, &replay_parent).unwrap();
            replay.finalize(&block).unwrap();
            assert_eq!(
                ReserveForkFrame::of(&block, &replay_parent),
                *expected,
                "protocol-3 replay at height {height}, credit history={}",
                h.credit,
            );
        }
        let exec = h.net.step(vec![], None, vec![]);
        assert_eq!(exec.height, RESERVE_FORK_HEIGHT);
        assert_eq!(
            aether_node::upgrade::protocol_at(&exec.schedule, exec.height),
            4,
        );
        if h.credit {
            assert_eq!(independents(&h.net), 4);
            assert_eq!(
                rewards::overdue(&exec.state),
                (8, 0),
                "four operators stop the legacy expiry counter exactly at activation"
            );
            assert_eq!(
                rewards::reserve_served(&exec.state, 8),
                Some(h.reserve.operator)
            );
        } else {
            let (_, roster) = rewards::next_roster(&exec.state)
                .expect("the protocol-4 boundary repairs the legacy silent seat");
            assert_eq!(roster.len(), 4);
            assert!(!roster
                .iter()
                .any(|m| m.0 == hex::encode(h.net.voting_key(3))));
            assert_eq!(
                h.reserve
                    .members
                    .iter()
                    .filter(|m| roster.contains(m))
                    .count(),
                1,
                "a reserve replaces exactly the silent seat"
            );
            assert!(h.running[..3].iter().all(|m| roster.contains(m)));
        }
        assert_eq!(Reserve::of(&exec.state).unwrap().members, h.reserve.members);
        let replay_exec = replay.execute(&h.net.last, &replay_parent).unwrap();
        replay.finalize(&h.net.last).unwrap();
        assert_eq!(
            ReserveForkFrame::of(&h.net.last, &replay_exec),
            ReserveForkFrame::of(&h.net.last, &exec),
            "replay also agrees on the first protocol-4 block"
        );
    }
}

#[test]
fn a_delayed_seed_uses_standby_even_when_the_frozen_pool_still_lists_the_silent_seat() {
    for after_switch in [false, true] {
        let (mut n, reserve, keys) = net(4, 4);
        let regs = (0..4).map(|i| n.register(i)).collect();
        n.step(regs, None, vec![]);
        n.run_to(DAY_EPOCHS * E);
        let (draw, frozen) = rewards::draw_pool(&n.parent.state).unwrap();
        assert_eq!(draw, 1);
        assert_eq!(frozen.len(), 4);
        assert!(frozen.contains(&common::mac_entry(3)));
        n.behaviour.insert(3, Mac::Off);
        n.run_to((DAY_EPOCHS + 1) * E);
        assert!(rewards::next_roster(&n.parent.state).is_none());
        let drop = (DAY_EPOCHS + 2) * E;
        let repaired = if after_switch {
            n.run_to(drop);
            let (_, roster) = rewards::next_roster(&n.parent.state)
                .expect("the urgent boundary repairs the seat before a delayed seed");
            land(&mut n, &roster, &keys);
            assert!(rewards::next_roster(&n.parent.state).is_none());
            assert_eq!(rewards::committee(&n.parent.state), roster);
            Some(roster)
        } else {
            n.run_to(drop - 1);
            None
        };
        let seed = n.committee.sign_seed(n.chain_id, draw);
        let (block, exec) = n
            .build_extras(vec![], None, vec![], n.answers(), vec![], None, Some(seed))
            .unwrap();
        n.chain.finalize(&block).unwrap();
        n.parent = exec;
        n.last = block;
        assert_eq!(
            beacons::recent(&n.parent.state, 3),
            Some((DAY_EPOCHS + 1, 0, 0))
        );
        assert_eq!(rewards::draw_pool(&n.parent.state).unwrap().1, frozen);
        let roster =
            if let Some(repaired) = repaired {
                assert!(rewards::next_roster(&n.parent.state).is_none(),
                "a delayed seed cannot reinstall the still-silent former member after the switch");
                assert_eq!(rewards::committee(&n.parent.state), repaired);
                repaired
            } else {
                rewards::next_roster(&n.parent.state)
                    .expect("a delayed seed preserves the proven reserve repair")
                    .1
            };
        assert_eq!(roster.len(), 4);
        assert!(!roster.contains(&common::mac_entry(3)));
        assert!((0..3).all(|i| roster.contains(&common::mac_entry(i))));
        assert_eq!(
            reserve
                .members
                .iter()
                .filter(|m| roster.contains(m))
                .count(),
            1
        );
    }
    // Frozen healthy spares remain usable after answering this epoch: their
    // `last_epoch` has advanced, unlike a boundary-only eligibility query.
    let (mut n, reserve, _) = net(5, 4);
    let regs = (0..5).map(|i| n.register(i)).collect();
    n.step(regs, None, vec![]);
    n.run_to(DAY_EPOCHS * E + E / 2 + 1);
    // Choose a seed-only block: randomized beacon slots must not accidentally
    // keep the pre-state path awake and mask the idle-seed regression.
    while beacons::touches(&n.parent.state, n.parent.height + 1) || !n.answers().is_empty() {
        n.step(vec![], None, vec![]);
        assert_eq!(n.parent.height / E, DAY_EPOCHS);
    }
    assert!(!rewards::distributes(&n.parent.state, n.parent.height + 1));
    assert!(!(n.parent.height + 1).is_multiple_of(E));
    assert!(n.parent.handoff.is_none());
    assert_eq!(n.parent.statement, aether_node::chain::Statement::default());
    assert!(aether_execution::registry::candidates(&n.parent.state)
        .iter()
        .all(|c| c.last_epoch == DAY_EPOCHS));
    let (draw, frozen) = rewards::draw_pool(&n.parent.state).unwrap();
    assert_eq!(frozen.len(), 5);
    let seed = n.committee.sign_seed(n.chain_id, draw);
    let (block, exec) = n
        .build_extras(vec![], None, vec![], n.answers(), vec![], None, Some(seed))
        .unwrap();
    n.chain.finalize(&block).unwrap();
    n.parent = exec;
    n.last = block;
    let (_, grown) = rewards::next_roster(&n.parent.state)
        .expect("a live spare is not discarded because it already answered this epoch");
    assert_eq!(grown.len(), 5);
    assert!(grown.contains(&common::mac_entry(4)));
    assert!(grown.iter().all(|m| !reserve.members.contains(m)));
    assert_eq!(rewards::draw_pool(&n.parent.state).unwrap().1, frozen);
}

#[test]
fn a_seed_keeps_the_previous_draw_roster_while_its_handoff_is_pending() {
    let (mut n, _, keys) = net(5, 4);
    let regs = (0..5).map(|i| n.register(i)).collect();
    n.step(regs, None, vec![]);
    n.run_to((DAY_EPOCHS - 3) * E);
    n.behaviour.insert(3, Mac::Off);
    n.run_to((DAY_EPOCHS - 1) * E);
    let (old_draw, roster) = rewards::next_roster(&n.parent.state)
        .expect("a standby repair is committed before the draw freeze");
    assert_eq!(old_draw, 0);
    assert_eq!(roster.len(), 4);
    assert!(!roster.contains(&common::mac_entry(3)));
    assert!(
        roster.contains(&common::mac_entry(4)),
        "the pending roster seats the healthy spare"
    );
    // Carry a genuine handoff late enough that its switch crosses the next
    // draw boundary; the new draw must leave its existing commitment alone.
    let carried_height = DAY_EPOCHS * E - aether_node::handoff::DELAY / 2;
    n.run_to(carried_height - 1);
    let (next, handoff) =
        n.committee
            .handoff_to(n.chain_id, 1, &common::seat_of(&n, &roster, &keys));
    n.committee = next;
    let carried = n.step_handoff(handoff);
    let switch = carried.handoff.as_ref().unwrap().switch;
    assert!(switch > DAY_EPOCHS * E);
    n.run_to(DAY_EPOCHS * E - 1);
    let seed = n.committee.sign_seed(n.chain_id, 1);
    let (block, exec) = n
        .build_extras(vec![], None, vec![], n.answers(), vec![], None, Some(seed))
        .unwrap();
    n.chain.finalize(&block).unwrap();
    n.parent = exec;
    n.last = block;
    assert_eq!(rewards::draw_pool(&n.parent.state).unwrap().0, 1);
    assert!(rewards::draw_pool(&n.parent.state)
        .unwrap()
        .1
        .contains(&common::mac_entry(4)));
    assert_eq!(
        rewards::next_roster(&n.parent.state),
        Some((old_draw, roster.clone())),
        "a seed cannot commit another repair while the old handoff is pending"
    );
    n.run_to(switch);
    assert_eq!(rewards::committee(&n.parent.state), roster);
    assert!(rewards::next_roster(&n.parent.state).is_none());
}
