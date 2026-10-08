//! The mainnet genesis rules end to end (docs/design/12-launch-plan.md,
//! docs/design/13-roadmap.md E행, docs/design/15-node-rewards.md): a chain
//! with the mainnet flags — history v2, node rewards from genesis — over many
//! epochs and warm-up days. The node pool of an epoch is the sum of the
//! issuance it halves into it; each operator gets exactly a sixteenth of it
//! while fewer than 16 are online, from 16 full-weight operators on the whole
//! pool is shared, and what caps and absences leave is never minted. Warm-up
//! climbs one step a day, and the founder's reserve keys join the voting set
//! into missing seats, keep standby through four independent operators, and
//! apply ordinary exit/expiry at five — while they serve, the
//! epochs count as the founder's participation.

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
/// Short epochs (twelve slots of three blocks, a one-block answer window) so a
/// day is 864 blocks, not 86_400: the warm-up climb (a step a day, 14 days)
/// still finishes in minutes.
const E: u64 = 36;
const MACS: usize = 20;
/// What the harness funds each operator with (gas money, not a premine of the
/// chain's: rewards are the only mints this test counts).
const FUNDED: u128 = 10u128.pow(20);

fn net(macs: u8, reserve: Option<Reserve>, committee: Option<Vec<(String, String)>>) -> Net {
    Net::new(Opts {
        chain_id: CHAIN,
        node_rewards: true,
        epoch_blocks: E,
        macs,
        min_streak: Some(0),
        history_v2: true,
        protocol: aether_node::upgrade::PROTOCOL,
        reserve,
        fees: false,
        committee,
    })
}

/// Every Mac's answers for the next block, one state view for all of them
/// (`answers` clones the state once per Mac: 20 Macs × 40_000 blocks adds up).
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
                Mac::Awake(mask) => mask & (1 << (d.epoch % DAY_EPOCHS)) != 0,
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

/// The mainnet rule set (docs/ops/mainnet-launch.md §2): every rule the
/// mainnet must have active at height 1, checked against this freshly built
/// mainnet-flag genesis — and against the chain as it actually runs. A new
/// rule joins the doc's table, `mainnet::check` and this list together.
#[test]
fn the_mainnet_rule_set_is_on_from_height_1() {
    use aether_node::mainnet;
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(5)).unwrap());
    let mut n = net(5, Some(Reserve { operator: founder, members: reserve_members() }), None);
    // The harness funds every operator with gas money (FUNDED above) so its
    // supply identities stay simple; the mainnet's genesis funds nobody. The
    // checklist runs on the mainnet shape of this genesis — everything but the
    // alloc is the chain as built, and the live checks below run on that chain.
    let mut cfg = n.chain.cfg();
    cfg.alloc.clear();
    // The harness shortens epochs and uses a made-up registrar key; the checklist
    // runs on a real key and, for the timing rule, as a rehearsal (audit 2, R2-3).
    {
        use aether_crypto::Signer;
        let key = aether_crypto::P256Signer::from_seed(&[5; 32]).unwrap().public_key();
        cfg.registrar = Some(aether_crypto::p256_xy(&key.bytes).unwrap());
    }
    let rules = mainnet::check_with(&cfg, true);
    assert_eq!(
        rules.iter().map(|r| r.name).collect::<Vec<_>>(),
        [
            "chain id",
            "protocol from genesis",
            "proof market",
            "registry v3",
            "standard predeploys",
            "registrar key",
            "registration cap",
            "16-seat growth",
            "epoch parameters",
            "candidate timing",
            "node rewards",
            "beacons",
            "re-attestation",
            "reserve rules",
            "smooth issuance",
            "history v2",
            "receipt commitments",
            "paid state growth",
            "pruning default",
            "no premine, no faucet",
            "zero-tip acceptance",
        ]
    );
    assert!(rules.iter().all(|r| r.ok), "{}", mainnet::missing(&rules));

    // What the checklist asserts is also what the chain runs: the activation is
    // on the schedule from height 0, the cap is in the registry's storage, and
    // the first block with a transaction records a proof-market statement.
    let g = n.chain.lock().finalized.clone();
    assert_eq!(aether_node::upgrade::protocol_at(&g.schedule, 1), aether_node::upgrade::PROTOCOL);
    assert_eq!(registry::max_per_epoch(&g.state), registry::MAX_PER_EPOCH);
    let mut minted = U256::ZERO;
    let registration = n.register(0);
    step(&mut n, &mut minted, vec![registration]);
    assert_ne!(n.parent.statement, Default::default(), "block 1 records a statement");

    // Audit 5, A5-4: the reserved chain ids (the running testnet, the
    // rehearsal default) fail exactly their own rule in strict mode, however
    // correct the rest of the genesis is. The harness shortens timing (a
    // rehearsal allowance): put the published policy back for the strict run.
    let mut reserved = cfg.clone();
    reserved.chain_id = 7_799;
    reserved.epoch_blocks = 0;
    reserved.min_streak = None;
    reserved.draw_epochs = None;
    let off = mainnet::check_with(&reserved, false).iter().filter(|r| !r.ok).map(|r| r.name).collect::<Vec<_>>();
    assert_eq!(off, ["chain id"], "only the chain id rule rejects the rehearsal id");
    let mut testnet = reserved.clone();
    testnet.chain_id = 7_780;
    let off = mainnet::check_with(&testnet, false).iter().filter(|r| !r.ok).map(|r| r.name).collect::<Vec<_>>();
    assert!(off.contains(&"chain id"), "the testnet id fails the chain id rule too: {off:?}");
}

