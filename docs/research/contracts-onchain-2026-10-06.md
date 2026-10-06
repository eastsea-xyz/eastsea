# EastSea contract execution research — 2026-10-06

**The guarded run `scripts/run-contracts-onchain.sh --with-node` passed on 2026-10-06: Rust executor suite 66/66, node `state_budget` 6/6 and `zero_fee` 13/13, Foundry core 161/161 and toolbox fixture regressions 14/14.** The measurements below are real executor records from that run. Foundry is a different execution environment; its results are reported separately. Work stayed on `codex/contracts-onchain`; no GUI, running node, `~/aether-testnet`, merge, push, or original toolbox write occurred.

## Reproduction

From this worktree, on a host where process listing is permitted:

```sh
scripts/run-contracts-onchain.sh --with-node
```

The runner checks both `pgrep -x rustc` and `pgrep -f 'cargo (build|test)'`, waits 60 seconds while either is running or `memory_pressure` reports less than 25% free, and refuses to launch Cargo if process lookup fails. Every Cargo test uses `-j 4`, `PATH="$HOME/.cargo/bin:$PATH"`, `CARGO_TARGET_DIR="$PWD/tmp/target.noindex"`, serial test execution and four Rayon threads. Node archive/certification/restart tests run after a fresh guard. Foundry builds/tests use four threads and repository-local ignored `out/`/`cache/` directories. The runner writes this report from metrics before deleting `tmp/` after a successful run.

The first compile failed with 11 errors: alloy does not implement `SolValue` for `u8` (it reserves `u8` for bytes), so tuples containing uint8 could not be encoded. The vault constructor/settings messages now use explicit `sol_data` tuple types and `engageBrake` a `sol!`-generated call; the ABI bytes equal Solidity's uint8. After that fix every test passed on the first run; no assertion was changed. Regenerating the fixtures reproduced `artifacts.json` byte for byte.

Commits on `codex/contracts-onchain`: the TokenVesting fix with its Foundry bounds tests; a verbatim toolbox snapshot (eastsea-toolbox 093f4f2); one commit per toolbox fix with its regression; the harness, fixtures and scripts; this report.

## Harness and bytecode

`crates/contracts-onchain` is a workspace test package depending on existing execution/crypto/state/types and ABI crates; no new external package versions were introduced. `scripts/generate-contract-fixtures.py --offline` recompiles the real `contracts/src` and the checked-in toolbox snapshot. It consumes Foundry `out/*.json` creation bytecode, runtime templates and ABIs, with pinned solc 0.8.19 for core and 0.8.24/Paris for toolbox/support. It never accesses the original toolbox during regeneration. Recursive OpenZeppelin dependencies and their MIT license are copied; `fixtures/provenance.json` pins source hashes, dependency revisions, compiler profiles and artifact digest. Audited toolbox changes apply only to this copy.

Every harness deployment uses **creation bytecode**, runs its constructor through `execute_block`, and signs the current wallet's `recommended_state_budget`; runtime templates are never substituted for constructor execution. Zero-budget deployment is rejected on proposer and validator paths with unchanged roots. Successful deployments record gas, state units, logical persisted bytes, signed budget and actual/floor fees. Normal calls and reverting calls are P-256 envelopes with re-signed budgets. Invalid signature, chain and nonce cases reject without state or receipts.

Each ordinary included transaction compares `build_block`, `execute_block`, and `execute_block_sequential`: state root, full receipts, receipt root, gas vector and persisted bytes. Reverts assert zero new slots/logs and equality of the entire state tree/code map outside sender/fee-account headers, exact nonce consumption, and exact fee debit. Positive execution and proving base fees have a dedicated settlement test. The default floor is exec=0, state=10^12 wei/unit, prove=0; reverted transactions still pay for their signed envelope/revert receipt and consumed exec/prove work. Contract value/state effects roll back.

