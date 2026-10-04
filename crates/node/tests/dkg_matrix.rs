//! Byzantine scenario matrix for the DKG / reshare agreement layer
//! (docs/design/28-dkg-agreement-spec.md).
//!
//! Every case drives the real `Ceremony` state machines (the same API the
//! production child runs) over the lossy, reordering in-memory network of
//! tests/dkg.rs, for n ∈ {4, 7} (f = 1, 2), one or f Byzantine participants,
//! and several seeds. One behaviour is injected per run — silent from the
//! start, silent after publishing a log, acks late just before or past the
//! dealing window, a dealer log late around the proposal point, the decision
//! missed past the grace, an honest restart mid-phase, equivocating logs,
//! equivocating votes (one key running two ceremonies over two log diets),
//! forged `Done` and malformed agreement messages, a prior round's replayed
//! log/decision, and another chain's valid messages. Handoff readiness
//! withholding runs at the `handoff::Readiness` layer.
//!
//! After every run the same checks assert the spec's invariants: one
//! certified transcript digest and byte-identical staged outputs (S1), no
//! revealed seated share in anything staged (S2), every staged share usable
//! under the staged output and distinct (S3), replayed and cross-chain
//! messages leaving state untouched (S5), and the per-case liveness claim
//! (clean failure plus fresh-round recovery, or recovery from one bounded
//! relay / a restart).
//!
//! Sized to finish in a few minutes:
//! `cargo test -p aether-node --test dkg_matrix -- --test-threads=2`.

use aether_node::block::PublicKey;
use aether_node::dkg::{Ceremony, KeyFile, Msg, Round, To};
use aether_node::dkg_agreement::{AgreementMsg, Certificate, Phase, SignedVote};
use commonware_codec::{DecodeExt as _, Encode};
use commonware_cryptography::bls12381::primitives::group::Share;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_utils::ordered::Set;
use commonware_utils::TryCollect;
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha20Rng;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::OnceLock;

const CHAIN: u64 = 7781;
const OTHER_CHAIN: u64 = 7782;
/// Driver ticks mirroring the production timers (500 ms each): dealing
/// closes at tick 20, proposals may start at tick 60, the decision grace is
/// 10 ticks, and the post-stage relay window is 120.
const CLOSE_TICK: usize = 20;
const PROPOSE_TICK: usize = 60;
const GRACE_TICKS: usize = 10;
const RELAY_TICKS: usize = 120;
/// The production agreement re-broadcasts everything every 4 ticks; matching
/// that period keeps the harness equivalent and its crypto work bounded.
const REBROADCAST_EVERY: usize = 4;
const RUN_CAP: usize = 600;
/// Failure cases can never certify; stop them early.
const FAIL_CAP: usize = PROPOSE_TICK + 80;

type Files = BTreeMap<PublicKey, KeyFile>;

/// Fault knobs: (silent_from_start, silent_from, deaf, hold_log_until, restart_at).
type Knobs = (bool, Option<usize>, Option<(usize, usize)>, Option<usize>, Option<usize>);

fn keys(n: u64) -> Vec<ed25519::PrivateKey> {
    (1..=n).map(aether_light::devnet_validator_key).collect()
}

/// Dealer randomness per node: stable across a restart (same seed, index and
/// round), fresh across rounds and chains — the deal journal's contract.
fn node_seed(seed: u64, i: usize, round: u64) -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64((seed ^ 0x5a5a_a5a5).rotate_left(8) ^ (i as u64 + 0x9e37) ^ round.wrapping_mul(1_000_003))
}

struct Net {
    queue: VecDeque<(PublicKey, PublicKey, Msg)>,
    rng: ChaCha20Rng,
}

impl Net {
    fn send(&mut self, from: &PublicKey, peers: &[PublicKey], out: Vec<(To, Msg)>) {
        for (to, msg) in out {
            let targets: Vec<PublicKey> = match to {
                To::One(p) => vec![p],
                To::All => peers.iter().filter(|p| *p != from).cloned().collect(),
            };
            for t in targets {
                // Random position: reordering.
                let at = self.rng.random_range(0..=self.queue.len());
                self.queue.insert(at, (from.clone(), t, msg.clone()));
            }
        }
    }
}

/// One matrix behaviour. Late/restart behaviours target an honest player
/// (index `n - 1`); the others target the Byzantine set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    /// Byzantine players never start: dealer logs reveal their seats, so no
    /// honest player may propose (S2), and a fresh round must recover (L1).
    SilentPlayer,
    /// Departing dealers never deal during a reshare; the remaining dealers
    /// are exactly a quorum and the new committee must stage.
    SilentDealer,
    /// Byzantine players ack and publish their log, then stop voting.
    SilentAfterLog,
    /// An honest player's inbound is blocked until just before dealing
    /// closes; its acks still land inside the window, so no reveal.
    LateAckBeforeClose,
    /// ... until after the window closed: the dealers' logs reveal the late
    /// seat, so nothing may stage.
    LateAckPastClose,
    /// An honest dealer's log is held until just before proposals start.
    LateLogBeforeProposal,
    /// ... until after the others proposed (the R2-1 schedule).
    LateLogAfterProposal,
    /// An honest player is partitioned past the decision grace and must
    /// recover from the peers' bounded rebroadcast (L3).
    LateDecisionPastGrace,
    /// An honest player is dropped and recreated (same seed) mid-dealing.
    RestartDuringDealing,
    /// ... mid-agreement.
    RestartDuringAgreement,
    /// Byzantine dealers sign two different valid logs and split them.
    EquivocateLogs,
    /// A Byzantine player runs a twin ceremony (same key, same dealing, one
    /// dealer's log withheld) and broadcasts both digests in the same views.
    EquivocateVotes,
    /// Byzantine players inject arbitrary `Done` and malformed agreement
    /// messages at proposal time (A3-2).
    ForgedMessages,
}

