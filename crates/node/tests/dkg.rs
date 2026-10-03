//! DKG ceremony over a lossy, reordering in-memory network.

use aether_node::block::PublicKey;
use aether_node::dkg::{Ceremony, KeyFile, Msg, Round, To};
use aether_node::dkg_agreement::{AgreementMsg, Phase, SignedVote};
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
    let mut proposals = vec![None; n as usize];
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
            if tick > 60 && cs[i].have_quorum_logs() {
                if let Some((digest, msg)) = cs[i].propose_transcript() {
                    if proposals[i].as_ref() != Some(&digest) {
                        proposals[i] = Some(digest);
                        out.push((To::All, msg));
                    }
                }
                out.extend(cs[i].tick_agreement());
            }
            if outputs[i].is_none() {
                if let Some(digest) = cs[i].certified_transcript() {
                    let (o, s) = cs[i].finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &digest).unwrap();
                    outputs[i] = Some(KeyFile::new(0, &o, &s));
                }
            }
            out.extend(cs[i].rebroadcast());
            net.send(&pks[i], &pks, out);
        }
        if outputs.iter().all(Option::is_some) {
            return outputs.into_iter().map(Option::unwrap).collect();
        }
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
        assert!(files[0].decode(4).unwrap().0.revealed().is_empty());
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
    let mut proposals = vec![None; cs.len()];
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
            if cs[i].is_player() && tick > 60 && cs[i].have_quorum_logs() {
                if let Some((digest, msg)) = cs[i].propose_transcript() {
                    if proposals[i].as_ref() != Some(&digest) {
                        proposals[i] = Some(digest);
                        out.push((To::All, msg));
                    }
                }
            }
            if tick > 60 { out.extend(cs[i].tick_agreement()); }
            if cs[i].is_player() && !files.contains_key(&pks[i]) {
                if let Some(digest) = cs[i].certified_transcript() {
                    let (o, s) = cs[i].finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &digest).unwrap();
                    files.insert(pks[i].clone(), KeyFile::new(round.round, &o, &s));
                }
            }
            out.extend(cs[i].rebroadcast());
            net.send(&pks[i], &pks, out);
        }
        if pks.iter().enumerate().filter(|(i, _)| cs[*i].is_player()).all(|(_, pk)| files.contains_key(pk)) {
            return files;
        }
    }
    panic!("round did not complete");
}