The new-genesis context uses the actual B5 helpers: 100,000 state-unit burst, 32 units/refill height, exponential state surcharge after 50,000 debt, 512 new slots/block, 2 MiB logical transaction/receipt cap, and separate 8 MiB encoded payload/4 KiB refill. Executor-only tests cannot certify encoded BAL/proof/control payloads: the existing real-node `state_budget` and `zero_fee` suites are explicitly included by `--with-node` for those boundaries. Both passed in the guarded run (6/6 and 13/13).

Time-sensitive flows advance block context and execute empty refill blocks. Once both debts are zero, skipping the remaining empty intervals preserves their fixed point. Protocol randomness seed writes and registry registrar/genesis parameters are explicit pre-state fixtures; these tests do not produce threshold-BLS seeds. Native fee funding of a fresh sender is metered separately. NFT ownership and airdrop claimed flags are new **contract storage**, while NFT-only recipients do not gain chain account headers.

## Contract inventory and measurements

Every row includes successful paid deployment and undersized-budget refusal where its constructor is exercised. Randomness has no state-changing API. The descriptions enumerate implemented scenarios, not a proved exhaustive branch-coverage claim. Individual labels and selector assertions live in the test source; runtime records are produced under `tmp/` and consumed by the report generator. `unmeasured` means no passing real-executor evidence exists yet.

<!-- CONTRACTS-ONCHAIN-TABLE:BEGIN -->
Inventory: **45 compiled fixtures; 75 Rust test functions**. Status is verified complete Rust run.

Gas, state units, bytes and floor fee below describe successful wallet-budget deployments. Multiple constructor configurations are shown as ranges. Fits/block is a **ceiling** from execution, state, new-slot and logical receipt limits; the independent encoded-payload limit can lower it.