/// Everything a run observes, by participant index.
struct Case {
    certified: BTreeMap<usize, Vec<u8>>,
    staged: BTreeMap<usize, KeyFile>,
    staged_at: BTreeMap<usize, usize>,
    /// Each dealer's signed `Log` as first broadcast.
    logs: BTreeMap<usize, Msg>,
    /// First decision certificate delivered to an honest player.
    decision: Option<Msg>,
}

struct Node {
    key: ed25519::PrivateKey,
    ceremony: Ceremony,
    twin: Option<Ceremony>,
    twin_blind: Option<PublicKey>,
    twin_proposal: Option<Vec<u8>>,
    /// Deals received (replayed into a restarted ceremony: the deal journal).
    seen_deals: Vec<(PublicKey, Msg)>,
    held_log: Vec<(To, Msg)>,
    hold_log_until: Option<usize>,
    /// Deliveries dropped during `[from, to)` ticks.
    deaf: Option<(usize, usize)>,
    silent_from_start: bool,
    silent_from: Option<usize>,
    restart_at: Option<usize>,
    proposal: Option<Vec<u8>>,
}

impl Node {
    fn deaf_now(&self, t: usize) -> bool {
        self.deaf.is_some_and(|(a, b)| t >= a && t < b)
    }
}

fn halves(pks: &[PublicKey], i: usize) -> (Vec<PublicKey>, Vec<PublicKey>) {
    let split = pks.len() / 2;
    let mut h1 = pks[..split].to_vec();
    let mut h2 = pks[split..].to_vec();
    h1.retain(|p| p != &pks[i]);
    h2.retain(|p| p != &pks[i]);
    (h1, h2)
}