fn dkg4() -> (Vec<ed25519::PrivateKey>, std::collections::BTreeMap<PublicKey, KeyFile>) {
    let ks: Vec<ed25519::PrivateKey> = (1..=5).map(aether_light::devnet_validator_key).collect();
    let first: Set<PublicKey> = ks[..4].iter().map(|k| k.public_key()).try_collect().unwrap();
    let files = run_round(&ks[..4], Round::dkg(first, 0), &Default::default(), 11, 0.2);
    assert_eq!(
        blake3::hash(&hex::decode(&files.values().next().unwrap().output).unwrap()).to_hex().to_string(),
        "d4d640dd887badd0590e992adeb4230a91f4bfc3cae786b8301f42222ac53163",
        "the all-honest genesis output is unchanged",
    );
    assert!(files.values().next().unwrap().decode(4).unwrap().0.revealed().is_empty());
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
    assert_eq!(
        blake3::hash(&hex::decode(&out.values().next().unwrap().output).unwrap()).to_hex().to_string(),
        "ca3965a8b58f4a8259bbcf4747fe102b00af9afab1f49f10a716fc96e10b9d7c",
        "the all-honest reshare output is unchanged",
    );
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

/// A player misses the dealing/ack window. Even a valid three-dealer bundle
/// must not become vote-eligible when those logs reveal the fourth share.
/// Once that player returns, a fresh round with fresh dealer randomness works.
#[test]
fn delayed_ack_revealed_share_requires_fresh_round() {
    let ks = keys(4);
    let pks: Vec<_> = ks.iter().map(|key| key.public_key()).collect();
    let roster: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
    let mut ceremonies = Vec::new();
    let mut queue = VecDeque::new();
    for (i, key) in ks.iter().enumerate() {
        let (ceremony, outbound) = Ceremony::start(
            ChaCha20Rng::seed_from_u64(400 + i as u64),
            key.clone(),
            Round::dkg(roster.clone(), 80).with_chain_id(7781),
            None,
        ).unwrap();
        for (recipient, message) in outbound {
            let To::One(recipient) = recipient else { continue };
            let to = pks.iter().position(|pk| *pk == recipient).unwrap();
            if i < 3 && to < 3 { queue.push_back((i, to, message)); }
        }
        ceremonies.push(ceremony);
    }
    while let Some((from, to, message)) = queue.pop_front() {
        for (recipient, reply) in ceremonies[to].on_message(&pks[from], message) {
            let To::One(recipient) = recipient else { continue };
            let peer = pks.iter().position(|pk| *pk == recipient).unwrap();
            queue.push_back((to, peer, reply));
        }
    }
    let logs: Vec<_> = ceremonies[..3].iter_mut().map(|c| {
        c.close_dealing().into_iter().find_map(|(_, msg)| matches!(msg, Msg::Log { .. }).then_some(msg)).unwrap()
    }).collect();
    for ceremony in &mut ceremonies[..3] {
        for (from, log) in logs.iter().enumerate() { ceremony.on_message(&pks[from], log.clone()); }
        assert!(ceremony.have_quorum_logs());
        assert!(ceremony.propose_transcript().is_none(), "publicly revealed seated share must never be vote-eligible");
        assert!(ceremony.certified_transcript().is_none());
    }

    let retry = run_round(&ks, Round::dkg(roster, 81).with_chain_id(7781), &Default::default(), 401, 0.0);
    assert_eq!(retry.len(), 4);
    let recovered: Vec<_> = retry.values().map(|file| file.decode(4).unwrap()).collect();
    assert!(recovered.iter().all(|(output, _)| output.revealed().is_empty()));
    assert!(recovered.windows(2).all(|pair| pair[0].0.encode().to_vec() == pair[1].0.encode().to_vec()));
    assert!(recovered.iter().all(|(output, share)| {
        aether_light::Scheme::signer(&aether_light::consensus_namespace(), output.players().clone(), output.public().clone(), share.clone()).is_some()
    }));
}

/// Signed logs can be delivered to different quorums before the decision.
/// No one stages the locally computed split output; the decided signed bundle
/// lets every online player rebuild a compatible share in the same attempt.
fn split_logs_then_recover(first_count: usize, first_missing: &[usize], second_missing: &[usize]) {
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
    assert!(cs.iter().all(Ceremony::have_quorum_logs));
    assert!(cs[0].finish(&mut ChaCha20Rng::seed_from_u64(7)).is_err(), "new-genesis local logs cannot bypass the stage gate");
    let proposals: Vec<_> = cs.iter_mut().map(|c| c.propose_transcript().unwrap()).collect();
    assert_ne!(proposals[0].0, proposals[first_count].0, "schedule must split the candidate transcripts");
    // Old bare announcements can show disagreement, but cannot stage a share.
    let dones: Vec<_> = proposals.iter().map(|(digest, _)| Msg::Done(digest.clone())).collect();
    for (to, ceremony) in cs.iter_mut().enumerate() {
        for (from, done) in dones.iter().enumerate() {
            if from != to { ceremony.on_message(&pks[from], done.clone()); }
        }
    }
    assert!(cs.iter().all(|c| c.certified_transcript().is_none()), "bare Done cannot stage a split output");
    for (to, ceremony) in cs.iter_mut().enumerate() {
        for (from, (_, proposal)) in proposals.iter().enumerate() {
            if from != to { ceremony.on_message(&pks[from], proposal.clone()); }
        }
    }
    certify(&mut cs, &pks, &[0, 1, 2, 3]);
    let staged: Vec<_> = cs.iter_mut().map(|c| {
        let digest = c.certified_transcript().unwrap();
        let (o, s) = c.finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &digest).unwrap();
        KeyFile::new(round.round, &o, &s)
    }).collect();
    let (canonical, _) = staged[0].decode(4).unwrap();
    let mut partials = Vec::new();
    for f in &staged {
        let (out, share) = f.decode(4).unwrap();
        assert_eq!(out.encode().to_vec(), canonical.encode().to_vec());
        let partial = aether_node::handoff::sign_seed_partial(7781, 9, &share);
        partials.push(aether_node::handoff::check_seed_partial(7781, canonical.public(), 9, &partial).unwrap());
    }
    let seed = aether_node::handoff::combine_seed(canonical.public(), 9, &partials[..3]).unwrap();
    aether_node::handoff::verify_seed(7781, canonical.public().public(), &seed).unwrap();
}

