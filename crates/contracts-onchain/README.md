# Contracts on the EastSea executor

This workspace package deploys Foundry-compiled creation bytecode through
`aether_execution::execute_block`, with signed P-256 envelopes and paid state.
It does not start a node, an app, or a JSON-RPC server.

From the repository root:

```sh
scripts/run-contracts-onchain.sh --with-node
```

The runner serializes Cargo, enforces idle Cargo/rustc and 25% free memory,
limits compilation and Rayon to four threads, and runs tests serially. It
sets `TMPDIR` and `CARGO_TARGET_DIR` inside root `tmp/`, writes measured costs
to the research report, and removes scratch/build products after success.
Process-list denial is a hard stop, not an idle-host indication.

`--with-node` additionally runs the existing `state_budget` and `zero_fee`
node integration tests, including full encoded-payload archive limits,
certification and restart. Without it, only the contract executor suite and
Foundry core/fixture regressions run.

Checked-in artifact fixtures allow Cargo tests to run without Foundry. To
regenerate them separately:

```sh
python3 scripts/generate-contract-fixtures.py --offline
```

See [fixture provenance and compiler settings](fixtures/README.md) and the
[research report](../../docs/research/contracts-onchain-2026-10-06.md).
The report distinguishes unexecuted Rust cases from passing Foundry evidence.
Do not claim complete chain coverage until the guarded Rust run succeeds.

The harness is a library (`aether_contracts_onchain::harness`), so a
Foundry-built contract can be measured from any test in this package. The
A0 workflow recorder, schema, account recipes (P-256 self batch, added-owner
relay, ERC-1271) and the deterministic local brake interface are documented in
[RECORDER.md](RECORDER.md). The runner writes workflow records to
`tmp/contracts-onchain-workflows.jsonl` and summarizes them in the report.

The test modules cover core escrow/distribution, identity/delegation/vaults,
assets/names, payments, DeFi/governance, the ERC721 market, chain resource
limits, the A0 recorder workflows and the reference brake. The harness
centralizes replay, nonce, wallet-budget, archive-fee and
rollback assertions. Repeated deployment configurations produce cost ranges;
fits/day estimates are state-budget upper bounds, not throughput promises.
