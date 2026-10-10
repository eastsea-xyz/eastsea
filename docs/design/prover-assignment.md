# Local prover assignment (0.7.4)

The prover schedules work using the operators in its current finalized registry.
For height `h`, rank unique operator addresses by
`BLAKE3("aether/prover-assignment/v1" || h_be_u64 || operator_20_bytes)` in descending
hash order, breaking a hash tie by ascending address. The first `min(k, N)`
operators are designated. Registry enumeration order and multiple candidates
belonging to one operator do not change its tickets.

The payout address in `AETHER_PROVE` identifies this prover's operator. A payout
address absent from the registry waits for grace expiry, as does a node on a
chain without registered candidates. Selection uses the current registry at
each job dispatch; registry changes can therefore change designation for an
older height. A proof already running is not interrupted by a registry change.

Inside the last W finalized unproven statements, take the oldest designated job
first. When there are no designated jobs, take the oldest job whose age strictly
exceeds T. Age is wall-clock time minus the block's finalized timestamp, clamped
to zero for a timestamp in the future. Quiet blocks that record no statement
do not occupy the window. Only an enabled proving service retains statements. It stores compact encoded
stateless inputs, so a grace rescue can replay after the ordinary 64-height
execution cache has moved on. Statements and parents never pin full states.
Full states, compact inputs, their containers, summaries, and receipts count
toward `--max-memory`, with 25% reserved for working allocations. Optional old
states leave first, then the oldest inputs if necessary. The finalized head,
its immediate parent, and history without a durable backing remain mandatory;
a budget smaller than that working set cannot be satisfied through eviction.

Node-local environment settings:

| Setting | Default | Meaning |
| --- | ---: | --- |
| `AETHER_PROVER_ASSIGNMENT_K` | 3 | Designated unique operators per height (1–4096) |
| `AETHER_PROVER_GRACE_SECS` | 60 | Seconds before any operator may start a rescue |
| `AETHER_PROVER_WINDOW` | 128 | Maximum compact unproven inputs, also limited by bytes (1–4096) |

Malformed settings leave the prover stopped with an error in its status. These
settings do not affect proof validity, block execution, reward issuance, genesis,
or the existing first-valid-proof-wins rule. An older client or a client choosing
to race every block can still win a reward. Assignment is cooperation, not an
on-chain access restriction.

After proof verification succeeds, pool admission sends an immediate local
notice to the proving service. Finalizing a block carrying a proof sends the
same notice, covering followers that did not see the validator's pool. The notice
kills only the separate proving sidecar for that height. A replacement starts
without the crash watchdog's back-off. A proof arriving during witness building
or at proof completion is checked again before work starts or submission occurs.
Invalid and unavailable verdicts never cancel work. A verified loss is remembered
even if that proof subsequently leaves the pool; sidecar failure cannot reopen
that job. Followers observe another validator's pending proof when it reaches
their own verified pool or finalized history, not through an unverified RPC hint.

`aether_proverStatus` adds `assignment_k`, `grace_seconds`, `window`, `cancelled`,
`last_cancelled_height`, and `last_abort_latency_ms`. Latency measures from the
verified-proof notice through the stopped sidecar request, including notification
delivery and process exit. It is distinct from network propagation or proof
verification latency.

A finite window and finite proving capacity have limits: if more than W open
inputs accumulate, the oldest input leaves local retention. The byte budget can shorten W; increasing W cannot bypass it. At one new provable block per second, two 20-second provers have capacity
for at most 0.1 unique proofs per second, even with perfect assignment. Grace
provides eligibility, not additional GPU capacity; it cannot guarantee every
block finishes by T plus proof time under overload or an arbitrarily long outage.
Provision enough online capacity, and keep W larger than the maximum backlog
during grace plus proving and submission. The simulation reports these limits
alongside duplicated work and reward shares.

Validation uses production selection in deterministic mixed-speed simulations,
four independent in-memory devnet peers with real registry registration and
verified proof inclusion, and fake sidecar fault tests for cancellation and
restart latency. The devnet uses padded commitment-echo proofs to exercise
inclusion without GPU proving; it is not a live network or a GPU benchmark.

See [simulation results and assumptions](prover-assignment-simulation.md).

H04 review follow-up and verification status: [retention qualification](prover-assignment-h04.md).
