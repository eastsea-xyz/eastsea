# Prover assignment: scheduling simulation

The integration test `crates/node/tests/prover_assignment_sim.rs` compares
three local scheduling strategies at 1 finalized block per second. Proof
validity and rewards are identical: the first valid proof receives one reward.
There are no genesis, reward-contract, or consensus changes.

Run through the shared compiler gate before invoking Cargo:

```sh
~/.claude/playbooks/aether-team/wait-compile.sh
cargo test -j4 -p aether-node --test prover_assignment_sim -- --nocapture
```

The lane owner runs the complete required gate, `cargo test -j4 -p aether-node
--tests`, and records measured output. This report distinguishes model
assumptions from observed hardware behavior; it is not a four-prover devnet
or proof-inclusion result.

## Model and comparisons

The deterministic model uses 10, 20, and 30 second proof-time cohorts. Each
operator represents one Mac and runs at most one proof at a time. N=2 has
one 10 second and one 30 second Mac. N=16 has 6/5/5 Macs, and N=300 has
100/100/100. All Macs and registry entries are online except in the separate
grace-rescue test. All finalized blocks require proofs.

Each run has 200 seconds of warmup, 1,000 seconds of measured block arrivals,
and 120 seconds of drain. Reports count blocks by their finalization time;
they include measured blocks completed during drain. Steady throughput counts
only completions before arrivals stop, divided by the 1,000 second measured
interval. Outstanding blocks, their oldest age, window evictions, and GPU
seconds spent on unfinished proofs remain visible.

| Strategy | Selection | Cancellation |
| --- | --- | --- |
| Historical requested baseline | Newest open block | Workers finish losing proofs |
| Cancellation-only control | Newest open block | Immediate when another valid proof wins |
| Assignment | Production `select`: oldest designated, then oldest after grace | Immediate when another valid proof wins |

The initial task describes a newest-block race. The lane's starting branch
already used oldest-first selection in `Chain::provable`; the historical
requested baseline above is explicit and should not be read as a measurement
of that branch's exact behavior. The cancellation-only comparison isolates
the effect of assignment from the effect of cancellation.

Assignment uses the actual production `designated` and `select` functions,
with k=3, T=60 seconds, and W=128. A cached result from production
`designated` is an idle eligibility hint; every dispatched assignment job is
selected by production `select` against the complete available retained
window. Proven and previously attempted heights are excluded. A worker
cannot restart a height it already attempted.

Events happen on one-second dispatch ticks. Completions precede cancellation
and dispatch. Busy workers keep their job when newer blocks arrive. A
completed or cancelled worker can start another eligible job on that tick.
Same-speed completion ties use a separate deterministic hash of height and
operator, avoiding fixed worker-index reward bias. The hash does not affect
assignment eligibility or proof duration.

All three strategies retain at most 128 unproven inputs; an input that leaves
the retained window cannot silently return. An already running proof may
still complete after its input leaves the window. Using the same retention
limit makes backlog and overflow visible in every comparison.

Proof observation and cancellation are instantaneous in this model. The
simulation's cancellation count is not a measured sidecar abort latency;
real polling, network delivery, process termination, and resubmission require
separate runtime validation.

## Metrics and executable acceptance

Duplicated GPU seconds are all GPU time spent on a proven block except its
winning proof. This includes cancelled partial attempts and completed losing
attempts. Reports normalize this both per measured finalized block and per
proven block, because newest-race strategies leave many blocks unproven.
GPU time on still-unproven blocks is reported separately rather than counted
as a saving. This avoids comparing a strategy that barely proves anything
against a strategy completing the whole workload using only raw totals.

The test prints each cohort's reward share, its mean share per Mac, the
number of Macs paid, and the minimum and maximum rewards in that cohort.
Gini includes every participating Mac, including zero-reward Macs.

At N=300 the test requires:

- At least 90% less duplicated GPU time per finalized block than both the
  historical baseline and cancellation-only control.
- Lower reward Gini than both controls.
- Every measured block proven, no window eviction, and maximum measured
  block age at most 90 seconds in this deterministic run.

These are assertions for this workload and deterministic operator set, not
universal network or economic guarantees. Hardware acceleration, failed
proofs, network latency, a changing registry, operator behavior, and
multiple Macs under one operator are outside this model.

## Expected reward share and capacity

The following large-sample calculation explains expected reward skew before
queueing and finite-run variation. It assumes all designated workers are
immediately idle, samples k distinct registered operators uniformly, and
shares same-speed ties symmetrically. For cohorts F/M/S, fast wins when at
least one fast operator is sampled; medium wins when no fast but at least
one medium is sampled; slow wins when all sampled operators are slow.

With `C(n,k)` denoting combinations, cohort shares are
`1 - C(M+S,k)/C(N,k)`, `(C(M+S,k) - C(S,k))/C(N,k)`, and
`C(S,k)/C(N,k)` respectively. Divide a cohort share by its operator count to
obtain the expected reward share per Mac. When N<k, sample all N operators.

| Macs (F/M/S) | Newest race expected share per Mac F/M/S | Assignment idle-limit expected share per Mac F/M/S |
| --- | --- | --- |
| 2 (1/0/1) | 100% / absent / 0% | 100% / absent / 0% |
| 16 (6/5/5) | 16.667% / 0% / 0% | 13.095% / 3.929% / 0.357% |
| 300 (100/100/100) | 1.000% / 0% / 0% | 0.70519% / 0.25851% / 0.03630% |

The printed discrete-event results include busy workers and queued jobs, so
they need not equal the idle-limit estimates. Assignment broadens access to
rewards; it does not guarantee every Mac receives a reward in a finite run.
First-valid-proof economics still favor faster operators within each group
of designated provers. The analytic newest-race Gini is 0.5/0.625/0.6667 for
N=2/16/300; finite-run Gini is printed by the test.

Capacity alone rules out universal liveness claims at small N. With one
10 second and one 30 second worker, even perfect assignment without duplicate
work can produce at most `1/10 + 1/30 = 0.1333` proofs per second. With
6/5/5 workers, the ideal maximum is `6/10 + 5/20 + 5/30 = 1.0167` proofs per
second, leaving almost no allowance for three-worker racing, cancellation,
or failures. The model explicitly asserts an outstanding, overdue queue in
these cases. A finite window cannot preserve every block during sustained
overload; evicted inputs require a separate recovery mechanism.

N=300 has an ideal capacity of 18.333 proofs per second, providing ample
slack for this workload. Aggregate capacity is necessary but is not alone
sufficient for a universal deadline: designated-first priority, bursts,
window retention, offline operators, and dispatch delay still matter.

The separate rescue case finalizes one block with all three designated
registered operators offline and an idle unregistered 10 second worker.
The block becomes eligible strictly after T, is dispatched at second 61,
and completes at second 71 with no duplicated work. Strict `age > T` plus
one-second idle polling means a feasible deadline is `T + dispatch tick +
proof time`, with observation and scheduling delay added in a real deployment.

## Measured output

Execution is pending behind the `integrate-073` release hold for 0.7.3. The
required `wait-compile.sh` has been invoked; no simulation runtime values or
GPU measurements are claimed while that gate remains closed.

The deterministic test prints the full nine-row comparison and speed-cohort
breakdown when run with `--nocapture`. Record values from the compiler-gated
lane run here; the analytic table above is not substituted for test output.
