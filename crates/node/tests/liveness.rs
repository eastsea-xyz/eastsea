//! A committee that sleeps (docs/design/13-roadmap.md, F) on a node-rewards
//! chain: hour-of-day profiles accrue from beacon answers, a silent member is
//! replaced at an epoch boundary while the old quorum still stands, and the
//! founder reserve keys seat themselves through a risky night and step down
//! once the odds recover.

mod common;

use aether_node::chain::Reserve;
use aether_rewards::{beacons, DAY_EPOCHS};
use common::{Mac, Net, Opts};
use commonware_cryptography::{ed25519, Signer as _};

const E: u64 = 20;

fn net(macs: u8, reserve: Option<Reserve>) -> Net {
    Net::new(Opts { chain_id: 7_793, node_rewards: true, epoch_blocks: E, macs, min_streak: Some(0), history_v2: false, protocol: 1, reserve })
}

/// Mac `i`'s committee entry (voting key, iroh node id).
fn mac(n: &Net, i: usize) -> (String, String) {
    (hex::encode(n.voting_key(i)), aether_net::EndpointId::from_bytes(&Net::node_id(i)).unwrap().to_string())
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
fn hour_profiles_accrue_from_real_beacon_answers() {
    let mut n = net(2, None);
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
    let mut n = net(8, None);
    let regs = (0..8).map(|i| n.register(i)).collect();
    n.step_with(regs, None, vec![], vec![]);
    // Six seats (one swap per epoch: 6/3 − 1 = 1), two spares waiting.
    {
        let mut g = n.chain.lock();
        g.committee = aether_node::rotation::Committee { members: (0..6).map(|i| mac(&n, i)).collect() };
    }
    n.run_to(DAY_EPOCHS * E);
    assert!(n.chain.lock().proposal.is_none(), "an honest committee proposes nothing");

    // Macs 4 and 5 go dark. After one silent epoch nothing happens yet...
    n.behaviour.insert(4, Mac::Off);
    n.behaviour.insert(5, Mac::Off);
    n.run_to((DAY_EPOCHS + 1) * E);
    assert!(n.chain.lock().proposal.is_none(), "one silent epoch is not silence");
    // ...after two, one of them (the cap is one seat in six) hands over to a spare.
    n.run_to((DAY_EPOCHS + 2) * E);
    let proposal = n.chain.lock().proposal.clone().expect("a silent member is replaced");
    assert_eq!(proposal.1.len(), 6, "the committee keeps its size");
    let seated = |i: usize| proposal.1.contains(&mac(&n, i));
    assert_eq!([seated(4), seated(5)].iter().filter(|x| !**x).count(), 1, "exactly one silent member goes");
    assert!(seated(6) ^ seated(7), "exactly one spare takes the seat");
    assert!((0..4).all(seated), "the answering members stay");

    // The handoff lands; the next epoch replaces the other one — one per epoch.
    {
        let mut g = n.chain.lock();
        g.committee = aether_node::rotation::Committee { members: proposal.1.clone() };
        g.proposal = None;
    }
    n.run_to((DAY_EPOCHS + 3) * E);
    let proposal = n.chain.lock().proposal.clone().expect("the second silent member is replaced");
    assert_eq!(proposal.1.len(), 6);
    let seated = |i: usize| proposal.1.contains(&mac(&n, i));
    assert!(!seated(4) && !seated(5), "both silent members are out");
    assert!(seated(6) && seated(7), "both spares are in");
    assert!((0..4).all(seated));
}

#[test]
fn reserve_keys_seat_themselves_through_a_risky_night_and_step_down() {
    // The founder is no Mac's operator here: every seated Mac is independent.
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(50)).unwrap());
    let reserve = Reserve { operator: founder, members: reserve_members() };
    let mut n = net(8, Some(reserve.clone()));
    assert_eq!(Reserve::of(&n.parent.state).map(|r| r.members), Some(reserve.members.clone()));
    let regs = (0..8).map(|i| n.register(i)).collect();
    n.step_with(regs, None, vec![], vec![]);
    {
        let mut g = n.chain.lock();
        g.committee = aether_node::rotation::Committee { members: (0..6).map(|i| mac(&n, i)).collect() };
    }
    // A healthy day: six always-on seats carry every hour, no key is needed.
    n.run_to(DAY_EPOCHS * E);
    assert!(n.chain.lock().proposal.is_none(), "a healthy committee seats no key");

    // Macs 4 and 5 start sleeping through hours 16..23. After their profiles
    // learn it, the committee's worst hour (P(quorum) well under 0.99) is the
    // reserve keys' business: exactly one key is enough (with it, the four
    // always-on seats alone carry the quorum of seven).
    n.behaviour.insert(4, Mac::Awake(0x0000_FFFF));
    n.behaviour.insert(5, Mac::Awake(0x0000_FFFF));
    n.run_to((2 * DAY_EPOCHS + 1) * E);
    let proposal = n.chain.lock().proposal.clone().expect("a key seats itself");
    let keys = |m: &[(String, String)]| reserve.members.iter().filter(|x| m.contains(x)).count();
    assert_eq!(keys(&proposal.1), 1, "only as many keys as the odds need: {:?}", proposal.1);
    assert_eq!(proposal.1.len(), 7);
    assert!((0..6).all(|i| proposal.1.contains(&mac(&n, i))), "the committee stays seated");
    // The handoff lands; the next epoch the seating is stable (no flapping).
    {
        let mut g = n.chain.lock();
        g.committee = aether_node::rotation::Committee { members: proposal.1.clone() };
        g.proposal = None;
    }
    n.run_to((2 * DAY_EPOCHS + 2) * E);
    assert!(n.chain.lock().proposal.is_none(), "the same odds keep the same seating");

    // The sleeping seats are swapped for always-on ones (0..3, 6, 7), the key
    // staying seated: every hour is safe again and it steps down.
    {
        let mut g = n.chain.lock();
        let mut members = [0, 1, 2, 3, 6, 7].map(|i| mac(&n, i)).to_vec();
        members.push(proposal.1.iter().find(|m| reserve.members.contains(m)).unwrap().clone());
        g.committee = aether_node::rotation::Committee { members };
        g.proposal = None;
    }
    n.run_to((2 * DAY_EPOCHS + 3) * E);
    let stepped_down = n.chain.lock().proposal.clone().expect("the key steps down");
    assert_eq!(keys(&stepped_down.1), 0);
    assert_eq!(stepped_down.1.len(), 6);
}
