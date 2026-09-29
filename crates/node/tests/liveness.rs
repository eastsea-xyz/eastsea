//! A committee that sleeps (docs/design/13-roadmap.md, F) on a node-rewards
//! chain: hour-of-day profiles accrue from beacon answers, and every
//! committee change — an early replacement, a reserve key's night join or
//! step-down, a protocol-3 spread draw — commits the roster it decided in
//! state, so each test lands the handoff to exactly that roster and follows
//! the switch that records it. A handoff naming any other roster is refused.

mod common;

use aether_node::chain::Reserve;
use aether_rewards::{beacons, DAY_EPOCHS};
use common::{Mac, Net, Opts};
use commonware_cryptography::{ed25519, Signer as _};

/// 48-block epochs (segments of four, a two-block answer window): the shortest
/// that leave a walkable gap between twelve slots.
const E: u64 = 48;
/// One draw (epoch_blocks × draw_epochs, the registry's default 24 epochs).
const SPAN: u64 = E * DAY_EPOCHS;

fn net(macs: u8, reserve: Option<Reserve>, committee: Vec<(String, String)>) -> Net {
    Net::new(Opts {
        chain_id: 7_793,
        node_rewards: true,
        epoch_blocks: E,
        macs,
        min_streak: Some(0),
        history_v2: false,
        protocol: 1,
        reserve,
        committee: Some(committee),
    })
}

/// The roster the chain committed for the current draw, if any (a draw's own
/// word is what binds a handoff; a stale one from an earlier draw binds nothing).
fn roster(n: &Net) -> Option<Vec<(String, String)>> {
    let draw = n.parent.height / SPAN;
    aether_rewards::next_roster(&n.parent.state)
        .filter(|(d, _)| *d == draw)
        .map(|(_, members)| members)
}

/// The reserve keys' private keys (seed 100 + i, as `reserve_members` names them).
fn reserve_keys() -> Vec<ed25519::PrivateKey> {
    (1..=3u64).map(|i| ed25519::PrivateKey::from_seed(100 + i)).collect()
}

fn reserve_members() -> Vec<(String, String)> {
    reserve_keys()
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let node = aether_net::SecretKey::from_bytes(&[0x71 + i as u8; 32]).public();
            (hex::encode(k.public_key().as_ref()), node.to_string())
        })
        .collect()
}

/// Land the handoff to `roster` — the one the chain committed — and run to
/// its switch: the running committee word becomes the roster, the committed
/// roster is spent, and the next handoff signs from the new committee. A
/// test's landings must carry strictly rising rounds. Returns the switch
/// height: the first epoch boundary at or after it is open for the next
/// decision (a pending handoff stands in every earlier one's way).
fn land(n: &mut Net, members: &[(String, String)], reserve: &[ed25519::PrivateKey], round: u64) -> u64 {
    let seats = common::seat_of(n, members, reserve);
    let (committee, handoff) = n.committee.handoff_to(n.chain_id, round, &seats);
    n.committee = committee;
    let carried = n.step_handoff(handoff);
    let switch = carried.height + aether_node::handoff::DELAY;
    n.run_to(switch);
    assert_eq!(
        aether_rewards::committee(&n.parent.state),
        members,
        "the switch records the committee the handoff named"
    );
    assert!(roster(n).is_none(), "the committed roster is spent at the switch");
    switch
}

#[test]
fn hour_profiles_accrue_from_real_beacon_answers() {
    let mut n = net(2, None, (0..2).map(common::mac_entry).collect());
    let regs = (0..2).map(|i| n.register(i)).collect();
    n.step_with(regs, None, vec![], vec![]);
    // Mac 1 is up through hours 0..7 and asleep after; Mac 0 is always on.
    n.behaviour.insert(1, Mac::Awake(0x0000_00FF));
    // Two whole days: every hour bucket observed exactly twice, so the
    // answered/offered EMAs move in lockstep and the ratios are exact.
    n.run_to((2 * DAY_EPOCHS + 1) * E);
    let p0 = beacons::profile(&n.parent.state, 0);
    let p1 = beacons::profile(&n.parent.state, 1);
    for h in 0..DAY_EPOCHS {
        assert_eq!(p0.at(h), Some(beacons::PROB_SCALE), "the always-on Mac, hour {h}");
        assert_eq!(p1.at(h), Some(u64::from(h < 8) * beacons::PROB_SCALE), "the sleeping Mac, hour {h}");
    }
    assert_eq!((p0.overall(), p0.worst()), (Some(beacons::PROB_SCALE), Some(beacons::PROB_SCALE)));
    assert_eq!(p1.worst(), Some(0));
    assert_eq!(p1.overall(), Some(beacons::PROB_SCALE / 3));
}

