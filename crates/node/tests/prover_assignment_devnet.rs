//! Four independent in-memory peers run the real registry and proof inclusion
//! paths. Proofs echo the replayed commitment; this is not a GPU or process
//! devnet, and does not measure sidecar cancellation latency.

mod common;

use aether_execution::{proofs, registry};
use aether_light::block::ProofClaim;
use aether_node::block::Block;
use aether_node::chain::{Chain, Executed, ProofVerifier, MAX_PROOFS_PER_BLOCK};
use aether_node::prover_assignment::{designated, Config};
use aether_types::Address;
use common::{EchoVerifier, Net, Opts};
use commonware_cryptography::Digestible as _;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

const OPERATORS: usize = 4;
// The pool accepts proofs of at least 32 KiB. Keep the fixture's echo claim
// and validate its padding rather than bypassing the production pool checks.
const PROOF_BYTES: usize = 32 << 10;
type Job = (Arc<Executed>, Arc<Executed>, Block);

struct PaddedEchoVerifier;

impl ProofVerifier for PaddedEchoVerifier {
    fn verify(&self, proof: &[u8], output: [u8; 32]) -> bool {
        proof.len() == PROOF_BYTES
            && EchoVerifier.verify(&proof[..32], output)
            && proof[32..].iter().all(|byte| *byte == 0)
    }
}

fn claim(height: u64, commitment: [u8; 32], prover: Address) -> ProofClaim {
    let mut proof = vec![0; PROOF_BYTES];
    proof[..32].copy_from_slice(&aether_proving::block::claim(commitment, prover));
    ProofClaim {
        height,
        prover,
        proof: hex::encode(proof),
    }
}

/// Rebuild the exact input used by the sidecar and re-execute its witness,
/// including registration transactions and the block's system pre-state.
fn replay_claim(net: &Net, job: &Job, prover: Address) -> ProofClaim {
    let (exec, parent, block) = job;
    let payload = block.payload().expect("selected block has a payload");
    let (pre, _) = net
        .chain
        .pre_state_with(
            parent,
            payload.version,
            &payload.proofs,
            &payload.beacons,
            &payload.registrations,
            payload.seed.as_ref(),
            true,
        )
        .expect("selected block's parent can reconstruct its pre-state");
    let context = Chain::block_context(&net.chain.cfg(), block, parent);
    let input = aether_proving::block::input(&pre, &context, &payload.txs, &[], prover)
        .expect("selected block has a stateless proving input");
    let replay = aether_proving::block::execute(&input).expect("proving witness executes");
    assert_eq!(
        replay.commitment(),
        exec.statement.commitment,
        "height {}",
        exec.height
    );
    claim(exec.height, replay.commitment(), prover)
}

fn assert_same_history(peers: &[Net]) {
    for peer in &peers[1..] {
        assert_eq!(peer.parent.height, peers[0].parent.height);
        assert_eq!(peer.parent.state.root(), peers[0].parent.state.root());
        assert_eq!(peer.last.digest(), peers[0].last.digest());
    }
}

fn step_all(peers: &mut [Net], claims: &[ProofClaim]) {
    for peer in peers.iter_mut() {
        peer.step(vec![], None, claims.to_vec());
    }
    assert_same_history(peers);
}

fn peers(config: &Config, height: u64) -> Vec<Net> {
    assert!(height > 0);
    let mut peers: Vec<_> = (0..OPERATORS)
        .map(|_| {
            let net = Net::new(Opts {
                chain_id: 7_794,
                node_rewards: true,
                epoch_blocks: 360,
                macs: OPERATORS as u8,
                min_streak: Some(0),
                history_v2: false,
                protocol: 2,
                reserve: None,
                fees: false,
                committee: None,
            });
            net.chain.set_prover_window(config.window);
            net.chain.lock().verifier = Some(Arc::new(PaddedEchoVerifier));
            net
        })
        .collect();
    // Construct each Chain separately: cloning a Chain would accidentally
    // share its local attempted/lost jobs and conceal assignment races.
    for peer in &mut peers {
        let registrations = (0..OPERATORS)
            .map(|operator| peer.register(operator))
            .collect();
        peer.step(registrations, None, vec![]);
    }
    assert_same_history(&peers);
    while peers[0].parent.height < height {
        step_all(&mut peers, &[]);
    }
    let expected: BTreeSet<_> = (0..OPERATORS).map(|i| peers[0].operator(i)).collect();
    for peer in &peers {
        let registered: BTreeSet<_> = registry::candidates(&peer.parent.state)
            .into_iter()
            .map(|candidate| candidate.operator)
            .collect();
        assert_eq!(
            registered, expected,
            "the real EVM registry supplies every peer's operators"
        );
    }
    peers
}

