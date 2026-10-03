//! Deterministic network simulation (docs/design/12-launch-plan.md step 1).
//!
//! The real validator stack (simplex + marshal + Application + Chain, the same
//! code `aether node` runs) on Commonware's deterministic runtime and simulated
//! p2p network. Time is virtual, so minutes of network time run in seconds and
//! every run is reproducible from its seed. Faults: lossy and jittery links,
//! a 2|2 partition, a validator cut off and rejoining, validators that disagree
//! about an inclusion list (the 2026-09-28 testnet stall), slow or stalled
//! disks, seeded crashes, silent or double-voting members, and clock skew.
//!
//! Checked every run: safety (no two different blocks finalized at one height,
//! across all validators), liveness (the chain advances once a quorum can talk),
//! agreement (validators share finalized hashes), and replay (the same
//! seed gives the same chain). `AETHER_SIM_SEEDS` / `AETHER_SIM_SECS` scale the
//! ignored `soak` test for long runs.

use aether_crypto::P256Signer;
use aether_execution::{sign_call_with, EvmCall};
use aether_light::block::Handoff;
use aether_light::{consensus_namespace, devnet_threshold, devnet_validator_key, Scheme};
use aether_node::application::{Application, MIN_BLOCK_INTERVAL_MS};
use aether_node::chain::{dev_accounts, dev_seed, Chain, ChainConfig};
use aether_node::engine::{self, MAX_BLOCK_BYTES};
use aether_node::epochs::ScheduleEpocher;
use aether_node::inclusion::InclusionList;
use aether_node::upgrade::MAINNET_NOTICE_BLOCKS;
use aether_types::{Address, Bytes, FeeVector, GasVector, U256};
use commonware_codec::Encode as _;
use commonware_consensus::marshal;
use commonware_consensus::simplex::types::{Notarize, Proposal, Vote};
use commonware_consensus::types::{Epoch, Round, View, ViewDelta};
use commonware_cryptography::bls12381::dkg::feldman_desmedt::deal;
use commonware_cryptography::bls12381::primitives::sharing::Mode;
use commonware_cryptography::bls12381::primitives::variant::MinSig;
use commonware_cryptography::{sha256::Digest, Hasher as _, Sha256, Signer as _};
use commonware_p2p::simulated::{self, Link, Network, Oracle};
use commonware_p2p::{Recipients, Sender as _};
use commonware_runtime::{deterministic, Clock, Quota, Runner as _, Supervisor as _};
use commonware_utils::{probability, N3f1, NZUsize, NZU32};
use rand::SeedableRng as _;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

mod slow_disk;
use slow_disk::{Disk, SlowDisk};

type Pk = commonware_cryptography::ed25519::PublicKey;
type Ctx = deterministic::Context;

const N: u64 = 4;
const BLOCK_MS: u64 = 1_000;
const QUOTA: Quota = Quota::per_second(NZU32!(u32::MAX));
const GOOD: Link = Link {
    latency: Duration::from_millis(40),
    jitter: Duration::from_millis(15),
    success_rate: probability!(0.97),
};

#[derive(Clone, Copy, Debug)]
enum Fault {
    None,
    /// Validators {1,2} and {3,4} cannot talk for `[from, to)` seconds (no quorum on either side).
    Partition {
        from: u64,
        to: u64,
    },
    /// Validator 4 is cut off for `[from, to)` seconds, then must catch up.
    Isolate {
        from: u64,
        to: u64,
    },
    /// For `[from, to)` seconds validators 1 and 3 hold an inclusion list naming
    /// a tx that validators 2 and 4 never see, so 1 and 3 refuse every block
    /// from 2 and 4 (the 2026-09-28 testnet stall: a notarized block that half
    /// the committee then refused to certify).
    SplitList {
        from: u64,
        to: u64,
    },
    /// Validator 4's disk takes `delay_ms` for every write and sync during
    /// `[from, to)` seconds (a nearly full, busy disk, as on 2026-09-28).
    SlowDisk {
        from: u64,
        to: u64,
        delay_ms: u64,
    },
    /// Validators 3 and 4 (half the committee, so no quorum without them) take
    /// `delay_ms` for every write and sync during `[from, to)` seconds.
    SlowDisks {
        from: u64,
        to: u64,
        delay_ms: u64,
    },
    /// One member casts votes but never proposes when it is elected leader.
    Silent { from: u64, to: u64 },
    /// Validator 4's local clock is ahead of the other validators.
    ClockSkew { from: u64, to: u64, skew_ms: i64 },
    /// Validator 4 signs two different notarize votes in the same views.
    Equivocate { from: u64, to: u64 },
}

