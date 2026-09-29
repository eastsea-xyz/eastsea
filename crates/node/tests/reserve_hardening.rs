//! The reserve hardening of 2026-09-29, end to end (docs/design/15-node-rewards.md
//! "남은 일", docs/design/12-launch-plan.md): a reserve key cannot register as
//! a candidate even by a direct transaction (finding 1), a committee handoff
//! binds to the roster the chain committed for the draw (finding 2), a network
//! file never seats a validator that is also a reserve key (finding 3), and
//! seats nobody needs stop the service credit with a warning in the log
//! (finding 6). A network without node rewards keeps accepting
//! committee-signed handoffs with no roster anywhere — the testnet's rules
//! never change.

mod common;

use aether_execution::registry;
use aether_node::chain::{ChainError, Reserve};
use aether_node::roster::{Member, NetworkFile, ReserveFile};
use aether_types::Address;
use common::{mac_entry, Net, Opts};
use commonware_cryptography::{ed25519, Signer as _};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

const CHAIN: u64 = 7_793;
/// 36-block epochs (twelve slots of three blocks, a one-block answer window):
/// the shortest twelve beacon slots fit with room to answer.
const E: u64 = 36;
/// One draw: 24 short epochs, as the default `draw_epochs` gives.
const SPAN: u64 = E * 24;

/// A log the overdue warning (finding 6) lands in: the harness drives the
/// chain on the test's own thread, so a thread-local subscriber sees it.
#[derive(Clone)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Log {
    type Writer = Log;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn net(reserve: Option<Reserve>, committee: Option<Vec<(String, String)>>) -> Net {
    Net::new(Opts { chain_id: CHAIN, node_rewards: true, epoch_blocks: E, macs: 4, min_streak: Some(0), history_v2: false, reserve, fees: false, committee })
}

/// Build the next block carrying `handoff`: it must be refused, as a bad
/// handoff (not on any other ground), and the refusal is the answer.
fn refused(n: &Net, handoff: aether_light::block::Handoff) -> ChainError {
    let err = n.build_with(vec![], None, vec![], vec![], vec![], Some(handoff)).err().expect("the block is refused");
    assert!(matches!(err, ChainError::BadHandoff(_)), "not a handoff refusal: {err:?}");
    err
}

/// The founder's address and the reserve keys: each one's ed25519 key and its
/// (key hex, iroh node id) pair, as the genesis `--reserve` line names them.
fn reserve_set() -> (Address, Vec<ed25519::PrivateKey>, Vec<(String, String)>) {
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(5)).unwrap());
    let set: Vec<_> = (1..=3u64)
        .map(|i| {
            let k = ed25519::PrivateKey::from_seed(100 + i);
            let pk = k.public_key();
            let node = aether_net::SecretKey::from_bytes(&[0x70 + i as u8; 32]).public();
            (k, (hex::encode(pk.as_ref()), node.to_string()))
        })
        .collect();
    let (keys, members): (Vec<_>, Vec<_>) = set.into_iter().unzip();
    (founder, keys, members)
}

#[test]
fn a_reserve_key_cannot_register_as_a_candidate() {
    // Finding 1: the genesis storage write parks the registry's `indexOf`
    // sentinel on every reserve key, so the deployed contract's own
    // `register()` — reached here by a direct transaction carrying a valid
    // attestation, not one the app built — refuses it.
    let (founder, rkeys, rmembers) = reserve_set();
    let mut n = net(Some(Reserve { operator: founder, members: rmembers }), None);
    let node = *aether_net::SecretKey::from_bytes(&[0x71; 32]).public().as_bytes();
    let rogue = n.register_raw(0, rkeys[0].public_key().as_ref().try_into().unwrap(), node);
    let (_, exec) = n.build_extras(vec![rogue], None, vec![], vec![], vec![], None, None).unwrap();
    assert!(!exec.receipts[0].success, "the registry itself reverts a reserve key");
    assert!(registry::candidates(&exec.state).is_empty(), "nothing registered");
    // An honest registration through the same door still goes through.
    let ok = n.register(1);
    n.step(vec![ok], None, vec![]);
    assert_eq!(registry::candidates(&n.parent.state).len(), 1);
}

