//! Deterministic scheduling model, not a consensus or GPU benchmark.
//! See docs/design/prover-assignment-simulation.md for its capacity limits.

use aether_node::prover_assignment::{designated, select, Config, OpenBlock};
use aether_types::Address;
use std::collections::BTreeSet;

const PROOF_SECONDS: [u64; 3] = [10, 20, 30];
const WARMUP: u64 = 200;
const PRODUCE_SECONDS: u64 = 1_200;
const DRAIN_SECONDS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    NewestRace,
    NewestWithCancellation,
    Assignment,
}

impl Policy {
    fn cancels(self) -> bool {
        self != Self::NewestRace
    }
}

#[derive(Clone, Copy)]
struct Job {
    block: usize,
    started: u64,
    finishes: u64,
}

struct Worker {
    address: Address,
    cohort: usize,
    online: bool,
    job: Option<Job>,
    attempted: BTreeSet<u64>,
}

struct Block {
    open: OpenBlock,
    assigned: Vec<Address>,
    retained: bool,
    winner: Option<usize>,
    finished: Option<u64>,
    gpu_seconds: u64,
}

struct Report {
    policy: Policy,
    nodes: usize,
    measured: usize,
    proven: usize,
    proven_at_stop: usize,
    unproven: usize,
    evicted: usize,
    duplicate_seconds: u64,
    pending_seconds: u64,
    mean_age: f64,
    max_age: u64,
    oldest_pending: u64,
    rewards: Vec<u64>,
    cohorts: Vec<usize>,
    cancellations: usize,
}

fn address(index: usize) -> Address {
    let mut bytes = [0_u8; 20];
    bytes[12..].copy_from_slice(&(index as u64 + 1).to_be_bytes());
    Address::from(bytes)
}

fn workers(n: usize) -> Vec<Worker> {
    (0..n)
        .map(|i| Worker {
            address: address(i),
            // Two Macs deliberately cover both ends of the speed range.
            cohort: if n == 2 { i * 2 } else { i % 3 },
            online: true,
            job: None,
            attempted: BTreeSet::new(),
        })
        .collect()
}

// Same-speed proofs can arrive together in virtual time. Give each operator
// a deterministic, block-specific arrival rank rather than favoring index 0.
fn arrival_rank(height: u64, operator: Address) -> [u8; 32] {
    *blake3::Hasher::new()
        .update(b"aether/prover-simulation/arrival/v1")
        .update(&height.to_be_bytes())
        .update(operator.as_slice())
        .finalize()
        .as_bytes()
}

fn gini(rewards: &[u64]) -> f64 {
    let mut sorted = rewards.to_vec();
    sorted.sort_unstable();
    let total = sorted.iter().sum::<u64>();
    if total == 0 || sorted.is_empty() {
        return 0.0;
    }
    let n = sorted.len() as f64;
    sorted
        .iter()
        .enumerate()
        .map(|(i, reward)| (2.0 * (i + 1) as f64 - n - 1.0) * *reward as f64)
        .sum::<f64>()
        / (n * total as f64)
}