/// What a run ends with, per validator.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    heights: Vec<u64>,
    /// Finalized block hash by height, per validator.
    finalized: Vec<BTreeMap<u64, String>>,
    roots: Vec<String>,
    txs: usize,
    /// Block timestamps (ms) of validator 1's finalized chain.
    timestamps: Vec<u64>,
    equivocation_detected: bool,
    handoff_switch: Option<u64>,
}

fn chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: 7_777,
        limits: GasVector {
            exec: 30_000_000,
            state: u64::MAX,
            prove: 200_000_000,
        },
        // Dev account 5 only ever sends through inclusion lists (`Fault::SplitList`).
        alloc: dev_accounts(5)
            .into_iter()
            .map(|(_, a)| (a, U256::from(10u128.pow(24))))
            .collect(),
        fees: true,
        registrar: None,
        epoch_blocks: 0,
        min_streak: None,
        draw_epochs: None,
        history_v2: false,
        protocol: 1,
        node_rewards: false,
        group: 0,
        max_committee: aether_node::rotation::GROW_UNTIL,
        committee: vec![],
        reserve: None,
    }
}

async fn link_all(oracle: &mut Oracle<Pk, Ctx>, keys: &[Pk], up: impl Fn(usize, usize) -> bool) {
    for (i, a) in keys.iter().enumerate() {
        for (j, b) in keys.iter().enumerate() {
            if i == j {
                continue;
            }
            let _ = oracle.remove_link(a.clone(), b.clone()).await;
            if up(i, j) {
                oracle
                    .add_link(a.clone(), b.clone(), GOOD)
                    .await
                    .expect("add link");
            }
        }
    }
}

async fn start_validator(context: &Ctx, oracle: &Oracle<Pk, Ctx>, i: u64, chain: Chain, disk: Disk, proposals: Arc<AtomicBool>) -> (Scheme, simulated::Sender<Pk, Ctx>) {
    start_validator_with_delay(context, oracle, i, chain, disk, proposals, BLOCK_MS).await
}