| Contract | Implemented cases | Chain result | Deploy exec gas | State units | Persisted bytes | Floor fee (wei) | Fits/block ceiling |
|---|---|---|---:|---:|---:|---:|---:|
| `core/AtomicSwap` | lock/claim/refund; native/ERC20, SHA-256, timeout, fees, failed payouts, callbacks | PASS (47 records) | 791243 | 3747 | 7333 | 3747000000000000 | 26 |
| `core/AtomicSwapEVM` | same complete swap lifecycle and errors as AtomicSwap | PASS (33 records) | 791255 | 3747 | 7333 | 3747000000000000 | 26 |
| `core/CommitteeRegistry` | signed attestation, registrar/caller/duplicate/cap/epoch/beacon; predeploy v2 compatibility | PASS (13 records) | 698818 | 3292 | 6477 | 3292000000000000 | 30 |
| `core/CommitteeRegistryV3` | registration, beacon, leaving/back, epochs, reserve sentinel, registrar rotation/revocation | PASS (34 records) | 868772 | 4128 | 8051 | 4128000000000000 | 24 |
| `core/EastSeaAccount` | 7702, owner/session/recovery mutations, P-256 signatures, limits, expiry, replay, revoke, F-05, ERC-1271, NFT receive | PASS (147 records) | 3829321 | 18689 | 35459 | 18689000000000000 | 5 |
| `core/EastSeaNames` | commit/reveal min/max age, register/clear/renew, transfer, records/reverse, grace, refund callbacks | PASS (93 records) | 1971055 | 9552 | 18261 | 9552000000000000 | 10 |
| `core/EastSeaVault` | signed spend/proposal/approve/cancel/execute/settings, quorum/delay/nonce, token/native callbacks | PASS (53 records) | 2276562–2323067 | 10952–11154 | 21564–21628 | 10952000000000000–11154000000000000 | 8–9 |
| `core/EastSeaVaultFactory` | predict/create, deterministic address, duplicate salt, invalid owner configuration | PASS (7 records) | 2666661 | 12979 | 24711 | 12979000000000000 | 7 |
| `core/MerkleDistributor` | sorted proofs, sponsored claim, duplicate/expired/bad proof, sweep, callbacks | PASS (22 records) | 584971–584995 | 2742 | 5870 | 2742000000000000 | 36 |
| `core/MerkleDistributorFactory` | create/atomic funding, bad amount/token/root/expiry, taxed-token refusal | PASS (11 records) | 993982 | 4777 | 9273 | 4777000000000000 | 20 |
| `core/Randomness` | no public mutation; paid deployment, unpublished/published epoch reads and explicit system seed writes | PASS (4 records) | 95569 | 323 | 888 | 323000000000000 | 309 |
| `core/ReleaseLog` | 7705 pin, permissionless publish, bounds/empty inputs, duplicate metadata, receipt commitment | PASS (13 records) | 361440 | 1632 | 3353 | 1632000000000000 | 61 |
| `core/TokenBatch` | send; shape/zero/allowance/failure/taxed/self-recipient/delta checks, atomicity | PASS (14 records) | 368349 | 1666 | 3417 | 1666000000000000 | 60 |
| `core/TokenLocker` | lock/extend/withdraw, caller/amount/token/time, taxed deposits, callbacks, retry | PASS (26 records) | 1022015 | 4882 | 9471 | 4882000000000000 | 20 |
| `core/TokenVesting` | create/claim/cancel, cliff/end/permissions, taxed deposit, callbacks, uint256-max arithmetic | PASS (37 records) | 1163223 | 5575 | 10775 | 5575000000000000 | 17 |
| `support/BrakeReferenceVault` | reference local deterministic entry brake: deficit/code/unreadable predicate, permissionless latch, entry halted, exit open | PASS (13 records) | 649918 | 3044 | 6254 | 3044000000000000 | 32 |
| `support/MarketNFT` | test instrument: ERC721 royalties/ERC165 failures and transfer failures | PASS (34 records) | 1173045 | 5573 | 10664 | 5573000000000000 | 17 |
| `support/NativeCallback` | test instrument: native/NFT receive failure and callback forwarding/reentry observation | PASS (143 records) | 822929 | 3905 | 7631 | 3905000000000000 | 25 |
| `support/SeizableToken` | test instrument: ERC20 with an open issuer seizure to reproduce a vault backing deficit | PASS (14 records) | 350204 | 1577 | 3249 | 1577000000000000 | 63 |
| `support/SignatureCheckerProbe` | test instrument: OZ SignatureChecker (Permit2-style ERC-1271) against a delegated P-256 account | PASS (4 records) | 303259 | 1346 | 2815 | 1346000000000000 | 74 |
| `support/SignedIntentBook` | test instrument: state-changing ERC-1271 consumer (OZ SignatureChecker), relayed owner-key approvals, replay record | PASS (8 records) | 404594 | 1845 | 3753 | 1845000000000000 | 54 |
| `support/TestToken` | test instrument: ERC20 fee/false-return/transfer callback observation | PASS (179 records) | 936507–956431 | 4442–4542 | 8837 | 4442000000000000–4542000000000000 | 22 |
| `toolbox/AgentVending` | order/deliver/refund, role/amount/hash/deadline/double-pay, brake, callbacks | PASS (39 records) | 723128 | 3404 | 7028 | 3404000000000000 | 29 |
| `toolbox/AllOrNothingCrowdfund` | contribute/refund/withdraw, failed/successful rounds, deadlines, brake, callbacks | PASS (41 records) | 767036 | 3622 | 7555 | 3622000000000000 | 27 |
| `toolbox/AmmFactory` | createPair, ordering/zero/same/duplicates/brake, empty-pair reuse | PASS (24 records) | 2311821 | 11268 | 21634 | 11268000000000000 | 8 |
| `toolbox/AmmPair` | mint/burn/swap/skim/sync, LP ERC20, invariant/reserve/liquidity, TWAP, taxed tokens, callbacks | PASS (19 records) | 1810789 | 8728 | 17006 | 8728000000000000 | 11 |
| `toolbox/AmmRouter` | liquidity add/remove, exact/taxed swaps, multihop, missing/empty pools, slippage/deadline | PASS (39 records) | 1639892 | 7913 | 15142 | 7913000000000000 | 12 |
| `toolbox/BondingLaunchpad` | buy/sell/graduate, fee/tax/cap/decay, poisoned/empty pairs, braked exit, callbacks | PASS (54 records) | 2784980–2785057 | 13228 | 24677 | 13228000000000000 | 7 |
| `toolbox/CommitRevealRaffle` | enter/reveal/drawWithoutSeed, block seed, early/bad/withheld seed, payout callbacks | PASS (39 records) | 788078 | 3725 | 7825 | 3725000000000000 | 26 |
| `toolbox/EastSeaNames` | commit/reveal min/max age, register/clear/renew, transfer, records/reverse, grace, refund callbacks | PASS (83 records) | 1955029 | 9474 | 18113 | 9474000000000000 | 10 |
| `toolbox/Editions1155` | create/mint/withdraw, limits, ERC1155 approvals/transfers/batches, royalties, narrowing, callbacks | PASS (46 records) | 2294913 | 11092 | 21340 | 11092000000000000 | 9 |
| `toolbox/FixedPriceMarket` | list/buy/cancel/withdraw/brake, ERC721 escrow, price/caller, royalties/credits, callbacks; ERC1155 refused | PASS (63 records) | 983875 | 4675 | 8857 | 4675000000000000 | 21 |
| `toolbox/FixedSupplyToken` | ERC20 transfer/approve/transferFrom, genuine EIP-2612 permit, expiry/nonce/signature/zero/max | PASS (26 records) | 1002091–1002175 | 4665 | 10403 | 4665000000000000 | 21 |
| `toolbox/InvoiceBook` | issue/settle/void/purge, roles/amount/memo/deadline/status, brake, payouts/callbacks | PASS (47 records) | 759449 | 3580 | 7116 | 3580000000000000 | 27 |
| `toolbox/LinearVesting` | preapproved constructor deposit, cliff/partial/full/public claim, donation conservation | PASS (14 records) | 573459 | 2510 | 6341 | 2510000000000000 | 39 |
| `toolbox/MerkleAirdrop` | native claim/sweep, sorted proof/deadline/double/max, callbacks, 128-user bucket capacity | PASS (37 records) | 497848–497872 | 2289 | 4881 | 2289000000000000 | 43 |
| `toolbox/MilestoneEscrow` | createDeal/approveMilestone/sellerWithdraw/buyerRefund, roles/status/amount, brake/callbacks | PASS (42 records) | 972098 | 4616 | 8745 | 4616000000000000 | 21 |
| `toolbox/NameGatedDrop` | name eligibility/expiry/duplicate/amount/pool, claim/sweep and callbacks | PASS (20 records) | 537584–537596 | 2489 | 5354 | 2489000000000000 | 40 |
| `toolbox/OnchainNFT` | mint/burn, ERC721 approvals/transfers/safe callbacks, traits/cap/brake, fresh-user capacity | PASS (49 records) | 2656203–2656287 | 12835 | 25021 | 12835000000000000 | 7 |
| `toolbox/Randomness` | no public mutation; paid deployment, unpublished/published epoch reads and explicit system seed writes | PASS (4 records) | 95569 | 323 | 888 | 323000000000000 | 309 |
| `toolbox/RewardDistributor` | fundRewards/stake/unstake/claim, time accrual, debt/conservation, taxed tokens/brake/callbacks | PASS (26 records) | 1087253 | 5168 | 10215 | 5168000000000000 | 19 |
| `toolbox/SimpleDAO` | propose/execute, secp signatures/quorum/replay, vote/timelock/expiry, mutable voting balance/callback | PASS (47 records) | 1065012 | 5061 | 10164 | 5061000000000000 | 19 |
| `toolbox/SimpleMultisig` | native receive/execute, secp sorted quorum/nonce/domain/replay, failed execution/callback | PASS (21 records) | 600425 | 2764 | 5926 | 2764000000000000 | 36 |
| `toolbox/SubscriptionManager` | subscribe/cancel/settleExpired/claimRevenue, dust/reserves/time/brake/callbacks, uint64/uint88 bounds | PASS (57 records) | 832023–832143 | 3942 | 7914 | 3942000000000000 | 25 |
| `toolbox/TokenTimeLock` | lockFor/release, cliff/linear/end, recipient/amount/token/brake, completed-grant reuse | PASS (30 records) | 907159 | 4292 | 8450 | 4292000000000000 | 23 |