#[test]
fn four_provers_distribute_designated_work_and_include_every_open_target() {
    const TARGET: u64 = 24;
    let config = Config::default();
    let mut peers = peers(&config, TARGET);
    let operators: Vec<_> = (0..OPERATORS).map(|i| peers[0].operator(i)).collect();
    let mut attempts: Vec<BTreeSet<u64>> = (0..OPERATORS).map(|_| BTreeSet::new()).collect();
    let mut target_jobs = [0usize; OPERATORS];

    // Each tick dispatches all four peers before delivering any competing
    // proof, so eligible duplication is visible rather than hidden by a
    // sequential fixture. Proofs then traverse the verified pool and the
    // ordinary two-proof proposal limit on every independent chain.
    for _ in 0..TARGET * 3 {
        if (1..=TARGET).all(|height| proofs::prover(&peers[0].parent.state, height).is_some()) {
            break;
        }
        let now_ms = peers[0].last.timestamp;
        let mut first = BTreeMap::new();
        for (i, peer) in peers.iter().enumerate() {
            let prover = operators[i];
            if let Some(job) = peer.chain.provable_for(prover, &config, now_ms) {
                let height = job.0.height;
                assert!(
                    attempts[i].insert(height),
                    "peer {i} retried a completed/lost job at {height}"
                );
                if height <= TARGET {
                    target_jobs[i] += 1;
                }
                if now_ms.saturating_sub(job.2.timestamp) <= config.grace.as_millis() as u64 {
                    assert!(designated(height, &operators, config.designated).contains(&prover));
                }
                first
                    .entry(height)
                    .or_insert_with(|| replay_claim(peer, &job, prover));
            }
        }
        for proof in first.values() {
            for peer in &peers {
                peer.chain
                    .add_own_proof(proof.clone())
                    .expect("a valid proof enters every peer's pool");
                assert!(
                    peer.chain.proof_seen(proof.height),
                    "verified pool proof cancels local work"
                );
            }
        }
        let included = peers[0].chain.proofs_for(&peers[0].parent);
        assert!(
            !included.is_empty(),
            "open target blocks must keep making proof progress"
        );
        assert!(included.len() <= MAX_PROOFS_PER_BLOCK);
        step_all(&mut peers, &included);
        for peer in &peers {
            for proof in &included {
                assert_eq!(
                    proofs::prover(&peer.parent.state, proof.height),
                    Some(proof.prover)
                );
                assert!(peer.chain.proof_seen(proof.height));
            }
        }
    }

    for peer in &peers {
        assert!((1..=TARGET).all(|height| proofs::prover(&peer.parent.state, height).is_some()));
    }
    assert!(
        target_jobs.iter().all(|jobs| *jobs > 0),
        "every registered prover receives designated target work: {target_jobs:?}"
    );
}

#[test]
fn offline_designated_provers_leave_work_open_until_grace_then_anyone_can_replay_it() {
    let config = Config {
        designated: 1,
        grace: Duration::from_secs(60),
        window: 128,
    };
    let mut peers = peers(&config, 1);
    let operators: Vec<_> = (0..OPERATORS).map(|i| peers[0].operator(i)).collect();
    let assigned = designated(1, &operators, config.designated)[0];
    let fallback = operators
        .iter()
        .copied()
        .find(|operator| *operator != assigned)
        .unwrap();
    let outsider = Address::repeat_byte(0xf0);
    let timestamp = peers[0].last.timestamp;
    let boundary = timestamp + 60_000;

    assert!(
        peers[0]
            .chain
            .provable_for(fallback, &config, boundary)
            .is_none(),
        "age must exceed the grace period"
    );
    let fallback_job = peers[0]
        .chain
        .provable_for(fallback, &config, boundary + 1)
        .expect("registered fallback starts after grace");
    assert_eq!(fallback_job.0.height, 1);
    let fallback_proof = replay_claim(&peers[0], &fallback_job, fallback);

    assert!(
        peers[1]
            .chain
            .provable_for(outsider, &config, boundary)
            .is_none(),
        "unregistered provers also wait for grace"
    );
    let outsider_job = peers[1]
        .chain
        .provable_for(outsider, &config, boundary + 1)
        .expect("unregistered fallback starts after grace");
    assert_eq!(outsider_job.0.height, 1);
    replay_claim(&peers[1], &outsider_job, outsider);

    let immediate = peers[2]
        .chain
        .provable_for(assigned, &config, timestamp)
        .expect("designated prover need not wait");
    assert_eq!(immediate.0.height, 1);
    for peer in &peers {
        peer.chain.add_own_proof(fallback_proof.clone()).unwrap();
    }
    let included = peers[0].chain.proofs_for(&peers[0].parent);
    step_all(&mut peers, &included);
    for peer in &peers {
        assert_eq!(proofs::prover(&peer.parent.state, 1), Some(fallback));
    }
}