#[test]
fn a_handoff_binds_to_the_roster_the_chain_committed() {
    let (founder, rkeys, rmembers) = reserve_set();
    // A committee already at four seats, and no Mac registered: whatever a
    // handoff names, the chain committed no roster for this draw — fail
    // closed, properly signed or not.
    {
        let n = net(Some(Reserve { operator: founder, members: rmembers.clone() }), None);
        let out: Vec<(ed25519::PrivateKey, String)> = (0..4).map(|i| (n.voting[i].clone(), mac_entry(i).1)).collect();
        let (_, handoff) = n.committee.handoff_to(CHAIN, 1, &out);
        refused(&n, handoff);
    }

    // The chain opens short of four seats: the boundary rule commits the
    // roster, and a handoff must name exactly it.
    let mut n = net(Some(Reserve { operator: founder, members: rmembers }), Some(vec![mac_entry(0), mac_entry(1)]));
    let regs = (0..3).map(|i| n.register(i)).collect();
    n.step(regs, None, vec![]);
    n.run_to(E);
    let (_, roster) = aether_rewards::next_roster(&n.parent.state).expect("the missing seats are filled");
    assert_eq!(roster.len(), 4);

    // The same roster with one reserve key swapped for another: signed just as
    // well, and refused — it does not name the committed roster.
    let mut rogue = common::seat_of(&n, &roster, &rkeys);
    let at = rogue.iter().position(|(k, _)| k.public_key() == rkeys[0].public_key()).unwrap();
    rogue[at].0 = rkeys[1].clone();
    let (_, handoff) = n.committee.handoff_to(CHAIN, 1, &rogue);
    refused(&n, handoff);

    // A stale commitment is no commitment: past the draw's first block, the
    // roster an earlier draw committed does not bind this one either.
    n.run_to(SPAN + 4);
    assert_eq!(aether_rewards::next_roster(&n.parent.state).map(|(d, _)| d), Some(0), "still the last draw's roster");
    let (_, handoff) = n.committee.handoff_to(CHAIN, 2, &common::seat_of(&n, &roster, &rkeys));
    refused(&n, handoff);

    // The next boundary commits this draw's roster (the same members, its own
    // tag): a handoff naming it goes through, and the switch records it.
    n.run_to(25 * E);
    let (draw, current) = aether_rewards::next_roster(&n.parent.state).expect("this draw's roster");
    assert_eq!(draw, 1);
    let (_, handoff) = n.committee.handoff_to(CHAIN, 3, &common::seat_of(&n, &current, &rkeys));
    let carried = n.step_handoff(handoff);
    n.run_to(carried.height + aether_node::handoff::DELAY);
    assert_eq!(aether_rewards::committee(&n.parent.state), current, "the switch records the committee");
    assert_eq!(aether_rewards::seated(&n.parent.state).0, 1);
    assert!(aether_rewards::next_roster(&n.parent.state).is_none(), "the roster has served its purpose");
}