async fn start_validator_with_delay(context: &Ctx, oracle: &Oracle<Pk, Ctx>, i: u64, chain: Chain, disk: Disk, proposals: Arc<AtomicBool>, block_ms: u64) -> (Scheme, simulated::Sender<Pk, Ctx>) {
    let (participants, polynomial, shares) = devnet_threshold(N);
    let key = devnet_validator_key(i);
    let me = key.public_key();
    let share = shares
        .into_iter()
        .find(|(pk, _)| *pk == me)
        .map(|(_, s)| s)
        .expect("validator share");
    let identity = *polynomial.public();
    let scheme = Scheme::signer(&consensus_namespace(), participants, polynomial, share)
        .expect("share matches");
    let control = oracle.control(me.clone());
    let pending = control.register(0, QUOTA).await.expect("register");
    let injected_votes = pending.0.clone();
    let injected_scheme = scheme.clone();
    let recovered = control.register(1, QUOTA).await.expect("register");
    let resolver = control.register(2, QUOTA).await.expect("register");
    let broadcast = control.register(3, QUOTA).await.expect("register");
    let backfill = control.register(4, QUOTA).await.expect("register");
    let marshal_resolver = marshal::resolver::p2p::init(
        context.child("backfill").with_attribute("validator", i),
        marshal::resolver::p2p::Config {
            public_key: me.clone(),
            peer_provider: oracle.manager(),
            blocker: oracle.control(me.clone()),
            mailbox_size: NZUsize!(1024),
            timeout: Duration::from_secs(2),
            fetch_retry_timeout: Duration::from_millis(100),
            priority_requests: false,
            priority_responses: false,
        },
        backfill,
    );
    let (_, genesis) = Chain::new(chain.cfg());
    let engine = engine::Engine::new(
        SlowDisk::new(context.child("engine").with_attribute("validator", i), disk),
        engine::Config {
            anchor: None,
            // Half the validators keep marshal's archives in the pruning layout (roadmap B4):
            // consensus must not care which one a node uses.
            layout: if i.is_multiple_of(2) {
                engine::Layout::Prunable
            } else {
                engine::Layout::Immutable
            },
            blocker: oracle.control(me.clone()),
            provider: oracle.manager(),
            partition_prefix: format!("v{i}"),
            journal_dir: None,
            me,
            scheme,
            identity,
            group: 0,
            epocher: ScheduleEpocher::new(vec![]),
            epoch_floor: None,
            genesis,
            application: Application::new(chain, block_ms).with_proposal_switch(proposals),
            mailbox_size: 1024,
            leader_timeout: Duration::from_secs(2),
            certification_timeout: Duration::from_secs(3),
            nullify_retry: Duration::from_secs(4),
            fetch_timeout: Duration::from_secs(2),
            activity_timeout: ViewDelta::new(20),
            skip_timeout: Duration::from_secs(5),
        },
    )
    .await;
    engine.start(pending, recovered, resolver, broadcast, marshal_resolver);
    (injected_scheme, injected_votes)
}

fn send_conflicting_votes(scheme: &Scheme, sender: &mut simulated::Sender<Pk, Ctx>, peers: &[Pk]) {
    for view in 1..128 {
        let round = Round::new(Epoch::zero(), View::new(view));
        let a = Proposal::new(round, View::zero(), Sha256::hash(&[format!("a{view}").as_bytes()]));
        let b = Proposal::new(round, View::zero(), Sha256::hash(&[format!("b{view}").as_bytes()]));
        for proposal in [a, b] {
            let vote = Vote::<Scheme, Digest>::Notarize(Notarize::sign(scheme, proposal).expect("share signs"));
            sender.send(Recipients::Some(peers.to_vec()), vote.encode(), true);
        }
    }
}

/// A signed handoff to the same roster is enough to exercise the durable
/// pending-handoff transition without changing the committee's voting keys.
fn signed_handoff() -> Handoff {
    let (participants, sharing, shares) = devnet_threshold(N);
    let mut seed = [0u8; 32];
    seed[..24].copy_from_slice(b"aether-devnet-threshold-");
    seed[24..].copy_from_slice(&N.to_be_bytes());
    let (output, _) = deal::<MinSig, Pk, N3f1>(
        rand_chacha::ChaCha20Rng::from_seed(seed), Mode::NonZeroCounter, participants.clone(),
    ).expect("devnet sharing");
    assert_eq!(output.public(), &sharing);
    let handoff = Handoff {
        round: 1,
        output: hex::encode(output.encode()),
        members: participants.iter().enumerate().map(|(i, key)| {
            let node = aether_net::SecretKey::from_bytes(&[0xa0 + i as u8; 32]).public();
            (hex::encode(key.encode()), node.to_string())
        }).collect(),
        signature: String::new(),
    };
    let partials: Vec<_> = shares.iter().take(sharing.required() as usize).map(|(_, share)| {
        let signed = aether_node::handoff::sign_partial(7_777, &handoff, share);
        aether_node::handoff::check_partial(7_777, &sharing, &handoff, &signed).expect("signed partial")
    }).collect();
    aether_node::handoff::combine(&sharing, &handoff, &partials).expect("quorum handoff")
}