fn dealer_of(msg: &Msg) -> Option<PublicKey> {
    match msg {
        Msg::Log { dealer, .. } => PublicKey::decode(dealer.as_slice()).ok(),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
fn run_case(
    ks: &[ed25519::PrivateKey],
    round: &Round,
    fault: Fault,
    byz: &[usize],
    seed: u64,
    prior: Option<&BTreeMap<PublicKey, Share>>,
) -> Case {
    let pks: Vec<PublicKey> = ks.iter().map(|k| k.public_key()).collect();
    let n = ks.len();
    let byz_set: BTreeSet<usize> = byz.iter().copied().collect();
    let target = n - 1;
    // The twin's log diet omits the first honest dealer, so its candidate
    // digest genuinely differs from the full bundle's.
    let blind_idx = (0..n).find(|i| !byz_set.contains(i)).unwrap_or(0);
    let mut net = Net { queue: VecDeque::new(), rng: ChaCha20Rng::seed_from_u64(seed ^ 0xc0de) };
    let mut nodes: Vec<Node> = Vec::with_capacity(n);
    for (i, k) in ks.iter().enumerate() {
        let share = prior.and_then(|p| p.get(&pks[i]).cloned());
        let twin_share = share.clone();
        let rng = node_seed(seed, i, round.round);
        let (ceremony, out) = Ceremony::start(rng, k.clone(), round.clone(), share).expect("ceremony starts");
        // knobs = (silent_from_start, silent_from, deaf, hold_log_until, restart_at)
        // Late behaviours block the target's inbound (deaf): a deal received
        // late makes the ack late, which is the real-world cause.
        let knobs: Knobs = match fault {
            Fault::SilentPlayer | Fault::SilentDealer => (byz_set.contains(&i), None, None, None, None),
            Fault::SilentAfterLog => (false, byz_set.contains(&i).then_some(PROPOSE_TICK), None, None, None),
            Fault::LateAckBeforeClose => (
                false, None, (i == target).then_some((0, CLOSE_TICK - 1)), None, None,
            ),
            Fault::LateAckPastClose => (
                false, None, (i == target).then_some((0, CLOSE_TICK + 10)), None, None,
            ),
            Fault::LateLogBeforeProposal => (
                false, None, None, (i == target).then_some(PROPOSE_TICK - 1), None,
            ),
            Fault::LateLogAfterProposal => (
                false, None, None, (i == target).then_some(PROPOSE_TICK + 5), None,
            ),
            Fault::LateDecisionPastGrace => (
                false, None,
                (i == target).then_some((PROPOSE_TICK + 1, PROPOSE_TICK + GRACE_TICKS + 40)),
                None, None,
            ),
            Fault::RestartDuringDealing => (false, None, None, None, (i == target).then_some(CLOSE_TICK - 8)),
            Fault::RestartDuringAgreement => (false, None, None, None, (i == target).then_some(PROPOSE_TICK + 5)),
            Fault::EquivocateLogs | Fault::ForgedMessages | Fault::EquivocateVotes | Fault::None => {
                (false, None, None, None, None)
            }
        };
        let twin = if fault == Fault::EquivocateVotes && byz_set.contains(&i) {
            Some(Ceremony::start(node_seed(seed, i, round.round), k.clone(), round.clone(), twin_share).expect("twin starts").0)
        } else {
            None
        };
        nodes.push(Node {
            key: k.clone(),
            ceremony,
            twin,
            twin_blind: (fault == Fault::EquivocateVotes && byz_set.contains(&i)).then(|| pks[blind_idx].clone()),
            twin_proposal: None,
            seen_deals: Vec::new(),
            held_log: Vec::new(),
            hold_log_until: knobs.3,
            deaf: knobs.2,
            silent_from_start: knobs.0,
            silent_from: knobs.1,
            restart_at: knobs.4,
            proposal: None,
        });
        // Silent players never send their opening deals either.
        let silent_start = nodes[i].silent_from_start;
        let initial = silent_start.then(Vec::new).unwrap_or(out);
        net.send(&pks[i], &pks, initial);
    }
    let mut case = Case { certified: BTreeMap::new(), staged: BTreeMap::new(), staged_at: BTreeMap::new(), logs: BTreeMap::new(), decision: None };
    let mut twin_closed = vec![false; n];
    let cap = match fault {
        Fault::SilentPlayer | Fault::LateAckPastClose => FAIL_CAP,
        _ => RUN_CAP,
    };
    for t in 0..cap {
        // Restarts: recreate with the same seed (stable dealer randomness)
        // and replay the recorded deals — the deal journal's behavior.
        for i in 0..n {
            if nodes[i].restart_at != Some(t) {
                continue;
            }
            let share = prior.and_then(|p| p.get(&pks[i]).cloned());
            let (mut ceremony, _) = Ceremony::start(node_seed(seed, i, round.round), nodes[i].key.clone(), round.clone(), share).expect("restart starts");
            for (from, deal) in nodes[i].seen_deals.clone() {
                let _ = ceremony.on_message(&from, deal);
            }
            nodes[i].ceremony = ceremony;
            nodes[i].proposal = None;
        }
        // Deliver everything queued this tick (deaf targets requeue).
        for _ in 0..net.queue.len() {
            let Some((from, to, msg)) = net.queue.pop_front() else { break };
            let Some(i) = pks.iter().position(|p| p == &to) else { continue };
            // Hoisted before the twin borrow below.
            let twin_blind_ref = if nodes[i].twin.is_some() { nodes[i].twin_blind.clone() } else { None };
            if nodes[i].deaf_now(t) {
                net.queue.push_back((from, to, msg));
                continue;
            }
            if matches!(msg, Msg::Deal { .. }) {
                nodes[i].seen_deals.push((from.clone(), msg.clone()));
            }
            if case.decision.is_none() && !byz_set.contains(&i) && matches!(msg, Msg::Agreement(AgreementMsg::Decision(_))) {
                case.decision = Some(msg.clone());
            }
            let out = nodes[i].ceremony.on_message(&from, msg.clone());
            // A silent node sends nothing — including on_message replies.
            let silent_now = nodes[i].silent_from_start || nodes[i].silent_from.is_some_and(|s| t >= s);
            if !silent_now {
                net.send(&to, &pks, out);
            }
            if let Some(twin) = nodes[i].twin.as_mut() {
                let blind = twin_blind_ref.clone();
                let blind_log = dealer_of(&msg).is_some_and(|d| Some(d) == blind);
                if !blind_log {
                    let _ = twin.on_message(&from, msg);
                }
            }
        }
        for i in 0..n {
            if nodes[i].silent_from_start || nodes[i].silent_from.is_some_and(|s| t >= s) {
                continue;
            }
            let mut out = nodes[i].ceremony.pending_deals();
            if nodes[i].ceremony.all_acked() || t > CLOSE_TICK {
                out.extend(nodes[i].ceremony.close_dealing());
            }
            // Equivocating dealers: broadcast the real log to one half and a
            // second valid log (fresh dealing randomness) to the other half.
            if fault == Fault::EquivocateLogs && byz_set.contains(&i) {
                let mut own: Vec<(To, Msg)> = Vec::new();
                out.retain(|(_, m)| {
                    let is_log = matches!(m, Msg::Log { .. });
                    if is_log {
                        own.push((To::All, m.clone()));
                    }
                    !is_log
                });
                if let Some((_, log_a)) = own.into_iter().next() {
                    let share = prior.and_then(|p| p.get(&pks[i]).cloned());
                    let (mut evil, _) = Ceremony::start(ChaCha20Rng::seed_from_u64(66_600 + seed + i as u64), nodes[i].key.clone(), round.clone(), share).expect("evil twin starts");
                    let log_b = evil.close_dealing().into_iter().find_map(|(_, m)| matches!(m, Msg::Log { .. }).then_some(m)).expect("evil dealer signs a log");
                    let (h1, h2) = halves(&pks, i);
                    for p in &h1 {
                        net.queue.push_back((pks[i].clone(), p.clone(), log_a.clone()));
                    }
                    for p in &h2 {
                        net.queue.push_back((pks[i].clone(), p.clone(), log_b.clone()));
                    }
                }
            }
            // Forged and malformed messages at proposal time: never fatal.
            if fault == Fault::ForgedMessages && byz_set.contains(&i) && t == PROPOSE_TICK + 2 {
                let signer = pks[i].encode().to_vec();
                let junk_vote = |view| SignedVote { view, phase: Phase::Precommit, digest: [0; 32], signer: signer.clone(), signature: vec![0; 64] };
                let junk = vec![
                    Msg::Done(vec![0u8; 32]),
                    Msg::Done(vec![1u8; 31]),
                    Msg::Done(Vec::new()),
                    Msg::Agreement(AgreementMsg::Vote(junk_vote(0))),
                    Msg::Agreement(AgreementMsg::Vote(junk_vote(100_000))),
                    Msg::Agreement(AgreementMsg::PrecommitCertificate(Certificate {
                        view: 0,
                        phase: Phase::Precommit,
                        digest: [0; 32],
                        votes: vec![junk_vote(0); 3],
                    })),
                    Msg::Transcript(vec![(vec![1; 32], vec![2; 96]); 64]),
                ];
                for msg in junk {
                    let victim = pks[(i + 1) % n].clone();
                    net.queue.push_back((pks[i].clone(), victim, msg));
                }
            }
            if nodes[i].ceremony.is_player() && t > PROPOSE_TICK && nodes[i].ceremony.have_quorum_logs() {
                if let Some((digest, msg)) = nodes[i].ceremony.propose_transcript() {
                    if nodes[i].proposal.as_ref() != Some(&digest) {
                        nodes[i].proposal = Some(digest);
                        out.push((To::All, msg));
                    }
                }
                out.extend(nodes[i].ceremony.tick_agreement());
            }
            for (_, m) in &out {
                match m {
                    Msg::Log { .. } => {
                        case.logs.entry(i).or_insert_with(|| m.clone());
                    }
                    Msg::Agreement(AgreementMsg::Decision(_)) if case.decision.is_none() => {
                        case.decision = Some(m.clone());
                    }
                    _ => {}
                }
            }
            if nodes[i].ceremony.is_player() && !case.staged.contains_key(&i) {
                if let Some(digest) = nodes[i].ceremony.certified_transcript() {
                    let mut finish_rng = ChaCha20Rng::seed_from_u64(7);
                    let (output, share) = nodes[i].ceremony.finish_decided(&mut finish_rng, &digest).expect("certified bundle finalizes");
                    case.certified.insert(i, digest);
                    case.staged.insert(i, KeyFile::new(round.round, &output, &share));
                    case.staged_at.insert(i, t);
                }
            }
            if t % REBROADCAST_EVERY == 0 {
                out.extend(nodes[i].ceremony.rebroadcast());
            }
            // Twin: proposes and votes its own digest to the other half.
            if fault == Fault::EquivocateVotes && byz_set.contains(&i) {
                let mut twin_out = Vec::new();
                let mut twin_proposal = nodes[i].twin_proposal.clone();
                if let Some(twin) = nodes[i].twin.as_mut() {
                    if t > CLOSE_TICK && !twin_closed[i] {
                        let _ = twin.close_dealing();
                        twin_closed[i] = true;
                    }
                    if t > PROPOSE_TICK && twin.have_quorum_logs() {
                        if let Some((digest, msg)) = twin.propose_transcript() {
                            if twin_proposal.as_ref() != Some(&digest) {
                                twin_proposal = Some(digest);
                                twin_out.push((To::All, msg));
                            }
                        }
                        twin_out.extend(twin.tick_agreement());
                    }
                }
                nodes[i].twin_proposal = twin_proposal;
                let (_, h2) = halves(&pks, i);
                for (_, msg) in twin_out {
                    for p in &h2 {
                        net.queue.push_back((pks[i].clone(), p.clone(), msg.clone()));
                    }
                }
            }
            // Hold schedules, then routing (vote equivocation splits the
            // agreement channel; everything else is full gossip).
            let mut flush = out;
            if nodes[i].hold_log_until.is_some_and(|h| t < h) {
                let (logs, rest): (Vec<_>, Vec<_>) = flush.into_iter().partition(|(_, m)| matches!(m, Msg::Log { .. }));
                nodes[i].held_log.extend(logs);
                flush = rest;
            } else {
                flush.splice(0..0, std::mem::take(&mut nodes[i].held_log));
            }
            if fault == Fault::EquivocateVotes && byz_set.contains(&i) {
                let (h1, _) = halves(&pks, i);
                let mut rest = Vec::new();
                for (to, msg) in flush {
                    if matches!(msg, Msg::Agreement(_)) {
                        for p in &h1 {
                            net.queue.push_back((pks[i].clone(), p.clone(), msg.clone()));
                        }
                    } else {
                        rest.push((to, msg));
                    }
                }
                net.send(&pks[i], &pks, rest);
            } else {
                net.send(&pks[i], &pks, flush);
            }
        }
        let complete = pks.iter().enumerate().all(|(i, _)| {
            !nodes[i].ceremony.is_player() || byz_set.contains(&i) || case.staged.contains_key(&i)
        });
        if complete {
            return case;
        }
    }
    case
}

/// S1 + S2 + S3 for a run whose honest players must all stage.
fn assert_one_output(case: &Case, n: u32, honest: &[usize]) {
    let digests: Vec<&Vec<u8>> = case.certified.values().collect();
    assert!(!digests.is_empty(), "the honest quorum must certify a transcript");
    assert!(
        digests.iter().all(|d| *d == digests[0]),
        "two different transcripts were certified: {:?}",
        case.certified
    );
    let staged: Vec<&KeyFile> = honest.iter().filter_map(|i| case.staged.get(i)).collect();
    assert_eq!(staged.len(), honest.len(), "every honest player staged (staged: {:?})", case.staged.keys().collect::<Vec<_>>());
    let canonical = staged[0].output.clone();
    assert!(staged.iter().all(|f| f.output == canonical), "staged outputs differ");
    let mut shares = Vec::new();
    for (pos, f) in staged.iter().enumerate() {
        let (output, share) = f.decode(n).expect("staged file decodes");
        assert!(output.revealed().is_empty(), "staged output {pos} reveals a seated share");
        assert!(
            aether_light::Scheme::signer(&aether_light::consensus_namespace(), output.players().clone(), output.public().clone(), share).is_some(),
            "staged share {pos} cannot sign under the output"
        );
        shares.push(f.share.clone());
    }
    shares.sort();
    shares.dedup();
    assert_eq!(shares.len(), staged.len(), "staged shares are distinct");
}

/// Nothing may be certified or staged from a bundle that reveals a seat.
fn assert_nothing_staged(case: &Case) {
    assert!(case.certified.is_empty(), "a revealing bundle was certified: {:?}", case.certified.keys().collect::<Vec<_>>());
    assert!(case.staged.is_empty(), "a revealing bundle was staged: {:?}", case.staged.keys().collect::<Vec<_>>());
}

fn files_by_key(case: &Case, pks: &[PublicKey]) -> Files {
    case.staged.iter().map(|(i, f)| (pks[*i].clone(), f.clone_fields())).collect()
}

/// `KeyFile` is not `Clone`; copy its (serializable) fields.
fn clone_fields_helper(f: &KeyFile) -> KeyFile {
    KeyFile { round: f.round, output: f.output.clone(), identity: f.identity.clone(), share: f.share.clone() }
}

trait KeyFileExt {
    fn clone_fields(&self) -> KeyFile;
}

impl KeyFileExt for KeyFile {
    fn clone_fields(&self) -> KeyFile {
        clone_fields_helper(self)
    }
}

fn shares_of(files: &Files, n: u32) -> BTreeMap<PublicKey, Share> {
    files.iter().map(|(pk, f)| (pk.clone(), f.decode(n).expect("share decodes").1)).collect()
}

/// A cached all-honest genesis per committee size (round 1), shared by the
/// reshare and readiness cases.
fn prior_files(n: usize) -> &'static (Vec<ed25519::PrivateKey>, Files) {
    static PRIORS: OnceLock<BTreeMap<usize, (Vec<ed25519::PrivateKey>, Files)>> = OnceLock::new();
    PRIORS.get_or_init(|| {
        let mut all = BTreeMap::new();
        for count in [4usize, 7] {
            let ks = keys(count as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let case = run_case(&ks, &Round::dkg(set, 1).with_chain_id(CHAIN), Fault::None, &[], 9_000 + count as u64, None);
            let pks: Vec<PublicKey> = ks.iter().map(|k| k.public_key()).collect();
            all.insert(count, (ks, files_by_key(&case, &pks)));
        }
        all
    }).get(&n).expect("cached committee size")
}

fn honest_of(n: usize, byz: &[usize]) -> Vec<usize> {
    (0..n).filter(|i| !byz.contains(i)).collect::<Vec<_>>()
}

/// Baseline: with no adversary, every player stages one usable output.
#[test]
fn healthy_committees_are_the_baseline() {
    for n in [4usize, 7] {
        for seed in [11u64, 12] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let case = run_case(&ks, &Round::dkg(set, 2).with_chain_id(CHAIN), Fault::None, &[], seed, None);
            assert_one_output(&case, n as u32, &(0..n).collect::<Vec<_>>());
        }
    }
}

/// S2/L1 with f silent players: their seats are revealed by every dealer
/// log, so no honest player may propose or stage anything; the attempt
/// fails cleanly and a fresh round (everyone present) recovers.
#[test]
fn silent_players_never_leak_into_a_staged_output() {
    for &(n, f) in &[(4usize, 1usize), (7, 2)] {
        for seed in [21u64, 22] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let byz: Vec<usize> = (n - f..n).collect();
            let failed = run_case(&ks, &Round::dkg(set.clone(), 3).with_chain_id(CHAIN), Fault::SilentPlayer, &byz, seed, None);
            assert_nothing_staged(&failed);
            // The dealer logs did flow: the run failed on the reveal gate,
            // not because nothing happened.
            assert!(failed.logs.len() >= n - f, "honest dealers published logs");
            // L1: the next attempt, with every seat present, must succeed.
            let retried = run_case(&ks, &Round::dkg(set, 4).with_chain_id(CHAIN), Fault::None, &[], seed + 50, None);
            assert_one_output(&retried, n as u32, &(0..n).collect::<Vec<_>>());
        }
    }
}