/// A real DKG output over the four genesis validators, from the tests/dkg.rs
/// harness pattern run over a perfect in-memory network: exactly the hex pair
/// `aether dkg` writes into network.json.
fn ceremony_output() -> (String, String) {
    use aether_node::block::PublicKey;
    use aether_node::dkg::{Ceremony, KeyFile, Round, To};
    use commonware_utils::TryCollect;
    use rand::SeedableRng as _;
    use rand_chacha::ChaCha20Rng;
    use std::collections::VecDeque;
    let players: Vec<_> = (1..=4u64).map(aether_light::devnet_validator_key).collect();
    let pks: Vec<PublicKey> = players.iter().map(|k| k.public_key()).collect();
    let participants: commonware_utils::ordered::Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
    let mut queue: VecDeque<(PublicKey, PublicKey, aether_node::dkg::Msg)> = VecDeque::new();
    let send = |queue: &mut VecDeque<(PublicKey, PublicKey, aether_node::dkg::Msg)>, from: &PublicKey, out: Vec<(To, aether_node::dkg::Msg)>| {
        for (to, msg) in out {
            let targets: Vec<PublicKey> = match to {
                To::One(p) => vec![p],
                To::All => pks.iter().filter(|p| *p != from).cloned().collect(),
            };
            for t in targets {
                queue.push_back((from.clone(), t, msg.clone()));
            }
        }
    };
    let mut cs = Vec::new();
    for (i, k) in players.iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(11), k.clone(), Round::dkg(participants.clone(), 0), None).unwrap();
        send(&mut queue, &pks[i], out);
        cs.push(c);
    }
    let idx = |p: &PublicKey| pks.iter().position(|x| x == p).unwrap();
    let mut done = None;
    for tick in 0..400 {
        for _ in 0..queue.len() {
            let Some((from, to, msg)) = queue.pop_front() else { break };
            let i = idx(&to);
            send(&mut queue, &to, cs[i].on_message(&from, msg));
        }
        for i in 0..pks.len() {
            let mut out = cs[i].pending_deals();
            if cs[i].all_acked() || tick > 20 {
                out.extend(cs[i].close_dealing());
            }
            if tick > 60 && cs[i].have_quorum_logs() {
                if let Some((_, msg)) = cs[i].propose_transcript() {
                    out.push((To::All, msg));
                }
                out.extend(cs[i].tick_agreement());
            }
            if done.is_none() {
                if let Some(digest) = cs[i].certified_transcript() {
                    let (o, s) = cs[i].finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &digest).unwrap();
                    done = Some(KeyFile::new(0, &o, &s));
                }
            }
            out.extend(cs[i].rebroadcast());
            send(&mut queue, &pks[i], out);
        }
        if done.is_some() {
            break;
        }
    }
    let f = done.expect("ceremony completes");
    (f.output, f.identity)
}