/// A transfer from dev account `from` with `nonce`, priced well above the base fee.
fn transfer(from: u8, nonce: u64, to: Address) -> aether_types::TxEnvelope {
    let signer = P256Signer::from_seed(&dev_seed(from)).expect("dev key");
    let call = EvmCall {
        to: Some(to),
        value: U256::from(1_000u64),
        input: Bytes::new(),
        gas_limit: 21_000,
        delegate: None,
    };
    let fees = FeeVector {
        exec: 100_000_000_000,
        state: 0,
        prove: 100_000_000_000,
    };
    sign_call_with(&signer, 7_777, nonce, fees, 1_000_000_000, &call).expect("sign")
}

/// Puts dev account 5's next tx on an inclusion list that only validators 1
/// and 3 hold, already past the voters' freeze. Their blocks include it; the
/// blocks of 2 and 4 leave it out, so 1 and 3 refuse to vote for them.
fn list_for_half(chains: &[Chain], t: u64) {
    let sender = dev_accounts(5)[4].1;
    let nonce = chains[0].lock().finalized.state.nonce(&sender);
    let tx = transfer(5, nonce, Address::repeat_byte(0xb5));
    let list = InclusionList {
        height: 1_000_000 + t,
        member: 1,
        txs: vec![tx],
        signature: String::new(),
    };
    let seen = Instant::now()
        .checked_sub(Duration::from_secs(5))
        .unwrap_or_else(Instant::now);
    for i in [0, 2] {
        chains[i].lock().inclusion.accept(&list, seen);
    }
}

fn simulate(seed: u64, secs: u64, fault: Fault) -> Outcome {
    simulate_with_rules(seed, secs, fault, false, BLOCK_MS)
}