/// A silent departing dealer (not a seated player) leaves exactly a quorum
/// of dealing dealers; the new committee still stages usable shares.
#[test]
fn silent_departing_dealers_leave_a_working_quorum() {
    for &(n, f) in &[(4usize, 1usize), (7, 2)] {
        for seed in [31u64, 32] {
            let (old_keys, files) = prior_files(n);
            let previous = files.values().next().unwrap().decode_output(n as u32).expect("prior output");
            let new_keys: Vec<_> = ((n + 1) as u64..=2 * n as u64).map(aether_light::devnet_validator_key).collect();
            let mut ks = old_keys.clone();
            ks.extend(new_keys.iter().cloned());
            let new_set: Set<PublicKey> = new_keys.iter().map(|k| k.public_key()).try_collect().unwrap();
            let round = Round::reshare(previous, new_set, 2).with_chain_id(CHAIN);
            let byz: Vec<usize> = (0..f).collect();
            let shares = shares_of(files, n as u32);
            let case = run_case(&ks, &round, Fault::SilentDealer, &byz, seed, Some(&shares));
            let new_players: Vec<usize> = (n..2 * n).collect();
            assert_one_output(&case, n as u32, &new_players);
            // The same committee identity survives the rotation.
            let identity = files.values().next().unwrap().identity.clone();
            assert!(case.staged.values().all(|f| f.identity == identity), "identity unchanged");
        }
    }
}