/// A final network.json in the mainnet shape: the published policy flags and
/// the committee fields under test.
fn final_file(chain: u64, round: u64, output: Option<String>, identity: Option<String>) -> aether_node::roster::NetworkFile {
    use aether_node::roster::{Member, NetworkFile, ReserveFile};
    let (x, y) = {
        use aether_crypto::Signer;
        let key = aether_crypto::P256Signer::from_seed(&[5; 32]).unwrap().public_key();
        aether_crypto::p256_xy(&key.bytes).unwrap()
    };
    let mut registrar = [0u8; 64];
    registrar[..32].copy_from_slice(&x);
    registrar[32..].copy_from_slice(&y);
    NetworkFile {
        chain_id: chain,
        validators: (1..=4u64)
            .map(|i| {
                let k = aether_light::devnet_validator_key(i);
                Member { key: hex::encode(k.public_key().as_ref()), node: aether_net::devnet_node_id(i).to_string() }
            })
            .collect(),
        identity,
        round,
        output,
        epochs: vec![],
        faucet: None,
        registrar: Some(hex::encode(registrar)),
        epoch_blocks: None,
        min_streak: None,
        draw_epochs: None,
        history: Some(2),
        protocol: Some(aether_node::upgrade::PROTOCOL),
        node_rewards: Some(true),
        reserve: Some(ReserveFile {
            operator: Address::repeat_byte(0x99),
            validators: (1..=3)
                .map(|i| Member { key: hex::encode([i; 32]), node: aether_net::SecretKey::from_bytes(&[i; 32]).public().to_string() })
                .collect(),
        }),
        group: None,
        max_committee: Some(aether_node::rotation::GROW_UNTIL as u64),
        genesis_validators: None,
        release: None,
    }
}

/// Audit 5, A5-4: the final-file gate. `mainnet-rules` does not stop at
/// rebuilding the genesis — it decodes the exact committee fields with the
/// node's own decoder, seats the roster, pins the identity to the group
/// public key and refuses revealed seated shares. The file the audit built
/// (valid structure, `identity: "aa"`, `output: "bb"`) must FAIL; a real
/// ceremony output must PASS; a roster the output does not seat must FAIL.
#[test]
fn the_final_file_gate_decodes_the_committee_it_is_given() {
    use aether_node::mainnet;
    let names = ["committee output decodes", "output seats the genesis roster", "identity is the group public key", "no revealed seated share"];

    // A real ceremony output passes every final rule.
    let (output, identity) = ceremony_output();
    let file = final_file(7_801, 0, Some(output), Some(identity));
    let rules = mainnet::check_final(&file, false);
    assert_eq!(rules.iter().map(|r| r.name).collect::<Vec<_>>(), names);
    assert!(rules.iter().all(|r| r.ok), "{}", mainnet::missing(&rules));

    // The audit's junk fields: valid structure, unusable committee.
    let junk = final_file(7_801, 0, Some("bb".into()), Some("aa".into()));
    let rules = mainnet::check_final(&junk, false);
    assert!(rules.iter().all(|r| !r.ok), "no rule may pass on an undecodable output: {:?}", rules);

    // A DKG over a different player set does not seat this roster.
    let (output, identity) = ceremony_output();
    let mut other = final_file(7_801, 0, Some(output), Some(identity));
    let replacement = aether_light::devnet_validator_key(9);
    other.validators[3] = aether_node::roster::Member {
        key: hex::encode(replacement.public_key().as_ref()),
        node: aether_net::devnet_node_id(9).to_string(),
    };
    let off = mainnet::check_final(&other, false).iter().filter(|r| !r.ok).map(|r| r.name).collect::<Vec<_>>();
    assert!(off.contains(&"output seats the genesis roster"), "a swapped validator fails the seating rule: {off:?}");

    // Pre-DKG is explicitly ok (assemble wrote it); half a final file is not.
    let pre = final_file(7_801, 0, None, None);
    assert!(mainnet::check_final(&pre, false).iter().all(|r| r.ok));
    let half = final_file(7_801, 0, None, Some("aa".into()));
    assert!(!mainnet::check_final(&half, false).iter().all(|r| r.ok));
}