#[test]
fn a_silent_member_is_replaced_while_the_quorum_still_stands() {
    // Six seats (one swap per epoch: 6/3 − 1 = 1) recorded in the genesis
    // committee word, two spares waiting.
    let mut n = net(8, None, (0..6).map(common::mac_entry).collect());
    let regs = (0..8).map(|i| n.register(i)).collect();
    n.step_with(regs, None, vec![], vec![]);
    n.run_to(DAY_EPOCHS * E);
    assert!(roster(&n).is_none(), "an honest committee commits nothing");

    // Macs 4 and 5 go dark. After one silent epoch nothing happens yet...
    n.behaviour.insert(4, Mac::Off);
    n.behaviour.insert(5, Mac::Off);
    n.run_to((DAY_EPOCHS + 1) * E);
    assert!(roster(&n).is_none(), "one silent epoch is not silence");
    // ...after two, one of them (the cap is one seat in six) is replaced by a
    // spare, and the chain commits that roster for the draw.
    n.run_to((DAY_EPOCHS + 2) * E);
    let first = roster(&n).expect("a silent member is replaced");
    assert_eq!(first.len(), 6, "the committee keeps its size");
    let seated = |m: &[(String, String)], i: usize| m.contains(&common::mac_entry(i));
    assert_eq!([seated(&first, 4), seated(&first, 5)].iter().filter(|x| !**x).count(), 1, "exactly one silent member goes");
    assert!(seated(&first, 6) ^ seated(&first, 7), "exactly one spare takes the seat");
    assert!((0..4).all(|i| seated(&first, i)), "the answering members stay");

    // Only a handoff naming that roster lands; it switches 64 blocks later (at
    // 585) and the next boundary it leaves open replaces the other one.
    let switched = land(&mut n, &first, &[], 1);
    n.run_to(switched - switched % E + E);
    let second = roster(&n).expect("the second silent member is replaced");
    assert_eq!(second.len(), 6);
    assert!(!seated(&second, 4) && !seated(&second, 5), "both silent members are out");
    assert!(seated(&second, 6) && seated(&second, 7), "both spares are in");
    assert!((0..4).all(|i| seated(&second, i)));
    land(&mut n, &second, &[], 2);
}

#[test]
fn reserve_keys_seat_themselves_through_a_risky_night_and_step_down() {
    // The founder is no Mac's operator here: every seated Mac is independent.
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(50)).unwrap());
    let rkeys = reserve_keys();
    let reserve = Reserve { operator: founder, members: reserve_members() };
    let mut n = net(8, Some(reserve.clone()), (0..6).map(common::mac_entry).collect());
    assert_eq!(Reserve::of(&n.parent.state).map(|r| r.members), Some(reserve.members.clone()));
    let regs = (0..8).map(|i| n.register(i)).collect();
    n.step_with(regs, None, vec![], vec![]);
    // A healthy day: six always-on seats carry every hour, no key is needed.
    n.run_to(DAY_EPOCHS * E);
    assert!(roster(&n).is_none(), "a healthy committee seats no key");

    // Macs 4 and 5 start sleeping through hours 16..23. Their first night
    // epoch is folded into the profiles at the next boundary: the committee's
    // worst hour (P(quorum) well under 0.99) becomes the reserve keys'
    // business, and exactly one key is enough — with it, the four always-on
    // seats alone carry the quorum of seven. The chain commits the roster and
    // only a handoff to it lands, recording the seating at its switch.
    n.behaviour.insert(4, Mac::Awake(0x0000_FFFF));
    n.behaviour.insert(5, Mac::Awake(0x0000_FFFF));
    n.run_to(41 * E); // the boundary after the first night epoch (hour 16)
    let joined = roster(&n).expect("a key seats itself");
    let keys = |m: &[(String, String)]| reserve.members.iter().filter(|x| m.contains(x)).count();
    assert_eq!(keys(&joined), 1, "only as many keys as the odds need: {joined:?}");
    assert_eq!(joined.len(), 7);
    assert!((0..6).all(|i| joined.contains(&common::mac_entry(i))), "the committee stays seated");
    let switched = land(&mut n, &joined, &rkeys, 1);
    assert_eq!(aether_rewards::seated(&n.parent.state), (1, switched), "the switch records the key's seat");

    // The sleeping Macs now go dark for good: the early-replacement rule
    // walks them out one per epoch while the key keeps its seat, and once
    // every seat is always on again the key steps down.
    n.behaviour.insert(4, Mac::Off);
    n.behaviour.insert(5, Mac::Off);
    n.run_to(45 * E); // the first open boundary after the switch
    let one_out = roster(&n).expect("the first silent member is replaced");
    assert_eq!(one_out.len(), 7);
    assert_eq!(keys(&one_out), 1, "the key keeps its seat");
    assert!(!one_out.contains(&common::mac_entry(4)) ^ !one_out.contains(&common::mac_entry(5)), "one sleeper goes");
    land(&mut n, &one_out, &rkeys, 2);

    n.run_to(49 * E);
    let both_out = roster(&n).expect("the second silent member is replaced");
    assert_eq!(both_out.len(), 7);
    assert_eq!(keys(&both_out), 1);
    assert!((0..4).chain(6..8).all(|i| both_out.contains(&common::mac_entry(i))), "the always-on Macs carry it");
    assert!(!both_out.contains(&common::mac_entry(4)) && !both_out.contains(&common::mac_entry(5)));
    land(&mut n, &both_out, &rkeys, 3);

    n.run_to(53 * E); // every seat always on: the worst hour is safe again
    let bare = roster(&n).expect("the key steps down");
    assert_eq!(keys(&bare), 0);
    assert_eq!(bare.len(), 6);
    land(&mut n, &bare, &rkeys, 4);
    assert_eq!(aether_rewards::seated(&n.parent.state), (0, 0), "the seating word clears with the last key");
}