/// A3-2/A4-2 shape: players that ack and publish their log, then stop, can
/// neither block certification nor change its outcome.
#[test]
fn players_silent_after_their_log_cannot_block_the_quorum() {
    for &(n, f) in &[(4usize, 1usize), (7, 2)] {
        for seed in [41u64, 42] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let byz: Vec<usize> = (n - f..n).collect();
            let case = run_case(&ks, &Round::dkg(set, 5).with_chain_id(CHAIN), Fault::SilentAfterLog, &byz, seed, None);
            assert_one_output(&case, n as u32, &honest_of(n, &byz));
        }
    }
}

/// Acks that land before the dealing window closes are included: the output
/// has no reveals and everyone stages.
#[test]
fn late_acks_before_the_dealing_close_are_included() {
    for n in [4usize, 7] {
        for seed in [51u64, 52] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let case = run_case(&ks, &Round::dkg(set, 6).with_chain_id(CHAIN), Fault::LateAckBeforeClose, &[], seed, None);
            assert_one_output(&case, n as u32, &(0..n).collect::<Vec<_>>());
        }
    }
}

/// Acks that land after dealing closed: the dealers' logs already reveal the
/// late seat, so nothing may be proposed or staged (S2), and only a fresh
/// round restores liveness (L1).
#[test]
fn late_acks_past_the_dealing_window_never_stage_a_revealing_bundle() {
    for n in [4usize, 7] {
        for seed in [61u64, 62] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let failed = run_case(&ks, &Round::dkg(set.clone(), 7).with_chain_id(CHAIN), Fault::LateAckPastClose, &[], seed, None);
            assert_nothing_staged(&failed);
            let retried = run_case(&ks, &Round::dkg(set, 8).with_chain_id(CHAIN), Fault::None, &[], seed + 50, None);
            assert_one_output(&retried, n as u32, &(0..n).collect::<Vec<_>>());
        }
    }
}