#[test]
fn three_to_one_log_split_recovers_in_decided_round() {
    // First two plus player 3 see four logs; player 4 sees only three.
    split_logs_then_recover(3, &[], &[0]);
}

#[test]
fn two_to_two_log_split_recovers_in_decided_round() {
    split_logs_then_recover(2, &[3], &[0]);
}

/// Run the new-genesis certificate protocol in memory. The returned messages
/// can be inspected by tests; inactive players receive nothing during it.
fn certify(
    cs: &mut [Ceremony],
    pks: &[PublicKey],
    active: &[usize],
) -> Vec<(usize, Msg)> {
    let mut late = Vec::new();
    for _ in 0..150 {
        let mut queue = VecDeque::new();
        for &from in active {
            for (_, msg) in cs[from].tick_agreement() {
                late.push((from, msg.clone()));
                for &to in active {
                    if to != from { queue.push_back((from, to, msg.clone())); }
                }
            }
        }
        let mut steps = 0;
        while let Some((from, to, msg)) = queue.pop_front() {
            steps += 1;
            assert!(steps < 50_000, "agreement gossip must settle");
            for (_, reply) in cs[to].on_message(&pks[from], msg) {
                late.push((to, reply.clone()));
                for &peer in active {
                    if peer != to { queue.push_back((to, peer, reply.clone())); }
                }
            }
        }
        if active.iter().all(|&i| cs[i].certified_transcript().is_some()) { return late; }
    }
    panic!("honest quorum did not certify a usable DKG transcript");
}