#[test]
fn only_verified_competing_proofs_cancel_and_lost_jobs_never_reopen() {
    let config = Config {
        designated: 1,
        ..Config::default()
    };
    let mut peers = peers(&config, 1);
    let operators: Vec<_> = (0..OPERATORS).map(|i| peers[0].operator(i)).collect();
    let local = designated(1, &operators, config.designated)[0];
    let winner = operators
        .iter()
        .copied()
        .find(|operator| *operator != local)
        .unwrap();
    let timestamp = peers[0].last.timestamp;
    let job = peers[0]
        .chain
        .provable_for(local, &config, timestamp)
        .unwrap();
    assert!(!peers[0].chain.proof_seen(1));

    // A claim bound to the local prover cannot be relabeled as a competitor.
    let mut invalid = replay_claim(&peers[0], &job, local);
    invalid.prover = winner;
    assert!(peers[0].chain.add_own_proof(invalid.clone()).is_err());
    assert!(!peers[0].chain.proof_seen(1));
    assert!(peers[0]
        .build(vec![], None, vec![invalid], peers[0].answers())
        .is_err());
    peers[0].chain.retry_proof(1);
    assert_eq!(
        peers[0]
            .chain
            .provable_for(local, &config, timestamp)
            .unwrap()
            .0
            .height,
        1,
        "a local failure can retry before any valid competing proof"
    );

    // Designation guides clients; consensus still accepts the first valid
    // proof from an operator outside the designated set.
    let proof = replay_claim(&peers[0], &job, winner);
    for peer in &peers {
        peer.chain.add_own_proof(proof.clone()).unwrap();
        assert!(peer.chain.proof_seen(1));
    }
    assert!(
        peers[0]
            .chain
            .add_own_proof(replay_claim(&peers[0], &job, local))
            .is_err(),
        "the first valid pooled proof retains its place"
    );
    peers[0].chain.drop_proofs(&[1]);
    peers[0].chain.retry_proof(1);
    assert!(
        peers[0]
            .chain
            .provable_for(local, &config, timestamp + 60_001)
            .is_none(),
        "removing a pooled proof must not reopen a permanently lost job"
    );
    peers[0].chain.add_own_proof(proof.clone()).unwrap();

    step_all(&mut peers, &[proof]);
    for peer in &peers {
        assert_eq!(proofs::prover(&peer.parent.state, 1), Some(winner));
        assert!(peer.chain.proof_seen(1));
        peer.chain.retry_proof(1);
        assert_ne!(
            peer.chain
                .provable_for(local, &config, timestamp + 60_001)
                .map(|job| job.0.height),
            Some(1)
        );
    }
}

#[test]
fn an_expired_grace_job_retains_its_parent_past_the_old_32_block_limit() {
    let config = Config::default();
    let peers = peers(&config, 80);
    let outsider = Address::repeat_byte(0xf0);
    let now_ms = peers[0].last.timestamp;

    for peer in &peers {
        let job = peer
            .chain
            .provable_for(outsider, &config, now_ms)
            .expect("oldest fallback remains locally replayable");
        assert_eq!(
            job.0.height, 1,
            "unregistered fallback selects the oldest expired block"
        );
        assert_eq!(
            job.1.height, 0,
            "the retained window includes its oldest block's parent"
        );
        assert!(peer.parent.height - job.0.height > 32);
        replay_claim(peer, &job, outsider);
    }
}