fn simulate_with_rules(seed: u64, secs: u64, fault: Fault, new_genesis: bool, block_ms: u64) -> Outcome {
    let cfg = deterministic::Config::new()
        .with_seed(seed)
        .with_timeout(Some(Duration::from_secs(secs + 120)));
    deterministic::Runner::new(cfg).start(|context| async move {
        let keys: Vec<Pk> = (1..=N)
            .map(|i| devnet_validator_key(i).public_key())
            .collect();
        let (network, mut oracle) = Network::new_with_peers(
            context.child("network"),
            simulated::Config {
                max_size: MAX_BLOCK_BYTES + 1024 * 1024,
                max_peers_per_set: NZUsize!(N as usize),
                disconnect_on_block: true,
                tracked_peer_sets: NZUsize!(1),
            },
            keys.clone(),
        )
        .await;
        network.start();
        link_all(&mut oracle, &keys, |_, _| true).await;

        let chains: Vec<Chain> = (0..N).map(|_| {
            let mut cfg = chain_config();
            cfg.history_v2 = new_genesis;
            Chain::new(cfg).0
        }).collect();
        let disks: Vec<Disk> = (0..N).map(|_| Disk::default()).collect();
        let proposals: Vec<_> = (0..N).map(|_| Arc::new(AtomicBool::new(true))).collect();
        let mut byzantine = None;
        for (i, chain) in chains.iter().enumerate() {
            let injected = start_validator_with_delay(&context, &oracle, i as u64 + 1, chain.clone(), disks[i].clone(), proposals[i].clone(), block_ms).await;
            if i == 3 {
                byzantine = Some(injected);
            }
        }

        let mut nonces = [0u64; 4];
        let mut sent = 0;
        let bob = Address::repeat_byte(0xb0);
        let mut state = Fault::None;
        let mut equivocation_detected = false;
        for t in 0..secs {
            // Apply the fault schedule at whole virtual seconds.
            let now = match fault {
                Fault::Partition { from, to } if (from..to).contains(&t) => fault,
                Fault::Isolate { from, to } if (from..to).contains(&t) => fault,
                Fault::SplitList { from, to } if (from..to).contains(&t) => fault,
                Fault::SlowDisk { from, to, .. } if (from..to).contains(&t) => fault,
                Fault::SlowDisks { from, to, .. } if (from..to).contains(&t) => fault,
                Fault::Silent { from, to } if (from..to).contains(&t) => fault,
                Fault::ClockSkew { from, to, .. } if (from..to).contains(&t) => fault,
                Fault::Equivocate { from, to } if (from..to).contains(&t) => fault,
                _ => Fault::None,
            };
            if std::mem::discriminant(&now) != std::mem::discriminant(&state) {
                for d in &disks {
                    d.set(Duration::ZERO);
                    d.set_skew_ms(0);
                }
                proposals[3].store(true, Ordering::SeqCst);
                match now {
                    Fault::None | Fault::SplitList { .. } | Fault::Silent { .. } | Fault::ClockSkew { .. } | Fault::Equivocate { .. } => {
                        link_all(&mut oracle, &keys, |_, _| true).await
                    }
                    Fault::Partition { .. } => {
                        link_all(&mut oracle, &keys, |i, j| (i < 2) == (j < 2)).await
                    }
                    Fault::Isolate { .. } => {
                        link_all(&mut oracle, &keys, |i, j| i != 3 && j != 3).await
                    }
                    Fault::SlowDisk { delay_ms, .. } => {
                        disks[3].set(Duration::from_millis(delay_ms))
                    }
                    Fault::SlowDisks { delay_ms, .. } => {
                        for d in &disks[2..] {
                            d.set(Duration::from_millis(delay_ms));
                        }
                    }
                }
                if let Fault::Silent { .. } = now {
                    proposals[3].store(false, Ordering::SeqCst);
                }
                if let Fault::ClockSkew { skew_ms, .. } = now {
                    disks[3].set_skew_ms(skew_ms);
                }
                if let Fault::Equivocate { .. } = now {
                    let (scheme, sender) = byzantine.as_mut().expect("validator 4");
                    send_conflicting_votes(scheme, sender, &keys[..3]);
                }
                if matches!(state, Fault::Equivocate { .. }) && matches!(now, Fault::None) {
                    for peer in &keys[..3] {
                        oracle.unblock(peer.clone(), keys[3].clone()).await.expect("heal peer");
                    }
                }
                state = now;
            }
            if let Fault::SplitList { .. } = now {
                list_for_half(&chains, t);
            }
            // A few payments per second, offered to every validator's mempool.
            if t % 2 == 0 {
                for from in 1..=4u8 {
                    let committed = chains[0]
                        .lock()
                        .finalized
                        .state
                        .nonce(&dev_accounts(4)[from as usize - 1].1);
                    let n = &mut nonces[from as usize - 1];
                    *n = (*n).max(committed);
                    let tx = transfer(from, *n, bob);
                    for c in &chains {
                        let _ = c.add_to_mempool(tx.clone());
                    }
                    *n += 1;
                    sent += 1;
                }
            }
            context.sleep(Duration::from_secs(1)).await;
            if matches!(fault, Fault::Equivocate { .. }) {
                equivocation_detected |= oracle.blocked().await.expect("blocked peers").iter().any(|(_, peer)| *peer == keys[3]);
            }
        }
        // Let everyone settle.
        context.sleep(Duration::from_secs(30)).await;

        let _ = sent;
        let mut result = outcome(&chains);
        result.equivocation_detected = equivocation_detected;
        result
    })
}

/// One lock at a time: a guard lives to the end of its statement.
fn outcome(chains: &[Chain]) -> Outcome {
    let heights = chains.iter().map(|c| c.finalized_height()).collect();
    let finalized = chains
        .iter()
        .map(|c| {
            c.lock()
                .blocks
                .iter()
                .map(|(h, b)| (*h, b.hash.clone()))
                .collect()
        })
        .collect();
    let roots = chains
        .iter()
        .map(|c| format!("{}", c.lock().finalized.state.root()))
        .collect();
    let (txs, timestamps) = {
        let g = chains[0].lock();
        (
            g.blocks.values().map(|b| b.txs.len()).sum(),
            g.blocks
                .values()
                .filter(|b| b.height > 0)
                .map(|b| b.timestamp_ms)
                .collect(),
        )
    };
    Outcome {
        heights,
        finalized,
        roots,
        txs,
        timestamps,
        equivocation_detected: false,
        handoff_switch: chains[0].lock().finalized.handoff.as_ref().map(|p| p.switch),
    }
}