/// Reproduce A3-1: player four has only three logs when three others decide.
/// Once one decided player stops, the fourth must still recover the certified
/// transcript and supply the third compatible partial signature.
fn late_minority_cannot_be_left_behind(reshare: bool) {
    let (ks, previous_files) = dkg4();
    let pks: Vec<_> = ks[..4].iter().map(|k| k.public_key()).collect();
    let set: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
    let (round, shares) = if reshare {
        let previous = previous_files.values().next().unwrap().decode(4).unwrap().0;
        (Round::reshare(previous, set, 72), shares_of(&previous_files, 4))
    } else {
        (Round::dkg(set, 72), Default::default())
    };
    let mut cs = Vec::new();
    let mut queue = VecDeque::new();
    for (i, key) in ks[..4].iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(700 + i as u64), key.clone(), round.clone(), shares.get(&pks[i]).cloned()).unwrap();
        for (to, msg) in out {
            if let To::One(peer) = to { queue.push_back((i, pks.iter().position(|p| p == &peer).unwrap(), msg)); }
        }
        cs.push(c);
    }
    while let Some((from, to, msg)) = queue.pop_front() {
        for (dest, reply) in cs[to].on_message(&pks[from], msg) {
            if let To::One(peer) = dest { queue.push_back((to, pks.iter().position(|p| p == &peer).unwrap(), reply)); }
        }
    }
    let logs: Vec<_> = cs.iter_mut().map(|c| c.close_dealing().into_iter().find_map(|(_, m)| matches!(m, Msg::Log { .. }).then_some(m)).unwrap()).collect();
    for (to, ceremony) in cs.iter_mut().enumerate() {
        for (from, log) in logs.iter().enumerate() {
            if to != 3 || from != 0 {
                ceremony.on_message(&pks[from], log.clone());
            }
        }
    }
    assert!(cs.iter().all(Ceremony::have_quorum_logs));
    assert!(!cs[3].have_all_logs());
    let (minority_digest, _) = cs[3].propose_transcript().unwrap();
    let proposals: Vec<_> = cs[..3].iter_mut().map(|c| c.propose_transcript().unwrap()).collect();
    let agreed = proposals[0].0.clone();
    assert!(proposals.iter().all(|(digest, _)| *digest == agreed));
    assert_ne!(minority_digest, agreed, "the schedule must split the candidate transcripts");
    // Keep player four off the bundle/decision channel until after the first
    // three have finalized. It still has all private dealings from the Ack phase.
    for (to, ceremony) in cs[..3].iter_mut().enumerate() {
        for from in 0..3 {
            if to != from { ceremony.on_message(&pks[from], proposals[from].1.clone()); }
        }
    }
    let certified_gossip = certify(&mut cs, &pks, &[0, 1, 2]);
    for c in &cs[..3] { assert_eq!(c.certified_transcript(), Some(agreed.clone())); }
    let (source, decision) = certified_gossip.iter().find(|(_, msg)| matches!(msg, Msg::Agreement(AgreementMsg::Decision(_))))
        .expect("quorum broadcasts a decision certificate");
    cs[3].on_message(&pks[*source], decision.clone());
    assert!(cs[3].certified_transcript().is_none(), "decision alone cannot finalize without the signed bundle");
    let mut usable = Vec::new();
    for ceremony in &mut cs[..2] {
        usable.push(ceremony.finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &agreed).unwrap());
    }
    // Player three also staged, then goes offline. Its share cannot help form
    // the next 3-of-4 consensus signature.
    let staged_then_offline = cs[2].finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &agreed).unwrap();
    assert_eq!(staged_then_offline.0.encode().to_vec(), usable[0].0.encode().to_vec());
    cs[0].on_message(&pks[3], Msg::Done(minority_digest));
    assert_ne!(cs[0].agreement(true), Some(false), "late minority Done cannot revoke a certificate");
    // The fourth player's receive path was partitioned through the old five
    // second return window and beyond a 30 second grace. Players 1 and 2 have
    // returned and no longer participate. One bounded relay of the decided
    // bundle and certificate must suffice for the late honest player.
    for _ in 0..70 {
        let mut out = cs[0].tick_agreement();
        out.extend(cs[0].rebroadcast());
        for (_, msg) in out { cs[3].on_message(&pks[0], msg); }
        cs[3].tick_agreement();
    }
    assert_eq!(cs[3].certified_transcript(), Some(agreed.clone()));
    usable.push(cs[3].finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &agreed).unwrap());
    let canonical = usable[0].0.encode().to_vec();
    assert!(usable.iter().all(|(o, _)| o.encode().to_vec() == canonical));
    let mut partials = Vec::new();
    for (_, share) in &usable {
        let partial = aether_node::handoff::sign_seed_partial(7781, 9, share);
        partials.push(aether_node::handoff::check_seed_partial(7781, usable[0].0.public(), 9, &partial).unwrap());
    }
    let seed = aether_node::handoff::combine_seed(usable[0].0.public(), 9, &partials).unwrap();
    aether_node::handoff::verify_seed(7781, usable[0].0.public().public(), &seed).unwrap();
}

#[test]
fn genesis_late_minority_after_majority_stage_and_one_offline() {
    late_minority_cannot_be_left_behind(false);
}

