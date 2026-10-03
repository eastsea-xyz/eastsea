//! DKG ceremony over a lossy, reordering in-memory network.

use aether_node::block::PublicKey;
use aether_node::dkg::{Ceremony, KeyFile, Msg, Round, To};
use commonware_codec::Encode;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_utils::{ordered::Set, TryCollect};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha20Rng;
use std::collections::VecDeque;

fn keys(n: u64) -> Vec<ed25519::PrivateKey> {
    (1..=n).map(aether_light::devnet_validator_key).collect()
}

struct Net {
    queue: VecDeque<(PublicKey, PublicKey, Msg)>,
    rng: ChaCha20Rng,
    drop_rate: f64,
}

impl Net {
    fn send(&mut self, from: &PublicKey, peers: &[PublicKey], out: Vec<(To, Msg)>) {
        for (to, msg) in out {
            let targets: Vec<PublicKey> = match to {
                To::One(p) => vec![p],
                To::All => peers.iter().filter(|p| *p != from).cloned().collect(),
            };
            for t in targets {
                if self.rng.random_bool(self.drop_rate) {
                    continue;
                }
                // Random position: reordering.
                let at = self.rng.random_range(0..=self.queue.len());
                self.queue.insert(at, (from.clone(), t, msg.clone()));
            }
        }
    }
}

/// Run to completion; `tamper` may inject extra messages after dealing.
fn run(n: u64, seed: u64, drop_rate: f64, tamper: impl Fn(&mut Net, &[PublicKey])) -> Vec<KeyFile> {
    let ks = keys(n);
    let pks: Vec<PublicKey> = ks.iter().map(|k| k.public_key()).collect();
    let participants: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
    let mut net = Net { queue: VecDeque::new(), rng: ChaCha20Rng::seed_from_u64(seed), drop_rate };
    let mut cs = Vec::new();
    for (i, k) in ks.iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(seed * 100 + i as u64), k.clone(), Round::dkg(participants.clone(), 0), None).unwrap();
        net.send(&pks[i], &pks, out);
        cs.push(c);
    }
    let idx = |p: &PublicKey| pks.iter().position(|x| x == p).unwrap();
    let mut outputs: Vec<Option<KeyFile>> = (0..n).map(|_| None).collect();
    let mut tampered = false;
    for tick in 0..400 {
        // Deliver everything queued this tick.
        for _ in 0..net.queue.len() {
            let Some((from, to, msg)) = net.queue.pop_front() else { break };
            let i = idx(&to);
            let out = cs[i].on_message(&from, msg);
            net.send(&to, &pks, out);
        }
        for i in 0..n as usize {
            // Timers: re-send, close dealing, finish, announce.
            let mut out = cs[i].pending_deals();
            if cs[i].all_acked() || tick > 20 {
                out.extend(cs[i].close_dealing());
            }
            if !tampered && tick == 25 {
                tampered = true;
                tamper(&mut net, &pks);
            }
            if outputs[i].is_none() && cs[i].have_all_logs() && tick > 30 {
                let (o, s) = cs[i].finish(&mut ChaCha20Rng::seed_from_u64(7)).unwrap();
                outputs[i] = Some(KeyFile::new(0, &o, &s));
            }
            out.extend(cs[i].rebroadcast());
            net.send(&pks[i], &pks, out);
        }
        if cs.iter().all(|c| c.agreement(false) == Some(true)) {
            return outputs.into_iter().map(Option::unwrap).collect();
        }
        assert!(cs.iter().all(|c| c.agreement(false) != Some(false)), "identities disagree");
    }
    panic!("ceremony did not complete");
}

fn check(files: &[KeyFile], n: u32) {
    let id = &files[0].identity;
    assert!(files.iter().all(|f| &f.identity == id), "same committee identity everywhere");
    let shares: Vec<_> = files.iter().map(|f| f.share.clone()).collect();
    let mut uniq = shares.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), shares.len(), "distinct shares");
    // Each share signs for the consensus scheme under the common polynomial.
    for (i, f) in files.iter().enumerate() {
        let (output, share) = f.decode(n).unwrap();
        let signer = aether_light::Scheme::signer(&aether_light::consensus_namespace(), output.players().clone(), output.public().clone(), share);
        assert!(signer.is_some(), "share {i} matches the public polynomial");
    }
}