Largest measured deployment: `core/EastSeaAccount` at 18,689 state units, 18.7% of the 100,000-unit burst (3,829,321 exec gas, 35,459 persisted bytes).

Common actions (worst measured record of each case). Burst fits/block applies to a block with the full 100,000-unit state budget; sustained/day is the 32-unit-per-height refill (2,764,800 units/day) divided by state units, an upper bound only.

| Action | Exec gas | State units | New slots | Persisted bytes | Burst fits/block (binding limit) | Sustained/day ceiling |
|---|---:|---:|---:|---:|---:|---:|
| Native transfer, existing recipient | 21000 | 16 | 0 | 487 | 1428 (exec) | 172,800 |
| Native transfer, new recipient account | 21000 | 116 | 0 | 487 | 862 (state) | 23,834 |
| ERC20 transfer to a new holder | 51538 | 125 | 1 | 779 | 512 (slots) | 22,118 |
| ERC20 approve | 46683 | 125 | 1 | 779 | 512 (slots) | 22,118 |
| NFT mint (OnchainNFT) | 103915 | 334 | 3 | 1067 | 170 (slots) | 8,277 |
| Edition mint (ERC1155) | 62246 | 131 | 1 | 971 | 481 (exec) | 21,105 |
| Airdrop claim (native) | 61340 | 125 | 1 | 779 | 489 (exec) | 22,118 |
| Merkle distributor claim (ERC20) | 87554 | 233 | 2 | 1035 | 256 (slots) | 11,866 |
| AMM router exact-input swap | 152413 | 160 | 1 | 1899 | 196 (exec) | 17,280 |
| Market list (ERC721 escrow) | 164588 | 534 | 5 | 1067 | 102 (slots) | 5,177 |
| Market buy with royalty | 118162 | 330 | 3 | 939 | 170 (slots) | 8,378 |
| Name commit | 103495 | 326 | 2 | 811 | 256 (slots) | 8,480 |
| Name register | 100395 | 236 | 2 | 1131 | 256 (slots) | 11,715 |
| Subscription subscribe | 73830 | 223 | 2 | 715 | 256 (slots) | 12,398 |
| Invoice issue | 72274 | 231 | 2 | 971 | 256 (slots) | 11,968 |
| Invoice settle | 58106 | 123 | 1 | 715 | 512 (slots) | 22,478 |
| Atomic swap native lock | 161580 | 632 | 6 | 1003 | 85 (slots) | 4,374 |
| Atomic swap token claim | 101933 | 235 | 2 | 1099 | 256 (slots) | 11,765 |