#[test]
fn overdue_seats_stop_the_credit_with_a_warning() {
    // Finding 6's other half: four independent operators qualify while the
    // reserve keys still sit — the handoff home never completed — and past the
    // two epochs of grace the chain says so where an operator sees it, in its
    // log (the payouts stopping is mainnet_rules' `…while_its_mac_sleeps`).
    let (founder, rkeys, rmembers) = reserve_set();
    let mut n = net(Some(Reserve { operator: founder, members: rmembers }), Some(vec![mac_entry(0), mac_entry(1)]));
    // Two seats stand, three independents register: the boundary commits their
    // roster (one reserve key fills the fourth seat) and the handoff seats it.
    let regs = (0..3).map(|i| n.register(i)).collect();
    n.step(regs, None, vec![]);
    n.run_to(E);
    let (_, roster) = aether_rewards::next_roster(&n.parent.state).expect("the missing seat is filled");
    let (_, handoff) = n.committee.handoff_to(CHAIN, 1, &common::seat_of(&n, &roster, &rkeys));
    let carried = n.step_handoff(handoff);
    n.run_to(carried.height + aether_node::handoff::DELAY);
    assert_eq!(aether_rewards::seated(&n.parent.state).0, 1, "one reserve key still seated");

    // The fourth independent qualifies: nobody needs the seat, and with no
    // handoff delivered the count climbs — 1, 2 (grace), then 3, where the
    // credit stops and the warning goes out.
    let reg = n.register(3);
    n.step(vec![reg], None, vec![]);
    assert_eq!(aether_node::rotation::independent(
        &aether_node::rotation::eligible(&n.parent.state, n.parent.height / E + 1, 0),
        |k| aether_node::rotation::operators(&n.parent.state).get(k).cloned(),
        &Reserve::of(&n.parent.state).unwrap(),
    ), 4, "four independent operators qualify");
    let log = Log(Arc::default());
    let capture = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(log.clone())
        .finish();
    tracing::subscriber::with_default(capture, || n.run_to(n.parent.height + 5 * E));
    let out = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    let (_, count) = aether_rewards::overdue(&n.parent.state);
    assert!(count > aether_rewards::RESERVE_GRACE_EPOCHS, "past the grace: {count}");
    assert!(
        out.contains("founder reserve keys still hold seats") && out.contains("service credit has stopped"),
        "the boundary that stops the credit warns:\n{out}"
    );
    assert_eq!(aether_rewards::seated(&n.parent.state).0, 1, "still seated: only the credit stopped");
}

#[test]
fn a_network_without_node_rewards_hands_over_as_before() {
    // The binding is a node-rewards rule: a genesis that did not turn them on
    // (testnet 7780) still accepts a committee-signed handoff with no roster
    // anywhere in state — its consensus rules never change.
    let mut n = Net::new(Opts { chain_id: CHAIN + 1, node_rewards: false, epoch_blocks: E, macs: 4, min_streak: Some(0), history_v2: false, reserve: None, fees: false, committee: None });
    let out: Vec<(ed25519::PrivateKey, String)> = (0..4).map(|i| (n.voting[i].clone(), mac_entry(i).1)).collect();
    let (_, handoff) = n.committee.handoff_to(CHAIN + 1, 1, &out);
    let carried = n.step_handoff(handoff);
    assert!(carried.handoff.is_some(), "accepted: no node-rewards rule binds it");
}

#[test]
fn a_network_file_never_seats_a_validator_that_is_also_a_reserve_key() {
    // Finding 3: `aether network` and every node's startup derive genesis from
    // network.json — both refuse a key (or a node id) that is both a validator
    // and a reserve key: the founder's Mac would run it either way, and the
    // overlap would hide from the independent-operator count.
    let member = |b: u8| Member { key: hex::encode([b; 32]), node: aether_net::SecretKey::from_bytes(&[b; 32]).public().to_string() };
    let file = |reserve: Vec<Member>| NetworkFile {
        chain_id: CHAIN,
        validators: vec![member(1), member(2), member(3), member(4)],
        identity: None,
        round: 0,
        output: None,
        epochs: vec![],
        faucet: None,
        registrar: None,
        epoch_blocks: None,
        min_streak: None,
        draw_epochs: None,
        history: None,
        node_rewards: Some(true),
        reserve: Some(ReserveFile { operator: Address::repeat_byte(0xf0), validators: reserve }),
        genesis_validators: None,
    };
    assert!(file(vec![member(9)]).genesis().is_ok(), "a reserve key of its own is fine");
    assert_eq!(file(vec![member(1)]).genesis().unwrap_err(), "validator 1: its key is also a reserve key");
    let mut by_node = member(9);
    by_node.node = member(2).node;
    assert_eq!(file(vec![by_node]).genesis().unwrap_err(), "validator 2: its node id is also a reserve key's");
    assert_eq!(file(vec![]).genesis().unwrap_err(), format!("1 to {} reserve keys", aether_rewards::MAX_RESERVE_KEYS));
}