#[test]
fn a_protocol3_spread_draw_hands_over_to_the_roster_it_commits() {
    // Four seats at genesis, six Macs registered: the spread draw's budget is
    // one seat (4 + budget ≤ 16, (4 − 1)/3 = 1), so exactly one of the two
    // waiting Macs joins where the (here all-equal) worst-hour odds tie —
    // the seed decides, and the chain commits the roster it drew.
    let mut n = net(6, None, (0..4).map(common::mac_entry).collect());
    n.chain.lock().protocol = 3; // this node runs protocol 3
    let upgrade = n.committee.sign_upgrade(&aether_node::upgrade::Upgrade {
        chain_id: n.chain_id,
        protocol: 3,
        activate_at: 64, // past the one-epoch notice (48 blocks) the chain demands
        releases: vec![aether_node::upgrade::Release {
            platform: "macos-arm64-dmg".into(),
            version: "0.6.0".into(),
            blake3: "ab".repeat(32),
            url: "https://x".into(),
        }],
        notes: String::new(),
        registrar: None,
    });
    let regs = (0..6).map(|i| n.register(i)).collect();
    n.step_with(regs, Some(upgrade), vec![], vec![]);
    // A day passes: the pool for draw 1 freezes at its first block, and the
    // committee signs the draw's seed.
    n.run_to(DAY_EPOCHS * E);
    let seed = n.committee.sign_seed(n.chain_id, 1);
    let (block, exec) = n.build_extras(vec![], None, vec![], n.answers(), None, Some(seed)).unwrap();
    n.chain.finalize(&block).unwrap();
    n.parent = exec.clone();
    n.last = block;
    let drawn = roster(&n).expect("the draw commits its roster");
    assert_eq!(drawn.len(), 5, "one seat is added: {drawn:?}");
    assert!((0..4).all(|i| drawn.contains(&common::mac_entry(i))), "the running committee stays");
    assert!(drawn.contains(&common::mac_entry(4)) ^ drawn.contains(&common::mac_entry(5)), "exactly one waiting Mac joins");

    // A handoff to any other roster is refused — swap the drawn seat for the
    // other waiting Mac and the block does not even execute.
    let other = (4..6).find(|i| !drawn.contains(&common::mac_entry(*i))).unwrap();
    let newcomer = 9 - other;
    let mut wrong: Vec<_> = drawn.iter().filter(|m| **m != common::mac_entry(newcomer)).cloned().collect();
    wrong.push(common::mac_entry(other));
    let (_, refused) = n.committee.handoff_to(n.chain_id, 1, &common::seat_of(&n, &wrong, &[]));
    match n.build_with(vec![], None, vec![], n.answers(), Some(refused)) {
        Err(aether_node::chain::ChainError::BadHandoff(_)) => {}
        Err(e) => panic!("a wrong roster's handoff failed another way: {e:?}"),
        Ok(_) => panic!("a handoff to another roster was accepted"),
    }

    // The handoff to the committed roster lands and switches the committee in.
    land(&mut n, &drawn, &[], 1);
}