Measured capacity: `AIRDROP_CAPACITY sampled_burst=128 units=16868 slots=129 bytes=124288 state_day_upper_bound=21703`.

Measured capacity: `NFT_CAPACITY burst=170 units=56780 slots=510 bytes=181390 state_day_upper_bound=8577`.
<!-- CONTRACTS-ONCHAIN-TABLE:END -->

## A0 workflow records

`crates/contracts-onchain/src/recorder.rs` records whole user workflows (native plan A0); see `crates/contracts-onchain/RECORDER.md`. Each record follows `crates/contracts-onchain/schema/workflow-record.v1.schema.json`. The runner writes the raw JSONL to `tmp/contracts-onchain-workflows.jsonl` and summarizes it here. Cold totals include setup (delegation, owner setup) and failed attempts; warm totals exclude the setup phase. Per-day figures are upper bounds at nominal one-second heights for the workflow alone.

<!-- CONTRACTS-ONCHAIN-WORKFLOWS:BEGIN -->
7 workflow records; status: verified complete Rust run.

| Workflow | Txs (failed) | User tx sigs | Typed sigs | Relayer txs | Cold units | Warm units | Cold floor fee (DBLN) | Failure fees (DBLN) | Warm/day at 10% / 50% / 100% refill (binding) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| `brake/reference-vault-latch` | 13 (3) | 8 | 0 | 5 | 898 | 898 | 0.000898 | 0.000054 | 307 (state_refill) / 1,539 (state_refill) / 3,078 (state_refill) |
| `example/token-mint-transfer` | 3 (0) | 3 | 0 | 0 | 1926 | 349 | 0.001926 | 0.000000 | 792 (state_refill) / 3,961 (state_refill) / 7,922 (state_refill) |
| `p256-added-owner-relay/pay-fresh-recipient` | 6 (2) | 2 | 3 | 4 | 694 | 314 | 0.000694 | 0.000052 | 880 (state_refill) / 4,402 (state_refill) / 8,805 (state_refill) |
| `p256-erc1271/relayed-signed-intent` | 5 (3) | 1 | 1 | 4 | 247 | 202 | 0.000247 | 0.000072 | 1,368 (state_refill) / 6,843 (state_refill) / 13,687 (state_refill) |
| `p256-self-batch/native-new+native-existing+erc20-new-holder` | 3 (1) | 3 | 0 | 0 | 327 | 282 | 0.000327 | 0.000036 | 980 (state_refill) / 4,902 (state_refill) / 9,804 (state_refill) |
| `p256-separate-transactions/native-new+native-existing+erc20-new-holder` | 3 (0) | 3 | 0 | 0 | 257 | 257 | 0.000257 | 0.000000 | 1,075 (state_refill) / 5,378 (state_refill) / 10,757 (state_refill) |
| `schema/drift` | 1 (0) | 1 | 0 | 0 | 16 | 16 | 0.000016 | 0.000000 | 17,280 (state_refill) / 86,400 (state_refill) / 172,800 (state_refill) |
<!-- CONTRACTS-ONCHAIN-WORKFLOWS:END -->