#[test]
fn reshare_late_minority_after_majority_stage_and_one_offline() {
    late_minority_cannot_be_left_behind(true);
}

#[test]
fn byzantine_zero_done_cannot_abort_honest_quorum() {
    let ks = keys(4);
    let pks: Vec<_> = ks.iter().map(|k| k.public_key()).collect();
    let set: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
    let mut cs = Vec::new();
    let mut queue = VecDeque::new();
    for (i, key) in ks.iter().enumerate() {
        let (c, out) = Ceremony::start(ChaCha20Rng::seed_from_u64(800 + i as u64), key.clone(), Round::dkg(set.clone(), 73), None).unwrap();
        for (to, msg) in out {
            if let To::One(peer) = to { queue.push_back((i, pks.iter().position(|p| p == &peer).unwrap(), msg)); }
        }
        cs.push(c);
    }
    while let Some((from, to, msg)) = queue.pop_front() {
        for (dest, reply) in cs[to].on_message(&pks[from], msg) {
            if let To::One(peer) = dest { queue.push_back((to, pks.iter().position(|p| p == &peer).unwrap(), reply)); }
        }
    }
    let logs: Vec<_> = cs.iter_mut().map(|c| c.close_dealing().into_iter().find_map(|(_, m)| matches!(m, Msg::Log { .. }).then_some(m)).unwrap()).collect();
    for c in &mut cs {
        for (from, log) in logs.iter().enumerate() { c.on_message(&pks[from], log.clone()); }
    }
    let proposals: Vec<_> = cs[..3].iter_mut().map(|c| c.propose_transcript().unwrap()).collect();
    let agreed = proposals[0].0.clone();
    assert!(proposals.iter().all(|(digest, _)| *digest == agreed));
    for (to, ceremony) in cs[..3].iter_mut().enumerate() {
        for from in 0..3 {
            if to != from { ceremony.on_message(&pks[from], proposals[from].1.clone()); }
        }
    }
    cs[0].on_message(&pks[3], Msg::Done(vec![0; 32]));
    cs[0].on_message(&pks[3], Msg::Done(vec![1; 31]));
    cs[0].on_message(&pks[3], Msg::Agreement(AgreementMsg::Vote(SignedVote {
        view: 0, phase: Phase::Precommit, digest: [0; 32],
        signer: pks[3].encode().to_vec(), signature: vec![0; 64],
    })));
    certify(&mut cs, &pks, &[0, 1, 2]);
    for c in &mut cs[..3] {
        assert_eq!(c.certified_transcript(), Some(agreed.clone()));
        c.finish_decided(&mut ChaCha20Rng::seed_from_u64(7), &agreed).unwrap();
    }
}

#[test]
fn chain_7780_legacy_done_remains_identity_only() {
    let ks = keys(4);
    let pks: Vec<_> = ks.iter().map(|k| k.public_key()).collect();
    let set: Set<PublicKey> = pks.iter().cloned().try_collect().unwrap();
    let mut cs = Vec::new();
    let mut queue = VecDeque::new();
    for (i, key) in ks.iter().enumerate() {
        let (c, out) = Ceremony::start(
            ChaCha20Rng::seed_from_u64(900 + i as u64), key.clone(), Round::dkg(set.clone(), 0).legacy_agreement(), None,
        ).unwrap();
        for (to, msg) in out {
            if let To::One(peer) = to { queue.push_back((i, pks.iter().position(|p| p == &peer).unwrap(), msg)); }
        }
        cs.push(c);
    }
    while let Some((from, to, msg)) = queue.pop_front() {
        for (dest, reply) in cs[to].on_message(&pks[from], msg) {
            if let To::One(peer) = dest { queue.push_back((to, pks.iter().position(|p| p == &peer).unwrap(), reply)); }
        }
    }
    let logs: Vec<_> = cs.iter_mut().map(|c| c.close_dealing().into_iter().find_map(|(_, m)| matches!(m, Msg::Log { .. }).then_some(m)).unwrap()).collect();
    for c in &mut cs {
        for (from, log) in logs.iter().enumerate() { c.on_message(&pks[from], log.clone()); }
    }
    let (output, _) = cs[0].finish(&mut ChaCha20Rng::seed_from_u64(7)).unwrap();
    let done = cs[0].rebroadcast().into_iter().find_map(|(_, m)| match m { Msg::Done(bytes) => Some(bytes), _ => None }).unwrap();
    assert_eq!(done, output.public().public().encode().to_vec(), "7780 announces only the unchanged identity bytes");
    assert!(cs[0].tick_agreement().is_empty(), "7780 emits no new-genesis agreement messages");
}

