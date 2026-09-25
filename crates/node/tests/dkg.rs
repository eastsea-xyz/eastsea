//! DKG ceremony over a lossy, reordering in-memory network.

use aether_node::block::PublicKey;
use aether_node::dkg::{Ceremony, KeyFile, Msg, Round, To};
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
        if cs.iter().all(|c| c.agreement() == Some(true)) {
            return outputs.into_iter().map(Option::unwrap).collect();
        }
        assert!(cs.iter().all(|c| c.agreement() != Some(false)), "identities disagree");
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
        assert!(cs.iter().all(|c| c.agreement() != Some(false)), "identities disagree");
        if cs.iter().all(|c| c.agreement() == Some(true)) {
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