#[test]
fn four_validators_agree_despite_loss_and_reordering() {
    for seed in 1..=3 {
        let files = run(4, seed, 0.3, |_, _| {});
        check(&files, 4);
    }
}

#[test]
fn equivocating_dealer_is_excluded_by_everyone() {
    // Validator 4 also signs a second, different log (from a second dealing)
    // and shows it to validator 1 only; relaying spreads it to all.
    let files = run(4, 9, 0.0, |net, pks| {
        let participants: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
        let (mut evil, _) = Ceremony::start(ChaCha20Rng::seed_from_u64(666), keys(4)[3].clone(), Round::dkg(participants, 0), None).unwrap();
        for (_, msg) in evil.close_dealing() {
            if let Msg::Log { .. } = msg {
                net.queue.push_back((pks[3].clone(), pks[0].clone(), msg));
                break;
            }
        }
    });
    check(&files, 4);
    let (output, _) = files[0].decode(4).unwrap();
    assert!(!output.dealers().position(&keys(4)[3].public_key()).is_some(), "equivocating dealer excluded");
    assert_eq!(output.dealers().len(), 3);
}

/// Generic driver: `online` keys run `round` (those not online never start).
fn run_round(
    online: &[ed25519::PrivateKey],
    round: Round,
    shares: &std::collections::BTreeMap<PublicKey, commonware_cryptography::bls12381::primitives::group::Share>,
    seed: u64,
    drop_rate: f64,
) -> std::collections::BTreeMap<PublicKey, KeyFile> {
    let pks: Vec<PublicKey> = online.iter().map(|k| k.public_key()).collect();
    let mut net = Net { queue: VecDeque::new(), rng: ChaCha20Rng::seed_from_u64(seed), drop_rate };
    let mut cs = Vec::new();
    for (i, k) in online.iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(seed * 31 + i as u64), k.clone(), round.clone(), shares.get(&pks[i]).cloned()).unwrap();
        net.send(&pks[i], &pks, out);
        cs.push(c);
    }
    let idx = |p: &PublicKey| pks.iter().position(|x| x == p);
    let mut files = std::collections::BTreeMap::new();
    for tick in 0..600 {
        for _ in 0..net.queue.len() {
            let Some((from, to, msg)) = net.queue.pop_front() else { break };
            if let Some(i) = idx(&to) {
                let out = cs[i].on_message(&from, msg);
                net.send(&to, &pks, out);
            }
        }
        for i in 0..cs.len() {
            let mut out = cs[i].pending_deals();
            if cs[i].all_acked() || tick > 20 {
                out.extend(cs[i].close_dealing());
            }
            let enough = cs[i].have_all_logs() || (tick > 60 && cs[i].have_quorum_logs());
            if cs[i].is_player() && !files.contains_key(&pks[i]) && enough && tick > 30 {
                let (o, s) = cs[i].finish(&mut ChaCha20Rng::seed_from_u64(7)).unwrap();
                files.insert(pks[i].clone(), KeyFile::new(round.round, &o, &s));
            }
            out.extend(cs[i].rebroadcast());
            net.send(&pks[i], &pks, out);
        }
        assert!(cs.iter().all(|c| c.agreement(false) != Some(false)), "identities disagree");
        if cs.iter().all(|c| c.agreement(false) == Some(true)) {
            return files;
        }
    }
    panic!("round did not complete");
}

fn dkg4() -> (Vec<ed25519::PrivateKey>, std::collections::BTreeMap<PublicKey, KeyFile>) {
    let ks: Vec<ed25519::PrivateKey> = (1..=5).map(aether_light::devnet_validator_key).collect();
    let first: Set<PublicKey> = ks[..4].iter().map(|k| k.public_key()).try_collect().unwrap();
    let files = run_round(&ks[..4], Round::dkg(first, 0), &Default::default(), 11, 0.2);
    (ks, files)
}