#[test]
fn new_genesis_notice_needs_seven_days_of_certified_chain_time() {
    // Four real validators, with a proposer configured ten times faster than
    // the consensus floor. Virtual time keeps this test short.
    let out = simulate_with_rules(77, 25, Fault::None, true, 100);
    let height = out.heights.iter().copied().min().unwrap();
    assert!(height > 2, "the simulated committee must actually finalize blocks: {out:?}");
    assert!(height < MAINNET_NOTICE_BLOCKS);
    for (index, timestamp) in out.timestamps.iter().enumerate() {
        let h = index as u64 + 1;
        assert!(*timestamp >= h * MIN_BLOCK_INTERVAL_MS, "height {h} finalized too early at {timestamp}");
    }
    for pair in out.timestamps.windows(2) {
        assert!(pair[1] - pair[0] >= MIN_BLOCK_INTERVAL_MS, "a 100 ms proposer finalized blocks too close together: {pair:?}");
    }
    assert!(
        MAINNET_NOTICE_BLOCKS * MIN_BLOCK_INTERVAL_MS >= 604_800_000,
        "the first permitted activation height must require seven days of chain time"
    );
}

/// Stop the whole deterministic process at seeded virtual times. The next
/// runner keeps only Commonware's durable storage; every engine and network
/// channel is created anew, and Application replays its finalized archive.
fn simulate_restarts(seed: u64, with_handoff: bool) -> Vec<Outcome> {
    let cuts = [12 + seed % 8, 15 + (seed / 7) % 9, if with_handoff { 30 } else { 50 }];
    let handoff = with_handoff.then(signed_handoff);
    let identity = *devnet_threshold(N).1.public();
    let mut runner = deterministic::Runner::new(
        deterministic::Config::new().with_seed(seed).with_timeout(Some(Duration::from_secs(180))),
    );
    let mut outcomes = Vec::new();
    for (phase, secs) in cuts.into_iter().enumerate() {
        let pending = handoff.clone();
        let (o, checkpoint) = runner.start_and_recover(|context| async move {
            let keys: Vec<Pk> = (1..=N).map(|i| devnet_validator_key(i).public_key()).collect();
            let (network, mut oracle) = Network::new_with_peers(
                context.child("network"),
                simulated::Config {
                    max_size: MAX_BLOCK_BYTES + 1024 * 1024,
                    max_peers_per_set: NZUsize!(N as usize),
                    disconnect_on_block: true,
                    tracked_peer_sets: NZUsize!(1),
                },
                keys,
            )
            .await;
            network.start();
            let keys: Vec<Pk> = (1..=N).map(|i| devnet_validator_key(i).public_key()).collect();
            link_all(&mut oracle, &keys, |_, _| true).await;
            let chains: Vec<Chain> = (0..N).map(|_| Chain::new(chain_config()).0).collect();
            if let Some(h) = pending {
                for chain in &chains {
                    let mut g = chain.lock();
                    g.identity = Some(identity);
                    if phase == 1 {
                        g.handoff_ready = Some(h.clone());
                    }
                }
            }
            for (i, chain) in chains.iter().enumerate() {
                if phase == 1 && i == seed as usize % N as usize {
                    continue; // This validator remains down until the last phase.
                }
                start_validator(
                    &context, &oracle, i as u64 + 1, chain.clone(), Disk::default(),
                    Arc::new(AtomicBool::new(true)),
                ).await;
            }
            let bob = Address::repeat_byte(0xb0);
            for t in 0..secs {
                if t % 2 == 0 {
                    for from in 1..=4u8 {
                        let sender = dev_accounts(4)[from as usize - 1].1;
                        let nonce = chains[0].lock().finalized.state.nonce(&sender);
                        let tx = transfer(from, nonce, bob);
                        for chain in &chains {
                            let _ = chain.add_to_mempool(tx.clone());
                        }
                    }
                }
                context.sleep(Duration::from_secs(1)).await;
            }
            if phase == 2 {
                context.sleep(Duration::from_secs(if with_handoff { 5 } else { 30 })).await;
            }
            outcome(&chains)
        });
        outcomes.push(o);
        runner = deterministic::Runner::from(checkpoint);
    }
    outcomes
}