/// R2-1 shape: a dealer log delivered just before or just after the others
/// proposed changes candidate digests mid-flight. Agreement is on the
/// transcript digest, so everyone converges on one certified output.
#[test]
fn a_late_dealer_log_cannot_split_the_staged_output() {
    for fault in [Fault::LateLogBeforeProposal, Fault::LateLogAfterProposal] {
        for n in [4usize, 7] {
            for seed in [71u64, 72] {
                let ks = keys(n as u64);
                let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
                let case = run_case(&ks, &Round::dkg(set, 9).with_chain_id(CHAIN), fault, &[], seed, None);
                assert_one_output(&case, n as u32, &(0..n).collect::<Vec<_>>());
            }
        }
    }
}

/// L3: a player partitioned past the decision grace recovers the certified
/// bundle from the peers' bounded rebroadcast and stages the same output.
#[test]
fn a_player_late_past_the_grace_recovers_from_one_bounded_relay() {
    for n in [4usize, 7] {
        for seed in [81u64, 82] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let case = run_case(&ks, &Round::dkg(set, 10).with_chain_id(CHAIN), Fault::LateDecisionPastGrace, &[], seed, None);
            assert_one_output(&case, n as u32, &(0..n).collect::<Vec<_>>());
            let unblock = PROPOSE_TICK + GRACE_TICKS + 40;
            let late = n - 1;
            let at = case.staged_at[&late];
            assert!(at >= unblock, "the late player staged at {at} before its partition lifted");
            assert!(at <= unblock + RELAY_TICKS, "recovery exceeded the bounded relay window: {at}");
        }
    }
}

/// L2: an honest restart (fresh state, same seed, deals replayed — the
/// journal's contract) neither blocks the quorum nor diverges.
#[test]
fn an_honest_restart_never_blocks_or_conflicts() {
    for fault in [Fault::RestartDuringDealing, Fault::RestartDuringAgreement] {
        for n in [4usize, 7] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let case = run_case(&ks, &Round::dkg(set, 11).with_chain_id(CHAIN), fault, &[], 91, None);
            assert_one_output(&case, n as u32, &(0..n).collect::<Vec<_>>());
        }
    }
}