fn shares_of(
    files: &std::collections::BTreeMap<PublicKey, KeyFile>,
    n: u32,
) -> std::collections::BTreeMap<PublicKey, commonware_cryptography::bls12381::primitives::group::Share> {
    files.iter().map(|(pk, f)| (pk.clone(), f.decode(n).unwrap().1)).collect()
}

#[test]
fn reshare_rotates_a_validator_and_keeps_the_identity() {
    let (ks, files) = dkg4();
    let identity = files.values().next().unwrap().identity.clone();
    let (previous, _) = files.values().next().unwrap().decode(4).unwrap();
    // Validator 1 leaves, validator 5 joins.
    let next: Set<PublicKey> = ks[1..5].iter().map(|k| k.public_key()).try_collect().unwrap();
    let round = Round::reshare(previous, next.clone(), 1);
    let out = run_round(&ks, round, &shares_of(&files, 4), 12, 0.2);
    assert_eq!(out.len(), 4, "every new member got a share");
    assert!(!out.contains_key(&ks[0].public_key()), "the leaving validator gets none");
    assert!(out.values().all(|f| f.identity == identity), "identity unchanged: wallets keep working");
    for (pk, f) in &out {
        let (o, s) = f.decode(4).unwrap();
        assert_eq!(o.players(), &next);
        assert!(aether_light::Scheme::signer(&aether_light::consensus_namespace(), o.players().clone(), o.public().clone(), s).is_some(), "{pk:?} can sign");
        // Fresh shares: the old share of a continuing member no longer matches.
        if let Some(old) = files.get(pk) {
            assert_ne!(old.share, f.share, "shares are refreshed");
        }
    }
}

#[test]
fn reshare_completes_with_an_old_validator_offline() {
    let (ks, files) = dkg4();
    let identity = files.values().next().unwrap().identity.clone();
    let (previous, _) = files.values().next().unwrap().decode(4).unwrap();
    // Validator 1 is gone for good (never starts); 2..4 reshare among themselves plus 5.
    let next: Set<PublicKey> = ks[1..5].iter().map(|k| k.public_key()).try_collect().unwrap();
    let out = run_round(&ks[1..5], Round::reshare(previous, next, 1), &shares_of(&files, 4), 13, 0.0);
    assert_eq!(out.len(), 4);
    assert!(out.values().all(|f| f.identity == identity));
}

/// The running committee hands the key to a new voting set: its threshold
/// signature on the handoff verifies under the unchanged identity; a forged
/// roster, a missing quorum or another committee's output does not.
#[test]
fn a_handoff_is_signed_by_the_running_committee() {
    use aether_node::handoff::{check_partial, combine, sign_partial, verify};
    use commonware_codec::Encode;
    let (ks, files) = dkg4();
    let (previous, _) = files.values().next().unwrap().decode(4).unwrap();
    let identity = *previous.public().public();
    let next: Set<PublicKey> = ks[1..5].iter().map(|k| k.public_key()).try_collect().unwrap();
    let out = run_round(&ks, Round::reshare(previous.clone(), next, 1), &shares_of(&files, 4), 14, 0.0);
    let new_file = out.values().next().unwrap();
    let members: Vec<(String, String)> = ks[1..5].iter().map(|k| (hex::encode(k.public_key().encode()), "node".to_string())).collect();
    let h = aether_light::block::Handoff { round: 1, output: new_file.output.clone(), members: members.clone(), signature: String::new() };
    const CHAIN: u64 = 7;

    // Old shares sign; any three of four combine.
    let old = shares_of(&files, 4);
    let partials: Vec<_> = old.values().map(|s| check_partial(CHAIN, previous.public(), &h, &sign_partial(CHAIN, &h, s)).unwrap()).collect();
    assert!(combine(previous.public(), &h, &partials[..2]).is_err(), "two of four is not a quorum");
    let signed = combine(previous.public(), &h, &partials[..3]).unwrap();
    verify(CHAIN, &identity, &signed).unwrap();

    // Another chain, another roster, or a signature over something else: rejected.
    assert!(verify(CHAIN + 1, &identity, &signed).is_err());
    let mut forged = signed.clone();
    forged.members[0].0 = hex::encode(ks[0].public_key().encode());
    assert!(verify(CHAIN, &identity, &forged).is_err(), "roster changed after signing");
    let mut other_nodes = signed.clone();
    other_nodes.members[0].1 = "elsewhere".into();
    assert!(verify(CHAIN, &identity, &other_nodes).is_err(), "node ids are signed too");
    // A share of the new sharing is not a running-committee share.
    let new_share = new_file.decode(4).unwrap().1;
    assert!(check_partial(CHAIN, previous.public(), &h, &sign_partial(CHAIN, &h, &new_share)).is_err());
}