## Capacity and protocol boundaries

The NFT capacity test sends 600 candidate mints to previously absent recipient accounts, compares the accepted burst with replay, and expects three new slots/mint: at most 170 mints before the 512-slot cap. The run measured exactly **170** mints (510 new slots, 56,780 state units) in the burst block; the slot cap, not the state budget, binds. The native airdrop test has 128 independent pre-funded claimants and genuine sorted Merkle proofs, measures the burst, then tests nine claims fitting one unit below the ten-claim cost and ten fitting after one refill. Both print measured units/bytes/slots and a paid-state daily upper bound.

A fresh one-day state budget is `100000 + 32 * 86400 = 2,864,800` units; sustained days after the burst have 2,764,800 units. Daily capacity divided by measured amortized claim/mint state cost is only an upper bound: archive bytes, BAL, execution/proving gas and changing storage costs can lower it. The independent archive burst/refill is `8 MiB + 4096 * 86400` canonical bytes before accounting for copies. The suite asserts a blocked transaction cannot prevent empty block execution and that capacity/price follow heights rather than wall time.

Actual genesis ReleaseLog bytecode exactly matches the compiled fixture. EastSeaAccount-v2 and registry-v2 instructions match current Solidity, but compiler CBOR metadata can differ; exact runtime hashes remain the pinned node artifacts. A specific test exercises the actual pinned account delegation. **CommitteeRegistryV3 is not the installed registry-v2 bytecode in this checkout.** V3 is tested as a deployed contract with genesis-layout parameters; changing the protocol predeploy to V3 requires a versioned consensus upgrade and was not done.

Legacy 7780 uses unlimited/free state, no logical archive charging and a compiled AtomicSwap deploy/lock/claim lifecycle. Existing execution tests cover the legacy receipt serialization shape; this new crate does not claim it reran those tests. EIP-7702 session/owner/guardian limits, revocation and recovery are asserted in the identity tests. F-05 is preserved explicitly: recovery does not invalidate the original top-level P-256 account key, which remains able to authorize execution/redelegation. ReleaseLog's builder bytes are append-only metadata; on-chain publish is permissionless and does not verify those signatures.

