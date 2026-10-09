# Prover retention review follow-up — 2026-10-08

Baseline: `1561f1202d50241f38d7be677ebe697e6433383b`. Findings: H04, M05 and M06 in `ef9c6ea:docs/audit/consensus-review-2026-10-08.md`.

H04 source changes disable retention until a prover starts, remove all statement/parent state pins, and retain postcard-compatible compact witnesses for longer grace rescues. Existing finalized execution remains the witness oracle. Input construction, decoding and replay run outside the chain mutex. Memory accounting includes conservative full-tree allocation estimates, code, journals, receipts, encoded inputs, block payloads and container capacities. Optional retention uses at most 75% of the history budget; head, immediate parent and undurable history form an unavoidable minimum. These are allocation estimates, not a measured process RSS cap. Oversized backlogs lose oldest optional inputs under the byte limit.

M05 source changes retry proposal construction without unrelated registration/beacon extras before removing proofs, preserving accepted claims through registrar rotation and verifier unavailability. Two regressions use real registrar activation, signatures, inclusion, payment-once and sidecar retry bookkeeping.

M06 mitigation snapshots scheduling metadata, performs registry extraction and assignment hashing outside the consensus mutex, and rejects stale/paid/attempted/missing work after reacquiring it. **M06 remains partial:** roster/per-height caching and a large-registry concurrency qualification are still needed. An uncached scan can outlast a block interval and repeatedly fail stale-head revalidation.

Added H04 regressions:

- `h04_disabled_proving_keeps_only_the_ordinary_state_versions`
- `h04_adjacent_unpaid_history_fits_budget_and_can_rescue`
- `h04_sparse_unpaid_history_fits_budget_and_can_rescue`

The enabled fixtures use the maximum 4096 window and an 8 MiB scaled version of the documented 8 GB Mac's 1 GiB history budget, checking state payload bytes, charged bytes, 25% headroom and native replay of the oldest grace input. An existing four-peer regression now replays a compact input after 80 heights.

Verification: source parsing with rustfmt and `git diff --check` passed. **Baseline RED execution, GREEN execution, node tests and clippy remain unexecuted while the shared compile semaphore is unavailable.** No regression failure/pass or runtime budget/RSS measurement is claimed. A gated baseline run was queued at 14:13 UTC; the lane stops after 20 minutes of slot waiting as instructed. No guest, remote host, real-node directory, app launch or push is involved.

Task-local handoff artifacts (uncommitted): `tmp/h04-baseline/` is an archive of the immutable baseline; `tmp/h04-tests.patch` and `tmp/m05-tests.patch` transplant the new regressions (the latter includes the original fallback as a test-only helper). `tmp/h04-evidence/red.log` records the gate outcome. All Cargo commands must retain `CARGO_BUILD_JOBS=4` and call `wait-compile.sh` in the same shell command. Run baseline `--lib h04_` and `--lib proof_recovery_tests`, then fixed `cargo test -j4 -p aether-node --tests` including existing resource-budget tests, and `cargo clippy -j4 -p aether-node --all-targets -- -D warnings`. None requires a Jolt guest build.