/// Longest wait between consecutive finalized blocks (virtual seconds).
fn block_gaps(o: &Outcome) -> u64 {
    let ts = &o.timestamps;
    ts.windows(2)
        .map(|w| (w[1] - w[0]) / 1000)
        .max()
        .unwrap_or(0)
}

/// Safety, liveness and agreement for one run.
fn check(o: &Outcome, min_height: u64) {
    // Safety: at every height, every validator that finalized it has the same block.
    let mut by_height: BTreeMap<u64, &String> = BTreeMap::new();
    for chain in &o.finalized {
        for (h, hash) in chain {
            let first = by_height.entry(*h).or_insert(hash);
            assert_eq!(*first, hash, "two different blocks finalized at height {h}");
        }
    }
    // Liveness and agreement: everyone caught up to the same head and state.
    let top = *o.heights.iter().max().unwrap();
    assert!(
        top >= min_height,
        "chain stalled at {top} (wanted {min_height}): {:?}",
        o.heights
    );
    assert!(
        o.heights.iter().all(|h| top - h <= 2),
        "a validator fell behind: {:?}",
        o.heights
    );
    let at_min = o.heights.iter().min().unwrap();
    let hashes: Vec<_> = o.finalized.iter().map(|c| c.get(at_min).cloned()).collect();
    assert!(
        hashes.windows(2).all(|w| w[0] == w[1]),
        "validators disagree at height {at_min}"
    );
    if o.heights.iter().all(|h| *h == top) {
        assert!(o.roots.windows(2).all(|w| w[0] == w[1]), "state roots disagree at the same height");
    }
    assert!(o.txs > 0, "no transactions were finalized");
}

#[test]
fn validators_restart_from_durable_journals_at_seeded_points() {
    for seed in [61, 67] {
        check_restart_phases(&simulate_restarts(seed, false));
    }
}

fn check_restart_phases(phases: &[Outcome]) {
    let mut finalized = BTreeMap::new();
    for phase in phases {
        for chain in &phase.finalized {
            for (height, hash) in chain {
                if let Some(previous) = finalized.insert(*height, hash) {
                    assert_eq!(previous, hash, "conflicting finalization across restarts at {height}");
                }
            }
        }
    }
    let middle_head = *phases[1].heights.iter().max().unwrap();
    let recovered_tail = *phases[2].heights.iter().min().unwrap();
    assert!(recovered_tail > middle_head + 20, "restart did not restore liveness");
    check(phases.last().unwrap(), 45);
}

#[test]
fn restart_during_signed_handoff_preserves_the_pending_switch() {
    let phases = simulate_restarts(61, true);
    let switch = phases[1].handoff_switch.expect("handoff finalized before crash");
    assert!(phases[1].heights.iter().all(|h| *h < switch), "crash is inside handoff delay");
    assert_eq!(phases[2].handoff_switch, Some(switch), "restarted nodes forgot the handoff");
    check_restart_phases(&phases);
}

#[test]
fn healthy_network_finalizes_and_agrees() {
    for seed in 0..3 {
        let o = simulate(seed, 60, Fault::None);
        eprintln!("healthy seed {seed}: heights {:?} txs {}", o.heights, o.txs);
        check(&o, 40);
    }
}

#[test]
fn partition_without_quorum_stalls_then_heals() {
    let o = simulate(11, 90, Fault::Partition { from: 20, to: 50 });
    // Blocks are ~1 s apart except across the partition, where nothing can finalize.
    let times = block_gaps(&o);
    eprintln!(
        "partition: heights {:?} txs {} longest gap {:?} s",
        o.heights, o.txs, times
    );
    assert!(
        times >= 25,
        "the chain kept finalizing without a quorum (longest gap {times} s)"
    );
    check(&o, 45);
}

#[test]
fn isolated_validator_does_not_stop_the_chain_and_catches_up() {
    let o = simulate(21, 90, Fault::Isolate { from: 15, to: 60 });
    check(&o, 60);
}