#[test]
fn new_genesis_rejects_signed_logs_from_7780_and_another_chain() {
    let ks = keys(4);
    let roster: Set<PublicKey> = ks.iter().map(|key| key.public_key()).try_collect().unwrap();
    let old_round = Round::dkg(roster.clone(), 0).legacy_agreement();
    let (mut old_dealer, _) = Ceremony::start(ChaCha20Rng::seed_from_u64(17), ks[0].clone(), old_round.clone(), None).unwrap();
    let old_log = old_dealer.close_dealing().into_iter().find_map(|(_, msg)| matches!(msg, Msg::Log { .. }).then_some(msg)).unwrap();
    let (mut legacy_receiver, _) = Ceremony::start(ChaCha20Rng::seed_from_u64(18), ks[1].clone(), old_round, None).unwrap();
    legacy_receiver.on_message(&ks[0].public_key(), old_log.clone());
    assert_eq!(legacy_receiver.log_count(), 1, "the frozen 7780 namespace still verifies its own logs");

    let (mut new_receiver, _) = Ceremony::start(
        ChaCha20Rng::seed_from_u64(18), ks[1].clone(), Round::dkg(roster.clone(), 0).with_chain_id(7781), None,
    ).unwrap();
    new_receiver.on_message(&ks[0].public_key(), old_log);
    assert_eq!(new_receiver.log_count(), 0, "an archived 7780 log cannot poison a new genesis");
    let (mut other_dealer, _) = Ceremony::start(
        ChaCha20Rng::seed_from_u64(17), ks[0].clone(), Round::dkg(roster, 0).with_chain_id(7782), None,
    ).unwrap();
    let other_log = other_dealer.close_dealing().into_iter().find_map(|(_, msg)| matches!(msg, Msg::Log { .. }).then_some(msg)).unwrap();
    new_receiver.on_message(&ks[0].public_key(), other_log);
    assert_eq!(new_receiver.log_count(), 0, "another new chain's signed log is also rejected");
}

#[test]
fn chain_bound_new_genesis_players_share_one_usable_output() {
    let ks = keys(4);
    let roster: Set<PublicKey> = ks.iter().map(|key| key.public_key()).try_collect().unwrap();
    let files = run_round(&ks, Round::dkg(roster, 0).with_chain_id(7781), &Default::default(), 94, 0.0);
    assert_eq!(files.len(), 4);
    let canonical = &files.values().next().unwrap().output;
    assert!(files.values().all(|file| &file.output == canonical));
    let mut partials = Vec::new();
    for file in files.values() {
        let (output, share) = file.decode(4).unwrap();
        let partial = aether_node::handoff::sign_seed_partial(7781, 1, &share);
        partials.push(aether_node::handoff::check_seed_partial(7781, output.public(), 1, &partial).unwrap());
    }
    let (output, _) = files.values().next().unwrap().decode(4).unwrap();
    assert!(output.revealed().is_empty(), "all-honest chain-bound output has no published share");
    let seed = aether_node::handoff::combine_seed(output.public(), 1, &partials[..3]).unwrap();
    aether_node::handoff::verify_seed(7781, output.public().public(), &seed).unwrap();
}