/// A player drawn into the new set that never shows up does not stop the
/// reshare: the dealers reveal its share, a quorum of players agrees, and the
/// others hold working shares of the same identity.
#[test]
fn reshare_completes_without_an_unreachable_new_player() {
    let (ks, files) = dkg4();
    let identity = files.values().next().unwrap().identity.clone();
    let (previous, _) = files.values().next().unwrap().decode(4).unwrap();
    let extra = aether_light::devnet_validator_key(6);
    // New set: 2, 3, 4, 5 and 6; validator 6 never starts.
    let next: Set<PublicKey> = ks[1..5].iter().map(|k| k.public_key()).chain([extra.public_key()]).try_collect().unwrap();
    let round = Round::reshare(previous, next, 1);
    let online: Vec<ed25519::PrivateKey> = ks.clone();
    let pks: Vec<PublicKey> = online.iter().map(|k| k.public_key()).collect();
    let shares = shares_of(&files, 4);
    let mut net = Net { queue: VecDeque::new(), rng: ChaCha20Rng::seed_from_u64(21), drop_rate: 0.0 };
    let mut cs = Vec::new();
    for (i, k) in online.iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(900 + i as u64), k.clone(), round.clone(), shares.get(&pks[i]).cloned()).unwrap();
        net.send(&pks[i], &pks, out);
        cs.push(c);
    }
    let idx = |p: &PublicKey| pks.iter().position(|x| x == p);
    let mut files_out = std::collections::BTreeMap::new();
    for tick in 0..400 {
        for _ in 0..net.queue.len() {
            let Some((from, to, msg)) = net.queue.pop_front() else { break };
            if let Some(i) = idx(&to) {
                let out = cs[i].on_message(&from, msg);
                net.send(&to, &pks, out);
            }
        }
        for i in 0..cs.len() {
            let mut out = cs[i].pending_deals();
            if cs[i].all_acked() || tick > 20 {
                out.extend(cs[i].close_dealing());
            }
            if cs[i].is_player() && !files_out.contains_key(&pks[i]) && cs[i].have_all_logs() && tick > 30 {
                let (o, s) = cs[i].finish(&mut ChaCha20Rng::seed_from_u64(7)).unwrap();
                files_out.insert(pks[i].clone(), KeyFile::new(1, &o, &s));
            }
            out.extend(cs[i].rebroadcast());
            net.send(&pks[i], &pks, out);
        }
        let late = tick > 60;
        assert!(cs.iter().all(|c| c.agreement(late) != Some(false)));
        if !late {
            assert!(!cs.iter().all(|c| c.agreement(false) == Some(true)), "cannot fully agree without player 6");
        }
        if late && cs.iter().all(|c| c.agreement(true) == Some(true)) {
            assert_eq!(files_out.len(), 4, "the four reachable new players hold shares");
            assert!(files_out.values().all(|f| f.identity == identity));
            for f in files_out.values() {
                let (o, s) = f.decode(5).unwrap();
                assert!(aether_light::Scheme::signer(&aether_light::consensus_namespace(), o.players().clone(), o.public().clone(), s).is_some());
            }
            return;
        }
    }
    panic!("the reshare did not complete without the unreachable player");
}