fn simulate(
    policy: Policy,
    mut workers: Vec<Worker>,
    registry: &[Address],
    produce: u64,
    warmup: u64,
) -> Report {
    let config = Config::default();
    let stop = produce + DRAIN_SECONDS;
    let mut blocks: Vec<Block> = Vec::new();
    let mut cancellations = 0;
    let mut evicted = 0;
    let mut proven_at_stop = 0;

    // One-second events match idle polling. Completion and proof visibility
    // precede dispatch; workers neither preempt for newer blocks nor run two
    // jobs. Cancellation observation is idealized as immediate.
    for now in 0..=stop {
        if now < produce {
            let height = now + 1;
            blocks.push(Block {
                open: OpenBlock {
                    height,
                    timestamp_ms: now * 1_000,
                },
                assigned: designated(height, registry, config.designated),
                retained: true,
                winner: None,
                finished: None,
                gpu_seconds: 0,
            });
            // Retained inputs are bounded in every policy. An evicted input
            // cannot silently reappear when newer jobs finish. In-flight work
            // may still finish and claim its original block.
            let retained: Vec<_> = blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.retained && b.winner.is_none())
                .map(|(i, _)| i)
                .collect();
            for index in retained
                .iter()
                .take(retained.len().saturating_sub(config.window))
            {
                blocks[*index].retained = false;
                if blocks[*index].open.timestamp_ms / 1_000 >= warmup {
                    evicted += 1;
                }
            }
        }

        let mut completed: Vec<_> = workers
            .iter()
            .enumerate()
            .filter_map(|(i, w)| w.job.filter(|j| j.finishes == now).map(|j| (i, j)))
            .collect();
        completed.sort_unstable_by_key(|(i, j)| {
            (
                j.block,
                arrival_rank(blocks[j.block].open.height, workers[*i].address),
            )
        });
        for (worker, job) in completed {
            let block = &mut blocks[job.block];
            block.gpu_seconds += now - job.started;
            if block.winner.is_none() {
                block.winner = Some(worker);
                block.finished = Some(now);
            }
            workers[worker].job = None;
        }

        if policy.cancels() {
            for worker in &mut workers {
                if let Some(job) = worker.job.filter(|j| blocks[j.block].winner.is_some()) {
                    blocks[job.block].gpu_seconds += now - job.started;
                    worker.job = None;
                    cancellations += 1;
                }
            }
        }
        if now == produce {
            proven_at_stop = blocks
                .iter()
                .filter(|b| b.open.timestamp_ms / 1_000 >= warmup && b.winner.is_some())
                .count();
        }
        if now == stop {
            break;
        }

        let open: Vec<_> = blocks
            .iter()
            .filter(|b| b.retained && b.winner.is_none())
            .map(|b| b.open)
            .collect();
        for worker in &mut workers {
            if !worker.online || worker.job.is_some() {
                continue;
            }
            let available: Vec<_> = open
                .iter()
                .filter(|b| !worker.attempted.contains(&b.height))
                .copied()
                .collect();
            let height = match policy {
                Policy::NewestRace | Policy::NewestWithCancellation => {
                    available.last().map(|b| b.height)
                }
                Policy::Assignment => {
                    // Cached production designation is only an idle hint.
                    // The production selector makes every actual decision
                    // against the complete available retained window.
                    let eligible = available.iter().any(|b| {
                        blocks[(b.height - 1) as usize]
                            .assigned
                            .contains(&worker.address)
                            || now * 1_000 - b.timestamp_ms > config.grace.as_millis() as u64
                    });
                    if eligible {
                        select(&available, registry, worker.address, now * 1_000, &config)
                    } else {
                        None
                    }
                }
            };
            if let Some(height) = height {
                assert!(
                    worker.attempted.insert(height),
                    "a worker retried a lost block"
                );
                worker.job = Some(Job {
                    block: (height - 1) as usize,
                    started: now,
                    finishes: now + PROOF_SECONDS[worker.cohort],
                });
            }
        }
    }

    // Charge unfinished work honestly. Work on an already-won block is waste;
    // work on an unproven block remains pending rather than a claimed saving.
    for worker in &workers {
        if let Some(job) = worker.job {
            blocks[job.block].gpu_seconds += stop - job.started;
        }
    }
    let mut report = Report {
        policy,
        nodes: workers.len(),
        measured: 0,
        proven: 0,
        proven_at_stop,
        unproven: 0,
        evicted,
        duplicate_seconds: 0,
        pending_seconds: 0,
        mean_age: 0.0,
        max_age: 0,
        oldest_pending: 0,
        rewards: vec![0; workers.len()],
        cohorts: workers.iter().map(|w| w.cohort).collect(),
        cancellations,
    };
    for block in blocks
        .iter()
        .filter(|b| b.open.timestamp_ms / 1_000 >= warmup)
    {
        report.measured += 1;
        let finalized = block.open.timestamp_ms / 1_000;
        if let Some(winner) = block.winner {
            report.proven += 1;
            report.rewards[winner] += 1;
            let age = block.finished.unwrap() - finalized;
            report.mean_age += age as f64;
            report.max_age = report.max_age.max(age);
            report.duplicate_seconds += block.gpu_seconds - PROOF_SECONDS[workers[winner].cohort];
        } else {
            report.unproven += 1;
            report.oldest_pending = report.oldest_pending.max(stop - finalized);
            report.pending_seconds += block.gpu_seconds;
        }
    }
    report.mean_age /= report.proven.max(1) as f64;
    report
}

impl Report {
    fn duplicate_per_finalized(&self) -> f64 {
        self.duplicate_seconds as f64 / self.measured as f64
    }