/// S6: a dealer that signs two different valid logs is excluded from the
/// certified bundle; the remaining dealers are exactly a quorum.
#[test]
fn equivocating_dealers_are_excluded_and_the_quorum_certifies() {
    for &(n, f) in &[(4usize, 1usize), (7, 2)] {
        for seed in [101u64, 102] {
            let ks = keys(n as u64);
            let pks: Vec<PublicKey> = ks.iter().map(|k| k.public_key()).collect();
            let set: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
            let byz: Vec<usize> = (n - f..n).collect();
            let case = run_case(&ks, &Round::dkg(set, 12).with_chain_id(CHAIN), Fault::EquivocateLogs, &byz, seed, None);
            assert_one_output(&case, n as u32, &honest_of(n, &byz));
            let (output, _) = case.staged.values().next().unwrap().decode(n as u32).unwrap();
            for &b in &byz {
                assert!(output.dealers().position(&pks[b]).is_none(), "equivocator {b} seated in the bundle");
            }
            assert_eq!(output.dealers().len(), n - f, "the honest dealers are exactly a quorum");
        }
    }
}

/// S1/S6: a Byzantine key running two ceremonies over two valid log diets
/// broadcasts two digests in the same views. Honest players count only one
/// vote per (view, phase) from that key, and still stage exactly one output.
#[test]
fn equivocating_votes_cannot_split_the_staged_output() {
    for &(n, f) in &[(4usize, 1usize), (7, 2)] {
        for seed in [111u64, 112] {
            let ks = keys(n as u64);
            let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
            let byz: Vec<usize> = (n - f..n).collect();
            let case = run_case(&ks, &Round::dkg(set, 13).with_chain_id(CHAIN), Fault::EquivocateVotes, &byz, seed, None);
            assert_one_output(&case, n as u32, &honest_of(n, &byz));
        }
    }
}

/// A3-2: arbitrary `Done`s, malformed votes, a forged certificate and an
/// oversized transcript never abort the honest quorum.
#[test]
fn forged_done_and_malformed_messages_never_abort_the_ceremony() {
    for n in [4usize, 7] {
        let ks = keys(n as u64);
        let set: Set<PublicKey> = ks.iter().map(|k| k.public_key()).try_collect().unwrap();
        let byz = vec![n - 1];
        let case = run_case(&ks, &Round::dkg(set, 14).with_chain_id(CHAIN), Fault::ForgedMessages, &byz, 121, None);
        assert_one_output(&case, n as u32, &honest_of(n, &byz));
    }
}

/// S5: a prior round's signed log and decision certificate must not count
/// in the next round, which still completes on its own digest.
#[test]
fn replayed_prior_round_messages_are_ignored() {
    for n in [4usize, 7] {
        let ks = keys(n as u64);
        let pks: Vec<PublicKey> = ks.iter().map(|k| k.public_key()).collect();
        let set: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
        let first = run_case(&ks, &Round::dkg(set.clone(), 20).with_chain_id(CHAIN), Fault::None, &[], 131, None);
        assert_one_output(&first, n as u32, &(0..n).collect::<Vec<_>>());
        let second_round = Round::dkg(set, 21).with_chain_id(CHAIN);
        let (mut probe, _) = Ceremony::start(node_seed(131, 0, 21), ks[0].clone(), second_round.clone(), None).unwrap();
        for (i, log) in &first.logs {
            probe.on_message(&pks[*i], log.clone());
        }
        assert_eq!(probe.log_count(), 0, "a prior round's signed log must not count");
        if let Some(decision) = &first.decision {
            probe.on_message(&pks[0], decision.clone());
            assert!(probe.certified_transcript().is_none(), "a prior round's decision must not decide this round");
        }
        let second = run_case(&ks, &second_round, Fault::None, &[], 131, None);
        assert_one_output(&second, n as u32, &(0..n).collect::<Vec<_>>());
        let old_digest = first.certified.values().next().unwrap();
        let new_digest = second.certified.values().next().unwrap();
        assert_ne!(old_digest, new_digest, "the rounds must not share a transcript digest");
    }
}

/// S5: another chain's valid log and decision (same roster, same round
/// number) must not count on this chain, which still completes.
#[test]
fn another_chains_valid_messages_are_ignored() {
    for n in [4usize, 7] {
        let ks = keys(n as u64);
        let pks: Vec<PublicKey> = ks.iter().map(|k| k.public_key()).collect();
        let set: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
        let other = run_case(&ks, &Round::dkg(set.clone(), 22).with_chain_id(OTHER_CHAIN), Fault::None, &[], 141, None);
        assert_one_output(&other, n as u32, &(0..n).collect::<Vec<_>>());
        let round = Round::dkg(set, 22).with_chain_id(CHAIN);
        let (mut probe, _) = Ceremony::start(node_seed(141, 0, 22), ks[0].clone(), round.clone(), None).unwrap();
        for (i, log) in &other.logs {
            probe.on_message(&pks[*i], log.clone());
        }
        assert_eq!(probe.log_count(), 0, "another chain's signed log must not count");
        if let Some(decision) = &other.decision {
            probe.on_message(&pks[0], decision.clone());
            assert!(probe.certified_transcript().is_none(), "another chain's decision must not decide this chain");
        }
        let case = run_case(&ks, &round, Fault::None, &[], 141, None);
        assert_one_output(&case, n as u32, &(0..n).collect::<Vec<_>>());
    }
}