/// Signed logs can be delivered to different quorums before the finish timer.
/// A mismatched output must never pass the stage gate; a fresh attempt after
/// delivery recovers and all staged shares verify under its public sharing.
fn split_logs_then_restart(first_count: usize, first_missing: &[usize], second_missing: &[usize]) {
    let (ks, files) = dkg4();
    let pks: Vec<_> = ks[..4].iter().map(|k| k.public_key()).collect();
    let (previous, _) = files.values().next().unwrap().decode(4).unwrap();
    let players: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
    let round = Round::reshare(previous, players, 42);
    let shares = shares_of(&files, 4);
    let mut cs = Vec::new();
    let mut queue = VecDeque::new();
    for (i, key) in ks[..4].iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(100 + i as u64), key.clone(), round.clone(), Some(shares[&pks[i]].clone())).unwrap();
        for (to, msg) in out {
            if let To::One(peer) = to { queue.push_back((i, pks.iter().position(|p| p == &peer).unwrap(), msg)); }
        }
        cs.push(c);
    }
    // Deliver every deal and acknowledgement before closing dealers.
    while let Some((from, to, msg)) = queue.pop_front() {
        for (dest, reply) in cs[to].on_message(&pks[from], msg) {
            if let To::One(peer) = dest { queue.push_back((to, pks.iter().position(|p| p == &peer).unwrap(), reply)); }
        }
    }
    let logs: Vec<_> = cs.iter_mut().map(|c| c.close_dealing().into_iter().find_map(|(_, m)| matches!(m, Msg::Log { .. }).then_some(m)).unwrap()).collect();
    for (to, ceremony) in cs.iter_mut().enumerate() {
        for (from, log) in logs.iter().enumerate() {
            let missing = if to < first_count { first_missing } else { second_missing };
            if !missing.contains(&from) && to != from { ceremony.on_message(&pks[from], log.clone()); }
        }
    }
    let mut output = Vec::new();
    for c in &mut cs {
        assert!(c.have_quorum_logs());
        output.push(c.finish(&mut ChaCha20Rng::seed_from_u64(7)).unwrap());
    }
    assert_ne!(output[0].0.encode().to_vec(), output[first_count].0.encode().to_vec(), "schedule must split the public output");
    // Only the announcements are delivered; the delayed logs remain withheld.
    let dones: Vec<_> = cs.iter().map(|c| c.rebroadcast().into_iter().find_map(|(_, m)| matches!(m, Msg::Done(_)).then_some(m)).unwrap()).collect();
    for (to, ceremony) in cs.iter_mut().enumerate() {
        for (from, done) in dones.iter().enumerate() {
            if from != to { ceremony.on_message(&pks[from], done.clone()); }
        }
    }
    assert!(cs.iter().all(|c| c.agreement(true) != Some(true)), "no split output may be staged");

    // The live supervisor retries a failed ceremony. On the next attempt all
    // logs arrive; the output and every post-switch partial agree.
    let staged = run_round(&ks[..4], round, &shares, 43, 0.0);
    assert_eq!(staged.len(), 4);
    let (canonical, _) = staged.values().next().unwrap().decode(4).unwrap();
    let mut partials = Vec::new();
    for f in staged.values() {
        let (out, share) = f.decode(4).unwrap();
        assert_eq!(out.encode().to_vec(), canonical.encode().to_vec());
        let partial = aether_node::handoff::sign_seed_partial(7781, 9, &share);
        partials.push(aether_node::handoff::check_seed_partial(7781, canonical.public(), 9, &partial).unwrap());
    }
    let seed = aether_node::handoff::combine_seed(canonical.public(), 9, &partials[..3]).unwrap();
    aether_node::handoff::verify_seed(7781, canonical.public().public(), &seed).unwrap();
}

#[test]
fn three_to_one_log_split_cannot_stage_and_restart_recovers() {
    // First two plus player 3 see four logs; player 4 sees only three.
    split_logs_then_restart(3, &[], &[0]);
}

#[test]
fn two_to_two_log_split_cannot_stage_and_restart_recovers() {
    split_logs_then_restart(2, &[3], &[0]);
}