## Defects and fixes

| Defect | Location changed | Before-fix evidence | Fix and regression |
|---|---|---|---|
| Large vesting deposits overflow at intermediate times (`deposited * elapsed`), preventing legitimate claim/cancel | `contracts/src/TokenLocker.sol` | Four Foundry bounds tests all failed with Panic(0x11) (re-verified against the pre-fix source) | Exact quotient/remainder decomposition; both factors of remainder multiplication are uint64-bounded. Four bounds tests pass, including 256 fuzz runs over full uint64 schedules; Rust max-deposit quarter-claim/half-cancel regression added. |
| Subscription duration/principal casts truncate to uint64/uint88, misaccounting expiry or trapping refund reserves | vendored `SubscriptionManager.sol` | Bounds suite: 1 passed, 4 failed; overwide single payments silently accepted, cumulative/expiry limits panicked | Validate duration against remaining expiry range and principal against remaining packed contribution range before casts. All five boundary tests pass; Rust regressions assert clean rejection and unchanged funds/state. |
| Edition wallet limit/price casts truncate to uint32/uint128, freezing minting or making it free | vendored `Editions1155.sol` | Source narrowing confirmed; overwide values exercise the original acceptance paths | Reject values exceeding packed field widths. Three Foundry tests pass for overwide inputs and exact valid bounds; Rust mint/deploy/regression coverage added. |
| SimpleMultisig cannot receive ordinary native funding | vendored `SimpleMultisig.sol` | Native-funding regression failed before fix | Add payable `receive()`. Two deposit tests pass, including creator/outsider funding; Rust signed payout/reentry tests added. |
| TokenTimeLock never permits a new grant after the previous grant is fully withdrawn | vendored `TokenTimeLock.sol` | Completed-lock reuse regression failed before fix | Reject only a still-active grant (`released < amount`); preserve completed history until replacement. Three Foundry tests pass for active rejection, history and reuse; Rust renewal regression added. |
| Positive royalty with receiver=zero is credited to an unreachable account, stranding sale proceeds | vendored `FixedPriceMarket.sol` | Royalty regression failed before fix (seller under-credited) | Treat the invalid optional royalty as zero; no zero-address credit is created. Foundry regression passes; Rust escrow/credit solvency and royalty modes covered. |

No execution/fee/consensus production code was changed. The vesting fix affects new deployments and corrected compiled fixtures; it does not rewrite already deployed bytecode. All toolbox fixes are local audit forks, not an upstream release. No consensus-rule change was proposed as a silent fix.

## Verification summaries and remaining work

Final headless checks in this worktree:

```text
contracts/ forge test --offline --threads 4 --summary
13 suites: 161 passed, 0 failed, 0 skipped
(includes 4 new TokenVesting bounds regressions, fuzz runs 256)

vendored toolbox forge test --offline --threads 4 --summary
5 suites: 14 passed, 0 failed, 0 skipped
Editions 3; Subscription 5; Multisig 2; TimeLock 3; Market royalty 1

Rust real-executor suite (contracts_onchain): 66 passed, 0 failed
Node state_budget: 6 passed, 0 failed
Node zero_fee: 13 passed, 0 failed
Fixture regeneration: artifacts.json unchanged (sha256 6719d948...)
```

The suite covers the named state-changing functions and many independent revert guards, callbacks, boundaries and flows; it does **not** establish exhaustive reachable Solidity branch coverage or all Cartesian parameter combinations. Terminal uint32/uint48 timestamps, arbitrary malformed token returndata, some signature-array mismatch combinations and impossible native transfers above funded account balances remain gaps. Full payload certification is covered by the existing node suite rather than reconstructed in the executor harness. Fits/day figures are state-budget upper bounds from measured costs, not throughput promises.