/// L4/S3 (A5-2): seats that withhold readiness proofs cannot be certified.
/// A four-seat committee refuses to retry below four seats (it keeps the
/// current committee); a seven-seat committee retries with the five ready
/// seats on a fresh round, and only that reduced handoff verifies.
#[test]
fn withheld_readiness_blocks_the_handoff_and_a_reduced_roster_recovers() {
    for &(n, f) in &[(4usize, 1usize), (7, 2)] {
        let ks = keys(n as u64);
        let pks: Vec<PublicKey> = ks.iter().map(|k| k.public_key()).collect();
        let set: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
        let case = run_case(&ks, &Round::dkg(set, 30).with_chain_id(CHAIN), Fault::None, &[], 151, None);
        assert_one_output(&case, n as u32, &(0..n).collect::<Vec<_>>());
        let output = case.staged.values().next().unwrap().output.clone();
        // Resharing preserves the constant group identity, so the round-30
        // genesis output pins it for every later handoff check here.
        let identity = *case.staged.values().next().unwrap().decode_output(n as u32).unwrap().public().public();
        let members: Vec<(String, String)> = pks.iter().map(|k| (hex::encode(k.encode()), "node".to_string())).collect();
        let withholders: Vec<usize> = (n - f..n).collect();
        let mut readiness = aether_node::handoff::Readiness { round: 30, output: output.clone(), members: members.clone(), proofs: Default::default() };
        for (i, pk) in pks.iter().enumerate() {
            if withholders.contains(&i) {
                continue; // Byzantine seats never prove readiness.
            }
            let share = case.staged[&i].decode(n as u32).unwrap().1;
            let member = hex::encode(pk.encode());
            let proof = aether_node::handoff::sign_ready(CHAIN, 30, &output, &members, &share);
            let complete = readiness.accept_proof(CHAIN, &member, &proof).expect("honest proof verifies");
            assert!(!complete, "a handoff with {f} unproven seat(s) must not complete");
        }
        // A handoff naming every seat without every proof never verifies.
        let partial: Vec<String> = (0..n).filter(|i| !withholders.contains(i)).map(|i| {
            let share = case.staged[&i].decode(n as u32).unwrap().1;
            aether_node::handoff::sign_ready(CHAIN, 30, &output, &members, &share)
        }).collect();
        let unready_handoff = aether_light::block::Handoff {
            round: 30,
            output: output.clone(),
            members: members.clone(),
            ready: partial,
            signature: String::new(),
        };
        assert!(aether_node::handoff::verify_output(CHAIN, &identity, &unready_handoff).is_err(), "an incomplete-ready handoff verified");

        if n == 4 {
            // Three ready seats are below the four-seat floor: the verdict is
            // an error and the committee keeps signing with the old roster.
            assert!(readiness.retry_members(CHAIN).is_err(), "three seats are too few to retry");
            continue;
        }
        let reduced = readiness.retry_members(CHAIN).expect("retry verdict").expect("five ready seats retry");
        assert_eq!(reduced.len(), 5);
        for &w in &withholders {
            assert!(!reduced.iter().any(|(key, _)| key.eq_ignore_ascii_case(&hex::encode(pks[w].encode()))));
        }
        // The fresh reduced round runs one instance per ready seat: those
        // seats hold the old shares (they deal — five of seven old dealers
        // is a dealing quorum) and seat the new five-player committee.
        let ready_idx: Vec<usize> = (0..n).filter(|i| !withholders.contains(i)).collect();
        let retry_keys: Vec<ed25519::PrivateKey> = ready_idx.iter().map(|&i| ks[i].clone()).collect();
        let reduced_pks: Vec<PublicKey> = retry_keys.iter().map(|k| k.public_key()).collect();
        let reduced_set: Set<PublicKey> = reduced_pks.iter().cloned().try_collect().unwrap();
        let previous = case.staged.values().next().unwrap().decode_output(n as u32).unwrap();
        let retry_round = Round::reshare(previous, reduced_set, 31).with_chain_id(CHAIN);
        let prior = shares_of(&files_by_key(&case, &pks), n as u32);
        let retry = run_case(&retry_keys, &retry_round, Fault::None, &[], 161, Some(&prior));
        let new_players: Vec<usize> = (0..5).collect();
        assert_one_output(&retry, 5, &new_players);
        let retry_output = retry.staged.values().next().unwrap().output.clone();
        let ready: Vec<String> = new_players.iter().map(|&i| {
            let share = retry.staged[&i].decode(5).unwrap().1;
            aether_node::handoff::sign_ready(CHAIN, 31, &retry_output, &reduced, &share)
        }).collect();
        let handoff = aether_light::block::Handoff {
            round: 31,
            output: retry_output,
            members: reduced,
            ready,
            signature: String::new(),
        };
        aether_node::handoff::verify_output(CHAIN, &identity, &handoff).expect("the fully proved reduced handoff verifies");
    }
}