    fn print(&self) {
        println!(
            "N={} {:?}: proven={}/{}, throughput={:.3}/s, duplicate_gpu_s/finalized={:.3}, duplicate_gpu_s/proven={:.3}, pending_gpu_s={}, age_mean/max={:.2}/{}s, unproven={}, oldest_pending={}s, window_evictions={}, cancellations={}, Gini={:.4}",
            self.nodes, self.policy, self.proven, self.measured,
            self.proven_at_stop as f64 / self.measured as f64,
            self.duplicate_per_finalized(), self.duplicate_seconds as f64 / self.proven.max(1) as f64,
            self.pending_seconds, self.mean_age, self.max_age, self.unproven,
            self.oldest_pending, self.evicted, self.cancellations, gini(&self.rewards),
        );
        for (cohort, seconds) in PROOF_SECONDS.iter().enumerate() {
            let members: Vec<_> = self
                .rewards
                .iter()
                .enumerate()
                .filter(|(i, _)| self.cohorts[*i] == cohort)
                .map(|(_, r)| *r)
                .collect();
            if members.is_empty() {
                continue;
            }
            let share = members.iter().sum::<u64>() as f64 / self.proven.max(1) as f64;
            println!("  {}s Macs={}: cohort_share={:.3}%, mean_share_per_Mac={:.4}%, paid={}/{}, wins_min/max={}/{}",
                seconds, members.len(), share * 100.0, share * 100.0 / members.len() as f64,
                members.iter().filter(|&&r| r > 0).count(), members.len(), members.iter().min().unwrap(), members.iter().max().unwrap());
        }
    }
}

#[test]
fn mixed_speed_assignment_reduces_waste_and_reward_concentration() {
    println!("warmup={}s, measurement={}s, drain={}s, block_rate=1/s, k=3, grace=60s, W=128, observation_latency=0s", WARMUP, PRODUCE_SECONDS - WARMUP, DRAIN_SECONDS);
    for n in [2, 16, 300] {
        let registry: Vec<_> = workers(n).iter().map(|w| w.address).collect();
        let legacy = simulate(
            Policy::NewestRace,
            workers(n),
            &registry,
            PRODUCE_SECONDS,
            WARMUP,
        );
        let cancel_only = simulate(
            Policy::NewestWithCancellation,
            workers(n),
            &registry,
            PRODUCE_SECONDS,
            WARMUP,
        );
        let assignment = simulate(
            Policy::Assignment,
            workers(n),
            &registry,
            PRODUCE_SECONDS,
            WARMUP,
        );
        legacy.print();
        cancel_only.print();
        assignment.print();
        if n == 300 {
            let legacy_reduction =
                1.0 - assignment.duplicate_per_finalized() / legacy.duplicate_per_finalized();
            let selection_reduction =
                1.0 - assignment.duplicate_per_finalized() / cancel_only.duplicate_per_finalized();
            println!(
                "N=300 duplicate reduction: historical={:.2}%, cancellation-only={:.2}%",
                legacy_reduction * 100.0,
                selection_reduction * 100.0
            );
            assert!(
                legacy_reduction >= 0.90,
                "historical reduction={legacy_reduction}"
            );
            assert!(
                selection_reduction >= 0.90,
                "selection reduction={selection_reduction}"
            );
            assert!(gini(&assignment.rewards) < gini(&legacy.rewards));
            assert!(gini(&assignment.rewards) < gini(&cancel_only.rewards));
            assert_eq!(assignment.proven, assignment.measured);
            // This provisioned 300-worker workload meets T + maximum proof
            // time; isolated grace rescue also measures the dispatch tick.
            assert!(
                assignment.max_age <= 60 + PROOF_SECONDS[2],
                "maximum age={}s",
                assignment.max_age
            );
            assert_eq!(assignment.evicted, 0);
        } else {
            // Capacity and duplicated attempts prevent a universal bound.
            // Do not hide the outstanding queue by measuring only winners.
            assert!(
                assignment.unproven > 0,
                "unexpectedly cleared overloaded N={n}"
            );
            assert!(assignment.oldest_pending > 60 + PROOF_SECONDS[2]);
        }
    }
}

#[test]
fn offline_designated_provers_are_rescued_after_grace_with_spare_capacity() {
    let registry: Vec<_> = (0..3).map(address).collect();
    let mut rescuers = workers(1);
    rescuers[0].address = address(100); // Unregistered; all designated operators are offline.
    let report = simulate(Policy::Assignment, rescuers, &registry, 1, 0);
    report.print();
    assert_eq!(report.proven, 1);
    assert_eq!(report.max_age, 61 + PROOF_SECONDS[0]);
    assert_eq!(report.duplicate_seconds, 0);
    assert_eq!(report.evicted, 0);
}

#[test]
fn fairness_counts_zero_reward_operators() {
    assert_eq!(gini(&[1, 1, 1]), 0.0);
    assert!((gini(&[1, 0, 0]) - 2.0 / 3.0).abs() < 1e-12);
    assert_eq!(gini(&[0, 0, 0]), 0.0);
}