/// Regression for the 2026-09-28 testnet stall (height 69651, view 69842).
/// Voters that disagree about an inclusion list must disagree before the
/// notarize vote, never at certification: a block half the committee refuses
/// to certify is notarized but can be neither finalized nor nullified, and the
/// chain stops for good. With the check before the notarize vote, blocks from
/// 2 and 4 miss the quorum and those views pass; blocks from 1 and 3 finalize.
#[test]
fn split_inclusion_list_does_not_stall_the_chain() {
    let o = simulate(31, 90, Fault::SplitList { from: 10, to: 60 });
    eprintln!(
        "split list: heights {:?} txs {} longest gap {} s",
        o.heights,
        o.txs,
        block_gaps(&o)
    );
    check(&o, 45);
    assert!(
        block_gaps(&o) < 20,
        "the chain stalled while validators disagreed about a list"
    );
}

/// A slow disk on one validator (every write and sync 400 ms): the other three
/// still form quorums, so the chain keeps finalizing at nearly full speed, and
/// the slow validator keeps up once its disk recovers.
#[test]
fn one_slow_disk_does_not_stop_the_chain() {
    let o = simulate(41, 90, Fault::SlowDisk { from: 10, to: 60, delay_ms: 400 });
    eprintln!(
        "slow disk: heights {:?} txs {} longest gap {} s",
        o.heights,
        o.txs,
        block_gaps(&o)
    );
    check(&o, 60);
    assert!(block_gaps(&o) < 10, "one slow disk held the chain up");
}

/// Disks stalled (30 s per write and sync) on half the committee: no quorum
/// can vote, so the chain stops, then resumes on its own once the disks do,
/// with no two validators finalizing different blocks.
#[test]
fn stalled_disks_stall_then_resume_safely() {
    let o = simulate(43, 120, Fault::SlowDisks { from: 20, to: 50, delay_ms: 30_000 });
    let gap = block_gaps(&o);
    eprintln!(
        "stalled disks: heights {:?} txs {} longest gap {gap} s",
        o.heights, o.txs
    );
    assert!(gap >= 20, "the chain kept finalizing on stalled disks (longest gap {gap} s)");
    check(&o, 50);
}

#[test]
fn silent_voter_does_not_stop_other_leaders() {
    let o = simulate(51, 90, Fault::Silent { from: 15, to: 60 });
    check(&o, 50);
    assert!(block_gaps(&o) < 15, "one silent proposer stalled the chain");
}

#[test]
fn a_skewed_clock_cannot_finalize_conflicting_blocks() {
    let o = simulate(53, 90, Fault::ClockSkew { from: 15, to: 60, skew_ms: 2_000 });
    check(&o, 45);
}

#[test]
fn equivocating_vote_is_detected_and_ignored() {
    let o = simulate(55, 90, Fault::Equivocate { from: 15, to: 40 });
    assert!(o.equivocation_detected, "honest peers did not block the double voter");
    check(&o, 45);
}

#[test]
fn same_seed_same_chain() {
    let a = simulate(7, 40, Fault::Isolate { from: 10, to: 20 });
    let b = simulate(7, 40, Fault::Isolate { from: 10, to: 20 });
    assert_eq!(a, b, "a run must be reproducible from its seed");
}

/// Long soak: `AETHER_SIM_SEEDS=100 AETHER_SIM_SECS=3600 cargo test --release -p aether-node --test sim -- --ignored`.
#[test]
#[ignore]
fn soak() {
    let seeds: u64 = std::env::var("AETHER_SIM_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let secs: u64 = std::env::var("AETHER_SIM_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    for seed in 0..seeds {
        let fault = match seed % 3 {
            0 => Fault::None,
            1 => Fault::Partition {
                from: secs / 4,
                to: secs / 4 + 30,
            },
            _ => Fault::Isolate {
                from: secs / 3,
                to: secs / 3 + secs / 5,
            },
        };
        let o = simulate(1_000 + seed, secs, fault);
        check(&o, secs / 3);
        eprintln!(
            "seed {seed}: {fault:?} height {:?} txs {}",
            o.heights, o.txs
        );
    }
}