#[test]
fn one_four_and_twenty_operators_get_exact_shares_and_the_rest_is_never_minted() {
    let mut n = net(MACS as u8, None, None);
    let mut minted = U256::ZERO;
    let proposer = leader_address(&ed25519::PrivateKey::from_seed(1).public_key());
    let others = [proposer, PROVER_ESCROW];
    let start = supply(&n, &others);
    assert_eq!(start, U256::from(MACS as u128 * FUNDED), "the genesis funds nobody but the operators' gas money");
    assert!(rewards::enabled(&n.parent.state), "node rewards from the first block");
    // Rewards are the only mint: the whole supply is the premine plus every
    // payout of every block (checked at each stop below, after every step).
    let check = |n: &Net, minted: &U256| {
        assert_eq!(supply(n, &others) + n.burned_state, start + *minted, "rewards are the only mint; state fees burn");
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
    let (before, _) = paid_epoch(&mut n, &mut minted, mixed, 4);
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
    // all twelve slots, so a Mac's weight is SLOTS × (14 + level).
    let epoch = 20 * DAY_EPOCHS + 10;
    run_to(&mut n, &mut minted, epoch * E);
    let weights: Vec<u64> = (0..4u64).map(|i| rewards::SLOTS * (WARMUP_STEPS + rewards::mac(&n.parent.state, i).level)).collect();
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
    let weights: Vec<u64> = (0..MACS as u64).map(|i| rewards::SLOTS * (WARMUP_STEPS + rewards::mac(&n.parent.state, i).level)).collect();
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
    assert_eq!(supply(&n, &others) + n.burned_state, start + minted, "the total supply is the premine plus minted rewards minus burned state fees");
}

#[test]
fn reserve_keys_fill_short_committees_and_return_to_standby_at_four() {
    // Mac 4 is the founder's own registered Mac: not independent (docs/design/15).
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(5)).unwrap());
    let reserve = Reserve { operator: founder, members: reserve_members() };
    let (rkeys, _): (Vec<ed25519::PrivateKey>, Vec<_>) = reserve_set().into_iter().unzip();
    let no_reserve = |m: &[(String, String)]| reserve.members.iter().all(|r| !m.contains(r));
    let reserve_seats = |m: &[(String, String)]| reserve.members.iter().filter(|r| m.contains(r)).count();
    let independents = |n: &Net| {
        // As the boundary rule sees it: the epoch starting here.
        let r = Reserve::of(&n.parent.state).expect("the reserve is on chain");
        let pool = aether_node::rotation::eligible(&n.parent.state, n.parent.height / E, 0);
        let ops = aether_node::rotation::operators(&n.parent.state);
        aether_node::rotation::independent(&pool, |k| ops.get(k).cloned(), &r)
    };

    // Two independent Macs and the founder's register against a committee
    // already standing at four genesis seats: not one reserve key joins (a
    // 4-seat committee growing to 7 would need 5 of 7, and one founder Mac
    // going dark would stall it: audit 1.1) — and with no roster committed,
    // no committee change may happen at all (finding 2, reserve_hardening.rs).
    {
        let mut n = net(5, Some(reserve.clone()), None);
        let mut minted = U256::ZERO;
        assert_eq!(n.operator(4), founder);
        assert_eq!(Reserve::of(&n.parent.state).unwrap().members, reserve.members);
        let regs = [0, 1, 4].iter().map(|i| n.register(*i)).collect();
        step(&mut n, &mut minted, regs);
        // Registration lands inside epoch 0. Stability has three full up
        // observations only after epochs 1, 2 and 3 have been distributed.
        run_to(&mut n, &mut minted, 4 * E);
        assert_eq!(independents(&n), 2);
        assert!(aether_rewards::next_roster(&n.parent.state).is_none(), "a full committee takes no reserve key");
    }

    // The network opens short of four seats — the bootstrap the reserve keys
    // are for: qualifying Macs fill first and the reserve keys take only the
    // seats still missing, and only a handoff naming exactly that committed
    // roster may carry the committee over.
    let mut n = net(5, Some(reserve.clone()), Some(vec![common::mac_entry(0), common::mac_entry(1)]));
    let mut minted = U256::ZERO;
    let regs = [0, 1, 4].iter().map(|i| n.register(*i)).collect();
    step(&mut n, &mut minted, regs);
    run_to(&mut n, &mut minted, E);
    let (_, initial) = aether_rewards::next_roster(&n.parent.state).expect("the short committee initially needs reserve seats");
    assert_eq!(reserve_seats(&initial), 2, "only the two genesis seats are available before stability is established");
    // The first draw keeps its committed roster until the next draw. Epoch 4
    // has the required three full beacon observations, but draw 1's first
    // non-freeze boundary is epoch 25. There the founder's Mac takes a seat
    // without counting toward the four independent operators.
    run_to(&mut n, &mut minted, 25 * E);
    assert_eq!(independents(&n), 2);
    let (_, roster) = aether_rewards::next_roster(&n.parent.state).expect("a short committee is refilled");
    assert_eq!(roster.len(), 4, "refilled to four seats");
    // Qualified Macs are seated first; reserve keys take only what is still
    // missing (exact counts per independent operator: rotation.rs unit tests).
    let others = roster.len() - reserve_seats(&roster);
    assert_eq!(reserve_seats(&roster), 4usize.saturating_sub(others), "only the missing seats");
    assert_eq!(reserve_seats(&roster), 1, "three qualifying Macs leave one seat for the reserve");
    assert!([common::mac_entry(0), common::mac_entry(1), common::mac_entry(4)].iter().all(|m| roster.contains(m)), "the founder's Mac fills a seat");
    let (seated, handoff) = n.committee.handoff_to(CHAIN, 1, &common::seat_of(&n, &roster, &rkeys));
    let carried = n.step_handoff(handoff);
    run_to(&mut n, &mut minted, carried.height + aether_node::handoff::DELAY);
    assert_eq!(aether_rewards::committee(&n.parent.state), roster, "the switch records the committee");
    assert_eq!(rewards::seated(&n.parent.state).0, reserve_seats(&roster) as u64);

    // Macs 2 and 3: four independent operators (the founder's Mac still does
    // not count), so every reserve key leaves.
    let regs = [2, 3].iter().map(|i| n.register(*i)).collect();
    step(&mut n, &mut minted, regs);
    let four = n.parent.height / E + 4; // registration epoch, then three full epochs
    run_to(&mut n, &mut minted, four * E);
    assert_eq!(independents(&n), 4);
    assert_eq!(aether_node::rotation::eligible(&n.parent.state, four, 0).len(), 5);
    let (_, leave) = aether_rewards::next_roster(&n.parent.state).expect("the next roster drops them");
    assert!(no_reserve(&leave), "the reserve keys leave at four");
    assert!(leave.contains(&common::mac_entry(0)) && leave.contains(&common::mac_entry(1)));
    assert!(leave.contains(&common::mac_entry(2)) || leave.contains(&common::mac_entry(3)), "a qualifying Mac takes the freed seat");
    let (_, handoff) = seated.handoff_to(CHAIN, 2, &common::seat_of(&n, &leave, &rkeys));
    let carried = n.step_handoff(handoff);
    run_to(&mut n, &mut minted, carried.height + aether_node::handoff::DELAY);
    assert_eq!(rewards::seated(&n.parent.state), (0, 0), "unseated: the word is cleared");
    assert!(no_reserve(&aether_rewards::committee(&n.parent.state)));
    assert_eq!(Reserve::of(&n.parent.state).unwrap().members, reserve.members, "unseated keys remain registered standby");
    // They earned nothing: not candidates, no beacons.
    assert!(registry::candidates(&n.parent.state).iter().all(|c| !reserve.members.iter().any(|(k, _)| *k == hex::encode(c.validator_key))));
}

fn reserve_members() -> Vec<(String, String)> {
    reserve_set().into_iter().map(|(_, m)| m).collect()
}

/// The founder's reserve keys: each one's ed25519 key and its (key hex,
/// iroh node id) pair, as the genesis `--reserve` line names them.
fn reserve_set() -> Vec<(ed25519::PrivateKey, (String, String))> {
    (1..=3u64)
        .map(|i| {
            let k = ed25519::PrivateKey::from_seed(100 + i);
            let pk = k.public_key();
            let node = aether_net::SecretKey::from_bytes(&[0x70 + i as u8; 32]).public();
            (k, (hex::encode(pk.as_ref()), node.to_string()))
        })
        .collect()
}

#[test]
fn reserve_service_pays_the_founder_while_its_mac_sleeps() {
    // Mac 4 is the founder's own registered Mac (docs/design/15, "창업자 예비
    // 키"). The network opens with mac 0's key alone in the committee — the
    // bootstrap the reserve keys are for. The founder's Mac warms up with the
    // network, then sleeps: the chain, not any one node, commits the seating
    // as a roster, and only a handoff naming exactly it carries the committee
    // over. Nothing before the switch (nothing is seated), nothing for the
    // epoch it lands inside, a full sixteenth for every epoch after it —
    // exactly as if the Mac had answered every slot, at the warm-up it
    // reached, never a second share. Five independent operators qualifying
    // while the keys still sit ends it: two more epochs of grace (finding 6),
    // then the credit stops until a committee without the keys takes over.
    let founder = common::addr(&aether_crypto::P256Signer::from_seed(&common::seed(5)).unwrap());
    let (rkeys, rmembers): (Vec<ed25519::PrivateKey>, Vec<(String, String)>) = reserve_set().into_iter().unzip();
    let reserve = Reserve { operator: founder, members: rmembers.clone() };
    let seated_in = |m: &[(String, String)]| rmembers.iter().filter(|r| m.contains(r)).count();
    let mut n = net(6, Some(reserve), Some(vec![common::mac_entry(0)]));
    let mut minted = U256::ZERO;
    assert_eq!(n.operator(4), founder);
    // Two independent operators and the founder's Mac warm up together; the
    // founder's Mac answers for itself, so the credit is idle. Macs 0, 1 and
    // 4 register, in that order — so their candidate (registry) indices are
    // 0, 1 and 2; Macs 2 and 3, registering later, are candidates 3 and 4.
    // Warm-up state is per candidate index.
    let regs = [0, 1, 4].iter().map(|&i| n.register(i)).collect::<Vec<_>>();
    step(&mut n, &mut minted, regs);
    for day in 0..WARMUP_STEPS as u64 {
        run_day(&mut n, &mut minted, day, &[0, 1, 2], day);
    }
    assert_eq!(rewards::mac(&n.parent.state, 2).level, WARMUP_STEPS, "full weight before it sleeps");
    assert_eq!(rewards::seated(&n.parent.state), (0, 0), "no handoff has landed: nothing seated");

    // The founder's Mac sleeps from the boundary the day-14 judgment and
    // draw 14's freeze both land on. It loses eligibility after its first
    // silent epoch (336), while its level stays where the judgment left it.
    // No handoff has landed, so the sleeping Mac earns nothing.
    n.behaviour.insert(4, Mac::Off);
    let full = n.parent.height / E;
    let (before, exec) = paid_epoch(&mut n, &mut minted, full, 5);
    assert_eq!(n.balance(4), before[4], "nothing seated: a sleeping Mac earns nothing");
    assert!(!exec.payouts.iter().any(|(_, op, _)| *op == founder), "the founder is off the payouts");
    assert_eq!(exec.payouts.len(), 2, "only the answering operators");

    // The next boundary removes the sleeping Mac from the eligible pool.
    // Mac 1 fills one of the committee's missing seats, and two reserve keys
    // fill the rest. Only a handoff naming that committed roster may carry it.
    let (_, roster) = aether_rewards::next_roster(&n.parent.state).expect("the chain seats the reserve keys");
    assert_eq!(roster.len(), 4);
    assert_eq!(seated_in(&roster), 2, "qualifying Macs first, the reserve keys fill the rest: {roster:?}");
    assert!(roster.contains(&common::mac_entry(0)) && roster.contains(&common::mac_entry(1)));
    assert!(!roster.contains(&common::mac_entry(4)), "the sleeping founder Mac is not seated");
    let (seated, handoff) = n.committee.handoff_to(CHAIN, 1, &common::seat_of(&n, &roster, &rkeys));
    let carried = n.step_handoff(handoff);
    let switch = carried.height + aether_node::handoff::DELAY;
    assert_eq!(switch % E, 29, "the seating switch lands inside an epoch");
    run_to(&mut n, &mut minted, switch);
    assert_eq!(rewards::seated(&n.parent.state), (2, switch), "two keys seated, from the switch");
    assert_eq!(aether_rewards::committee(&n.parent.state), roster);
    let half = switch / E;
    let (before, exec) = paid_epoch(&mut n, &mut minted, half, 5);
    assert_eq!(n.balance(4), before[4], "the epoch the seating lands in is not served");
    assert_eq!(exec.payouts.len(), 2, "the founder is off the payouts");

    // Every epoch the keys hold the seats pays the founder what its Mac would
    // have earned answering every slot: a sixteenth, counted as one
    // operator among the answering ones, on the record.
    for e in half + 1..half + 3 {
        let (before, exec) = paid_epoch(&mut n, &mut minted, e, 5);
        let pool = pool_of_epoch(e);
        assert_eq!(n.balance(4) - before[4], pool / U256::from(MAX_SHARE), "epoch {e}: the sleeping founder's full share");
        assert_eq!(n.balance(0) - before[0], pool / U256::from(MAX_SHARE), "epoch {e}: an answering operator gets the same");
        assert!(exec.payouts.iter().any(|(_, op, a)| *op == founder && *a == pool / U256::from(MAX_SHARE)), "the payout is on the record");
        assert_eq!(exec.payouts.len(), 3, "the founder counts as one operator");
    }
    assert_eq!(rewards::mac(&n.parent.state, 2).level, WARMUP_STEPS, "the service moved no warm-up");

    // Macs 2 and 3 register partway through an epoch. After their next three
    // full epochs, four independent operators qualify. The unused reserve
    // seats can return to the Macs, but service cannot expire while a handoff
    // is still outstanding at four.
    let regs = (2..4).map(|i| n.register(i)).collect::<Vec<_>>();
    step(&mut n, &mut minted, regs);
    let four = n.parent.height / E + 4;
    run_to(&mut n, &mut minted, four * E);
    let (_, leave) = aether_rewards::next_roster(&n.parent.state).expect("the chain unseats them");
    assert_eq!(seated_in(&leave), 0, "every reserve key leaves at once");
    assert!(leave.contains(&common::mac_entry(0)) && leave.contains(&common::mac_entry(1)));
    assert_eq!(rewards::overdue(&n.parent.state), (four, 0), "four operators never start expiry");
    for e in four..four + 4 {
        let (before, _) = paid_epoch(&mut n, &mut minted, e, 6);
        assert_eq!(n.balance(4) - before[4], pool_of_epoch(e) / U256::from(MAX_SHARE), "epoch {e}: standby service does not expire at four");
        assert_eq!(rewards::overdue(&n.parent.state), (e + 1, 0));
    }

    // Mac 5 is a fifth independent operator (Mac 4 remains the founder).
    let reg = n.register(5);
    step(&mut n, &mut minted, vec![reg]);
    let five = n.parent.height / E + 4;
    run_to(&mut n, &mut minted, five * E);
    assert_eq!(rewards::overdue(&n.parent.state), (five, 1), "five operators start the existing grace");

    // Finding 6: the handoff home has not happened — the keys still sit. The
    // credit lasts the two epochs of grace and stops, on chain.
    for e in [five, five + 1] {
        let (before, exec) = paid_epoch(&mut n, &mut minted, e, 6);
        let pool = pool_of_epoch(e);
        assert_eq!(n.balance(4) - before[4], pool / U256::from(MAX_SHARE), "epoch {e}: still inside the grace");
        assert_eq!(exec.payouts.len(), 6, "the founder counts as one operator");
    }
    assert_eq!(rewards::overdue(&n.parent.state), (five + 2, 3), "past the grace the count keeps climbing");
    for e in [five + 2, five + 3] {
        let (before, exec) = paid_epoch(&mut n, &mut minted, e, 6);
        assert_eq!(n.balance(4), before[4], "epoch {e}: past the grace, no credit while the keys still sit");
        assert!(!exec.payouts.iter().any(|(_, op, _)| *op == founder), "the founder is off the payouts");
        assert_eq!(exec.payouts.len(), 5, "only the answering operators");
    }

    // A committee without the keys takes over: from its switch — inside an
    // epoch again — the seating word is clear and the count resets.
    let (_, handoff) = seated.handoff_to(CHAIN, 2, &common::seat_of(&n, &leave, &rkeys));
    let carried = n.step_handoff(handoff);
    let unswitch = carried.height + aether_node::handoff::DELAY;
    assert_eq!(unswitch % E, 29, "the unseating switch lands inside an epoch too");
    run_to(&mut n, &mut minted, unswitch);
    assert_eq!(rewards::seated(&n.parent.state), (0, 0), "unseated: the word is cleared");
    let gone = unswitch / E + 1;
    let (before, exec) = paid_epoch(&mut n, &mut minted, gone, 6);
    assert_eq!(n.balance(4), before[4], "unseated, no credit for a sleeping Mac");
    assert!(!exec.payouts.iter().any(|(_, op, _)| *op == founder), "the founder is off the payouts");
    assert_eq!(exec.payouts.len(), 5, "only the answering operators");
    assert_eq!(rewards::mac(&n.parent.state, 2).level, WARMUP_STEPS, "still no warm-up movement");
}
