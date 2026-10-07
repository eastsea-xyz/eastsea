# EastSea contracts on a live chain — 2026-10-06

The founder asked: "다른 계약들 올려서 확인해봄?" Until now every contract fixture
(`crates/contracts-onchain`, 41 fixtures) had run only in-process through
`execute_block` ([contracts-onchain-2026-10-06.md](contracts-onchain-2026-10-06.md)).
This report covers the first time all of them ran on a real chain.

**Answer: yes. All 41 fixtures were deployed and driven on a real local
new-genesis chain** (4 validators over loopback TCP, protocol 3, paid state
with the B5 budget active). Every transaction was signed with P-256 by the
wallet signing code and submitted over JSON-RPC. Every receipt was read back
over RPC. The explorer and three toolbox frontends were checked in headless
Chrome.

- **Final run: 152/152 flow steps.** That covers 41 deployments, the user
  flows and 47 expected reverts. All 47 were included and failed on chain.
  46 of them carry a decoded reason. The exception is the vault factory's
  duplicate-salt CREATE2, which reverts with empty data. The explorer passed
  36/36 checks and the three toolbox frontends 14/14.
- Gas and state units match the in-process numbers. State units matched
  exactly for every contract. Exec gas matched exactly or differed by 12–120
  gas, which comes from different constructor arguments; LinearVesting
  differed by 3,186 gas.
- **The B5 budget is the practical limit.** After the 100,000-unit burst, the
  suite waited 83–100 minutes in total for the 32-units-per-block refill.
  During that time the state price was 11–52× the floor.
- **Five bugs found.**
  - Three were fixed in their own commits with regression tests: a wallet bug
    that loses user fees, a misleading refusal text, and a silent wrong
    `eth_getLogs` answer.
  - The fourth was the harness itself.
  - The fifth, queued transactions silently dropped once the B5 price
    rises, is now visible and recoverable (branch `claude/b5-stuck-tx`,
    see "Stress"): every transaction ends included, refused with a message,
    or pending/dropped with a reason.
- No consensus rule was changed. The protocol observations are under
  "Consensus-level observations".

## The two measured runs

| | Run 1 | Run 2 (this table) |
|---|---|---|
| Harness | `1cae3f4` | `70f063c` |
| Binary | pre-fix (lane HEAD at start) | with fixes `d0af1c6`, `4234a31`, `e5370a4` |
| Chain | fresh genesis, heights 5 → 5,556 | fresh genesis, heights 6 → 5,561 |
| Flow steps | 151/152 ok | **152/152 ok** |
| Wall time / waiting for B5 refill | 113 min / 100 min | 94 min / 83 min |
| State units charged | 270,479 | 270,591 |
| Explorer checks | 36/36 | 36/36 |
| Toolbox checks | 14/14 | 14/14 |
| Stress | single sender (see below) | four senders (see below) |

**Run 1's one failed step was a harness timing artifact.** "TokenVesting.claim
before cliff" did not revert because a 30 s budget wait before the create had
already carried the stream past its 20 s cliff. Run 2 uses a 300 s cliff and
reverts with `NothingToClaim()`.

**Run 1 also recorded the wallet bug (#1 below) on chain.** Two plain
`aether send` transfers were included with `success=false gas=21000`: one to
EastSeaVault and one to the batch-delegated dev4. In run 2, with the fix, the
same two sends succeed with 21,055 gas.

FAST=1 (a legacy free-state genesis, used only to debug the drivers) also ran
152/152 in 582 s with the fixed binary. Its numbers are not B5 numbers and are
not used here.

## What ran

`scripts/contracts-live.sh` (rerunnable; phases `bin deps chain registrar
flows explorer dapp stop reset bin chain stress stop`):

1. **Chain.** `aether network --chain-id 7796 --protocol 3 --history 2
   --node-rewards --dev-registrar --faucet <dev1>` assembles a new genesis.
   Chain 7796 is `0x1e64`, the id the toolbox frontends require. History v2
   plus node rewards turn on paid state and B5. Then:
   - DKG over loopback.
   - The ceremony record (`aether ceremony-record`), passed to every
     validator with `--ceremony`, the way `mainnet-rehearsal.sh` does it.
   - 4 × `aether node` on 127.0.0.1, with P2P on 8711–8714 and RPC on
     8645–8648.

   These ports are clear of the testnet's 8545, 8601–8604, 9101–9104, 18545
   and 19101.

   The faucet funds dev1 at genesis. The chain phase then runs the real
   registry flow: `aether candidate-register` with the dev registrar, success
   in block 5.
2. **Flows** (`scripts/contracts-live/deploy-flows.mjs`, `flows.mjs`). Every
   state change goes through the `aether` CLI (`deploy`, `call`, `send`,
   `batch`, `set-guardian`). The CLI is the wallet path:
   - `sign_call_with` plus `recommended_state_budget`
   - fee caps quoted from the node's `aether_status`
   - P-256 dev keys
   - submission via `aether_sendTransaction`

   For each contract the driver:
   - deploys it, with constructor arguments as the in-process fixtures build
     them;
   - runs 2–5 calls of its main user flow;
   - sends at least one call that is expected to revert on chain.

   It records submit→finalized latency, block height, exec gas, state units,
   the state fee from the receipt (`aether_getReceipt`) and the decoded revert
   reason from the receipt output. It also counts each contract's logs over
   `eth_getLogs`.

   When admission refuses a transaction because the B5 budget is spent, the
   driver waits and resubmits (every 15 s), the way a wallet would, and
   records the wait.

   Two special cases:
   - **EastSeaAccount** is an EIP-7702 target (`execute` is `onlySelf`). Its
     bare copy is deployed and shows `OnlySelf()`. Its live user flow is the
     wallet's own 7702 path against the protocol's pinned account:
     `aether batch` (delegate, then pay two recipients in one tx) and
     `aether set-guardian`.
   - **CommitteeRegistry and CommitteeRegistryV3** can only revert when
     deployed bare (`BadAttestation`), because a user deploy has no genesis
     registrar. The real registry flow is the `registrar` phase above.
3. **Explorer** (`explorer-check.mjs`). It serves `apps/explorer` locally and
   opens it in headless Chrome (playwright-core with system Chrome). It points
   the explorer at the local node with the public gateway fallback off, then
   reads the DOM text of:
   - the home page, a block, a deploy tx, a reverted tx with its decoded
     reason, a token-transfer tx with its decoded event, an account and a
     token page;
   - the deploy tx and the main call tx of the 5 most user-facing examples:
     FixedSupplyToken, OnchainNFT, FixedPriceMarket, EastSeaNames and
     AmmRouter. For each it checks status, block and the log count against
     the receipt.

   Screenshots went to `tmp/` only.
4. **Toolbox frontends** (`dapp-check.mjs`). It serves
   `/Volumes/workspace/eastsea-toolbox` read-only and loads three apps —
   `apps/token` (FixedSupplyToken), `apps/nft` (Editions1155) and
   `apps/names` (NameGatedDrop) — against the deployed contracts.
   - **The MV3 extension cannot be loaded headless.** The pages therefore get
     an EIP-1193 shim on `window.aether`. Its writes go through the same CLI
     signing path and its reads go to the node.
   - Each app connects, passes the chain gate, runs an `eth_call` view, sends
     a write and reads its receipt, and lists events from `eth_getLogs`.
   - The names app also sends a second claim that is expected to revert.
5. **Stress** (`stress.mjs`) runs on a fresh genesis, so it starts with a full
   burst:
   - 200 transfers to never-seen accounts, from dev1–dev4, in waves of 20;
   - 20 EastSeaAccount-size deploys (16,599 units each) from dev6–dev10, all at
     the same time.

   It checks that the height keeps advancing, classifies every transaction,
   and keeps the refusal text verbatim.

Resource rules were followed: one cargo command at a time, `-j 4`, guarded on
other `rustc`/`cargo` and ≥ 25 % free memory. All chain data, logs and
screenshots went under the worktree's `tmp/`, and every node was stopped by
trap or `stop`. The rules on what not to touch were also followed: the wallet
GUI was never launched, and nothing touched `~/aether-testnet`,
`~/Library/Application Support/Aether` or poc-nas.

## Per-contract results (live vs in-process)

From run 2:

| Contract | Live result (steps) | Deploy height | Deploy exec gas (live) | (in-process) | State units (live) | (in-process) | B5 wait before deploy | Median submit→final | Logs: receipts / eth_getLogs | Expected revert(s), as decoded from the receipt |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| `core/AtomicSwap` | PASS 4/4 | 22 | 791243 | 791243 | 3747 | 3747 | 0 | 1018 ms | 2 / 2 | WrongPreimage() |
| `core/AtomicSwapEVM` | PASS 4/4 | 28 | 791255 | 791255 | 3747 | 3747 | 0 | 1637 ms | 2 / 2 | AlreadySettled() |
| `core/CommitteeRegistry` | PASS 3/3 | 33 | 698818 | 698818 | 3292 | 3292 | 0 | 1014 ms | 0 / 0 | BadAttestation(); Unknown() |
| `core/CommitteeRegistryV3` | PASS 3/3 | 36 | 868772 | 868772 | 4128 | 4128 | 0 | 1012 ms | 0 / 0 | BadAttestation(); Unknown() |
| `core/EastSeaAccount` | PASS 6/6 | 39 | 3404536 | 3404536 | 16599 | 16599 | 0 | 2029 ms | 3 / 0 | OnlySelf() |
| `core/EastSeaNames` | PASS 6/6 | 51 | 1971055 | 1971055 | 9552 | 9552 | 0 | 1566 ms | 5 / 6 | CommitTooNew(3); UnknownCommitment() |
| `core/EastSeaVault` | PASS 4/4 | 133 | 2276622 | 2276562–2323067 | 10952 | 10952–11154 | 0 | 1643 ms | 1 / 1 | BadSignature() |
| `core/EastSeaVaultFactory` | PASS 4/4 | 139 | 2666661 | 2666661 | 12979 | 12979 | 0 | 1013 ms | 1 / 1 | (no data) |
| `core/MerkleDistributor` | PASS 3/3 | 352 | 585019 | 584971–584995 | 2742 | 2742 | 90 s | 1016 ms | 2 / 1 | AlreadyClaimed(0) |
| `core/MerkleDistributorFactory` | PASS 3/3 | 506 | 993982 | 993982 | 4777 | 4777 | 150 s | 1018 ms | 2 / 1 | Error(allowance) |
| `core/Randomness` | PASS 2/2 | 615 | 95569 | 95569 | 323 | 323 | 15 s | 1004 ms | 0 / 0 | – |
| `core/ReleaseLog` | PASS 3/3 | 677 | 361440 | 361440 | 1632 | 1632 | 45 s | 1014 ms | 1 / 1 | EmptyManifest() |
| `core/TokenBatch` | PASS 3/3 | 740 | 368349 | 368349 | 1666 | 1666 | 45 s | 1021 ms | 3 / 1 | LengthMismatch() |
| `core/TokenLocker` | PASS 5/5 | 909 | 1022015 | 1022015 | 4882 | 4882 | 150 s | 2028 ms | 4 / 2 | StillLocked(1791306363); BadLock() |
| `core/TokenVesting` | PASS 5/5 | 1114 | 1163223 | 1163223 | 5575 | 5575 | 165 s | 1022 ms | 4 / 2 | NothingToClaim(); BadStream() |
| `support/MarketNFT` | PASS 1/1 | 12 | 1173045 | 1173045 | 5573 | 5573 | 0 | 2022 ms | 0 / 4 | – |
| `support/NativeCallback` | PASS 2/2 | 20 | 822929 | 822929 | 3905 | 3905 | 0 | 1071 ms | 0 / 0 | – |
| `support/TestToken` | PASS 4/4 | 10 | 936507 | 936507–956431 | 4442 | 4442–4542 | 0 | 2028 ms | 1 / 36 | Error(balance) |
| `toolbox/AgentVending` | PASS 5/5 | 1254 | 723176 | 723128 | 3404 | 3404 | 105 s | 2026 ms | 2 / 2 | WrongPrice(100000000000000, 1000000000000000); NotAgent(0xF436397Bb0D4826765f917d9FDd9805459b6B3ff) |
| `toolbox/AllOrNothingCrowdfund` | PASS 5/5 | 1369 | 767072 | 767036 | 3622 | 3622 | 105 s | 2028 ms | 3 / 3 | RefundNotAllowed(1500000000000000, 2000000000000000, 1791306826) |
| `toolbox/AmmFactory` | PASS 3/3 | 1726 | 2311821 | 2311821 | 11268 | 11268 | 300 s | 1023 ms | 1 / 1 | IdenticalTokens(0x9184bEc21F11d02364598e0Ca41cd6d7E96AE58e) |
| `toolbox/AmmPair` | PASS 3/3 | 2012 | 1810777 | 1810789 | 8728 | 8728 | 285 s | 2023 ms | 1 / 1 | InsufficientLiquidityMinted() |
| `toolbox/AmmRouter` | PASS 5/5 | 2259 | 1639880 | 1639892 | 7913 | 7913 | 240 s | 3079 ms | 14 / 1 | InvalidPath() |
| `toolbox/BondingLaunchpad` | PASS 4/4 | 2992 | 2785177 | 2784980–2785057 | 13228 | 13228 | 436 s | 2615 ms | 9 / 2 | InsufficientInput() |
| `toolbox/CommitRevealRaffle` | PASS 6/6 | 3128 | 788102 | 788078 | 3725 | 3725 | 135 s | 1021 ms | 2 / 3 | WrongPrice(100000000000000, 1000000000000000); RevealTooEarly(1791308597, 1791308616); InvalidSeed(0xbbeecdc05c0a866a8dabc849a37ed68cbd51ff7f6f0be478f24a682822f71df2, 0x053b845a20d74204012270866d59cefbac3e1e1f0a2bf7079ec986efba0e8aad) |
| `toolbox/EastSeaNames` | PASS 4/4 | 54 | 1955029 | 1955029 | 9474 | 9474 | 0 | 1111 ms | 5 / 6 | – |
| `toolbox/Editions1155` | PASS 4/4 | 3486 | 2294913 | 2294913 | 11092 | 11092 | 330 s | 2025 ms | 2 / 3 | UnknownEdition(99); WalletCapReached(1, 1) |
| `toolbox/FixedPriceMarket` | PASS 3/3 | 3645 | 983875 | 983875 | 4675 | 4675 | 135 s | 2022 ms | 1 / 3 | UnknownListing(1) |
| `toolbox/FixedSupplyToken` | PASS 3/3 | 3822 | 1002247 | 1002091–1002175 | 4665 | 4665 | 135 s | 1011 ms | 2 / 4 | ERC20InsufficientAllowance(0xF436397Bb0D4826765f917d9FDd9805459b6B3ff, 0, 1) |
| `toolbox/InvoiceBook` | PASS 3/3 | 3954 | 759449 | 759449 | 3580 | 3580 | 135 s | 2020 ms | 1 / 2 | NotPayee(0xF436397Bb0D4826765f917d9FDd9805459b6B3ff) |
| `toolbox/LinearVesting` | PASS 3/3 | 4053 | 576645 | 573459 | 2510 | 2510 | 75 s | 1015 ms | 2 / 1 | ZeroAmount() |
| `toolbox/MerkleAirdrop` | PASS 3/3 | 4129 | 497860 | 497848–497872 | 2289 | 2289 | 75 s | 1018 ms | 1 / 1 | AlreadyClaimed() |
| `toolbox/MilestoneEscrow` | PASS 3/3 | 4280 | 972098 | 972098 | 4616 | 4616 | 150 s | 1560 ms | 1 / 3 | NothingToWithdraw() |
| `toolbox/NameGatedDrop` | PASS 4/4 | 5538 | 537656 | 537584–537596 | 2489 | 2489 | 30 s | 1022 ms | 1 / 1 | NoPrimaryName(0xF436397Bb0D4826765f917d9FDd9805459b6B3ff); AlreadyClaimed() |
| `toolbox/OnchainNFT` | PASS 3/3 | 4705 | 2656263 | 2656203–2656287 | 12835 | 12835 | 406 s | 1634 ms | 1 / 3 | NotCreator(0x0e6dAd66906734568D7BADb0749AB9c15768d13e) |
| `toolbox/Randomness` | PASS 2/2 | 631 | 95569 | 95569 | 323 | 323 | 15 s | 1042 ms | 0 / 0 | – |
| `toolbox/RewardDistributor` | PASS 3/3 | 4875 | 1087253 | 1087253 | 5168 | 5168 | 165 s | 1020 ms | 2 / 3 | InsufficientStake(10000000000000000000000000, 9000000000000000000) |
| `toolbox/SimpleDAO` | PASS 5/5 | 5078 | 1065060 | 1065012 | 5061 | 5061 | 150 s | 1021 ms | 2 / 2 | NotExecutableYet(1791310595, 1791310654); AlreadyExecuted(1) |
| `toolbox/SimpleMultisig` | PASS 3/3 | 5182 | 600425 | 600425 | 2764 | 2764 | 30 s | 1007 ms | 1 / 1 | AlreadyExecuted() |
| `toolbox/SubscriptionManager` | PASS 4/4 | 5306 | 832023 | 832023–832143 | 3942 | 3942 | 120 s | 1015 ms | 2 / 3 | PaymentTooSmall(1, 2) |
| `toolbox/TokenTimeLock` | PASS 4/4 | 5447 | 907159 | 907159 | 4292 | 4292 | 120 s | 1018 ms | 4 / 2 | NothingToRelease() |

Steps: 150/150 ok across the 41 fixtures; 2 other steps (fund dev2, fund dev3).

How to read the table:

- **Live result** counts this contract's steps that passed: the deploy, its
  user-flow calls and its expected reverts. An expected revert passes only if
  it was included with `success=false`.
- **In-process** columns are the successful paid deployment from the
  in-process report.
- **B5 wait** is how long the wallet path waited for refill before the deploy
  was admitted.
- **Logs** shows two counts. The first is the logs on the receipts this step
  list recorded (a multi-call step records its last tx). The second is the
  contract's total over `eth_getLogs`, walked in 2,000-block windows.
- **Expected revert(s)** is the reason decoded from the receipt's output with
  the fixtures' error ABIs. That is what a wallet or the explorer can show.

## Discrepancies vs the in-process run

- **State units: none.** Every deployment charged exactly the in-process state
  units. Where the in-process report gives a range (TestToken, EastSeaVault,
  OnchainNFT…), the live value is inside it.
- **Exec gas: small, explained.** Some contracts differ by 12–120 gas:
  AgentVending, AllOrNothingCrowdfund, AmmPair, AmmRouter, BondingLaunchpad,
  CommitRevealRaffle, MerkleDistributor, NameGatedDrop, SimpleDAO and
  FixedSupplyToken. These constructors take addresses and amounts, and the
  live run passes different ones (real dev addresses, live token addresses).
  Calldata costs 16 gas per non-zero byte and 4 per zero byte, so the
  intrinsic cost moves.
  - LinearVesting differs by 3,186 gas (576,645 vs 573,459), the one larger
    gap. Its constructor pulls the deposit with `transferFrom`, so the cost
    depends on the token's balance and allowance slots, which the two runs
    set up differently. This was not investigated further. State units match
    (2,510).
- **Common actions match where the scenario is the same.**

  | Action | Exec gas, live | Exec gas, in-process | State units, live | State units, in-process |
  |---|---:|---:|---:|---:|
  | Native transfer to a new account | 21,000 | 21,000 | 116 | 116 |
  | Name commit | 103,495 | 103,495 | 326 | 326 |
  | Edition mint | 62,246 | 62,246 | 131 | 131 |
  | Invoice settle | 58,106 | 58,106 | 123 | 123 |
  | Atomic-swap native lock | 161,652 | 161,580 | 632 | 632 |

  Name register used 106,080 gas live vs 100,395. The live name and value
  differ, and the overpayment refund path runs. State units match (236).

  The live router swap (189,515 gas / 360 units) is not the in-process case
  (152,413 / 160). Live, the output goes to a new holder of the second token
  (a new slot) and the path is two tokens with fresh reserves.
- **Fees are not the floor once the burst is spent.** In-process "floor fee"
  is state units × 10^12 wei. Live, the first nine deployments (until about
  50,000 units of debt) paid exactly that. From then on the exponential
  surcharge applied:
  - toolbox/EastSeaNames paid 2.5× the floor and EastSeaVault 4.8×.
  - Every deployment after the 12th paid 11.5–52× the floor (e.g.
    MerkleDistributor about 0.117 coin for 2,742 units).
  - The price stayed there because the suite kept the debt near 100,000 for
    the whole run.

  This is the B5 design working as specified, not a discrepancy in metering.
  But nothing in-process shows what a user actually pays during a busy hour.
- **Latency.** In-process has no latency. Across 148 wallet-path
  transactions, submit→finalized took:
  - run 2: minimum 1.00 s (one 1 s block), median 1.05 s, p90 2.0 s,
    maximum 3.1 s;
  - run 1: median 2.3 s, maximum 5.3 s.

  These times exclude B5 waits, which happen before submission.
- **`eth_getLogs` window.**
  - Run 1 counted logs with one whole-run query and found events for 18/43
    contracts.
  - Run 2 walks 2,000-block windows and finds events for 37/43. The other 6
    emit nothing on these paths: the registries, NativeCallback and both
    Randomness copies.
  - The clamp, recorded by run 2: TestToken has **36** logs across the run,
    but one request for blocks 1–5,561 returned **11** with no error. The node
    scans only the newest 2,000 blocks (see "Consensus-level observations").

## Bugs found and fixed (non-consensus, each with a regression test)

| # | Severity | Bug | Fix (commit) |
|---|---|---|---|
| 1 | **High** (users lose fees) | **Every wallet path signs exactly 21,000 exec gas for a plain transfer, whatever the recipient.** This covers the app (`crates/ffi` `prepare_transfer`), the extension (wasm builder default for `data: 0x`) and `aether send`. A send to a contract with a `receive()` (EastSeaVault, the toolbox multisig, airdrop, DAO and drop pools) is included, fails out of gas, and still pays its fee. Worse: once a user turns on **batch payments or recovery guardians**, the wallet delegates their account to EastSeaAccount (EIP-7702). From then on **every ordinary transfer to that user fails**. Live evidence (run 1, pre-fix binary): `aether send` to EastSeaVault and to the batch-delegated dev4 were both `success=false gas=21000`. | `4234a31`: `plain_transfer_gas_limit` (execution) signs 100,000 when `eth_getCode` shows code, else 21,000. Applied in the app (quote and signed limit agree, so the M1 shown-fee check holds), the extension and the CLI. Tests: execution (21,000 to code fails, the sized limit succeeds, a 7702 designator counts as code), ffi (quote and envelope size), extension (`withTransferGas`). In run 2 both sends succeed. |
| 2 | Medium (wallet-facing text) | When the B5 budget is spent, admission refused with **"transaction exceeds block gas limit"**. These are the same words as for a transaction that can never fit, so users and wallets cannot tell "too big" from "busy, retry in a few minutes". | `d0af1c6`: admission now says `state budget: this transaction needs N state units and M are available right now; the budget refills 32 per block, retry in about K blocks`, or that it exceeds any block. Only the text changed; the fit rule is untouched. Seen verbatim in the stress run (below). Test in `crates/execution/tests/state_growth.rs`. |
| 3 | Medium (silent wrong answer) | A malformed `eth_getLogs` block tag fell back to `latest`. The toolbox frontends ask `fromBlock: head − 5000`. On a chain younger than 5,000 blocks (the first 83 minutes of any new genesis, mainnet included) that is `"0x-e7f"`, so the node silently scanned **only the newest block**. The frontends then showed "no events in the last 5,000 blocks". | `e5370a4`: a malformed tag is `-32602` on the node and at the public gate. The standard tags and hex quantities are unchanged. Test in `rpc::public_read_tests`. The toolbox itself still needs to clamp `fromBlock` at 0 and ask ≤ 2,000 blocks; that repo was not written to. |
| 5 | **High** | Queued transactions whose state fee cap falls below the rising B5 price are never includable. They are held until the 10-minute mempool TTL and then dropped without any error. 78 of 200 accepted transfers in the stress burst never ran. | `claude/b5-stuck-tx`, no consensus change: 2x state-cap headroom in the app and CLI (the shown maximum prices the signed cap), mempool tombstones with the reason, `aether_getReceipt` `pending`/`dropped` status, wallet and agent reasons with a same-nonce resend, per-sender submit order. Rerun: 0 silent losses (see "Stress"). |
| 4 | Low (harness) | The first harness commit had never run on a node. Problems found:<br>• nodes refused to start (no ceremony record);<br>• dev addresses used the secp256k1 rule instead of `keccak(1 ‖ compressed P-256)`;<br>• constructor arguments lost 4 bytes;<br>• `@noble/hashes` hashed ABI hex *strings* as UTF-8;<br>• vault digests were hashed twice;<br>• DAO and multisig hashes were double-prefixed;<br>• about a dozen ABI and argument mistakes;<br>• `readFile` misuse;<br>• regexes that did not match the explorer DOM. | `1cae3f4`, `70f063c`. FAST=1 (legacy free-state genesis) exists only to debug drivers in about 10 minutes. Its numbers are never used here. |

Commits on `glm/contracts-live`, after the previous coder's `6ef7ae3`, in
order:

1. `1cae3f4` harness fixes
2. `d0af1c6` admission text
3. `4234a31` wallet transfer gas
4. `e5370a4` getLogs block tags
5. `70f063c` harness after run 1
6. this report, together with the table generator's overlap fix

## Stress: B5 under a burst

The burst is 200 transfers to never-seen accounts from dev1–dev4 (50 each,
submitted in waves of 20 within about 0.4 s) and 20 EastSeaAccount deploys
from dev6–dev10, on a fresh genesis. The drain window is the 10-minute
mempool TTL plus a minute, and the chain is watched for a further 20 s.
`stress.mjs` polls `aether_getReceipt` for every hash until it is included
or dropped, and fails the run if any hash ends with no answer (a silent loss)
or any refusal has no text.

The same harness ran twice on 2026-10-07: once with the pre-fix binary
(branch base `fa78208`), where the new silent-loss check fails as it should,
and once with the fix. Round 2 (`claude/b5-stuck-tx-2`, after the red-team
review `b5-stuck-review-2026-10-07.md`) reran it with a follower node of the
same chain, so the burst is also read the way a remote wallet reads.

| What | Run 2 (2026-10-06) | Pre-fix binary, new harness | With the fix (`claude/b5-stuck-tx`) | Round 2 (`claude/b5-stuck-tx-2`) |
|---|---|---|---|---|
| Block production | Never stopped (max stall 0 s), height 3 → 616 | Never stopped (max stall 0 s), height 5 → 706 | Never stopped (max stall 3 s at 2 s sampling), height 5 → 578 | Never stopped (max stall 6.1 s at 2 s sampling), height 5 → 272 |
| Included | 127: 122 transfers + 5 deploys, 97,147 state units, one block | 57: 52 transfers + 5 deploys, 99,477 units (blocks 19–20) | 57: 52 transfers + 5 deploys, 99,477 units, one block (22), 2.6–2.7 s after submission | 55: 50 transfers + 5 deploys, one block (16) |
| Refused at admission, with text | 15 deploys | 13 deploys | 15 deploys (`needs 18689 state units and 75 are available right now; the budget refills 32 per block, retry in about 582 blocks`) | 15 deploys (`needs 18689 state units and 115 are available right now; … retry in about 581 blocks`) |
| Dropped with a reason | — | 0 | **148 transfers, all `state_price_above_cap`** (cap 2,000,000,000,000, price 13,242,614,489,368 when the TTL expired them) | **150 transfers, all `state_price_above_cap`** (cap 2,000,000,000,000, price 28,234,933,841,948–28,307,307,871,816 when the TTL expired them) |
| Accepted, never included, no reason (silent) | **78 transfers** | **150** (148 transfers + 2 deploys) → `STRESS FAIL` | **0** | **0** |
| Mempool after the window | empty | empty | empty | empty |
| Follower's answer for the 150 the validator held (pending, then dropped) | — | — | — | `null` for all 150, both after submission and after the drain |
| The wallet's routed read (admitting validator → follower → validator) for those 150 | — | — | — | answered all 150 both times (`routedNull` 0; the run fails otherwise) |

**Round 2's follower check.** The follower (`aether follow --from-rpc`,
port 8649) re-executes every finalized block. It returned all 55 receipts
once they were included, and `null` for every hash still waiting in or
dropped from the validator's mempool. A remote wallet that trusted the
follower's `null` would have marked those 150 as failed or unknown (review
finding 2). The ffi now asks the validator that admitted the send first,
and falls back to a validator when a follower has no record. The stress
driver replays that order over JSON-RPC, and none of the 150 came back
empty. The same order runs over real QUIC in
`crates/ffi/tests/wallet_pending_route.rs`. The devnet validators run
offline over TCP and serve no wallet QUIC endpoint, so the FFI itself was
not pointed at this devnet.

**Why fewer transfers landed.** EastSeaAccount grew from 15,546 to 17,513
bytes on lead-merge after run 2 (ERC-1271, `ff6b31a`), so each deploy now
costs 18,689 state units. The five deploys that fit took 93,445 of the
100,000-unit burst, leaving room for 52 transfers of 116 units. The rest
met a state price of about 52x the floor after the burst block (debt
~99,500 units), still 13x when the TTL expired them. The 2x headroom does not reach that: the price falls to 2x
the floor only after about 1,200 one-second blocks
(`fees::blocks_until_state_price_at_most`), past the 10-minute TTL. So they
were still dropped, but every one with its reason, which the wallet turns
into "네트워크가 붐벼 수수료가 이 거래에 허용한 최대치보다 올라 처리되지
않았어요 (아직 체인에 기록되지 않음). 새 가격으로 다시 보낼 수 있어요." Since
round 2 this text no longer claims permanent non-payment, because a drop is
one node's observation.

Run 1 (single sender, pre-fix binary) instead showed these behaviours:

- **The per-sender pending cap.** Transfers 65 onwards were refused with
  `rejected: sender has 64 pending transactions`.
- **The old wording.** Deploys were refused with `transaction exceeds block
  gas limit`, which is bug #2.
- **A dropped transaction behind a nonce gap.** Because waves race each other,
  nonce 68 was refused while nonce 69 was accepted. Nonce 69 then sat behind
  the gap until the 10-minute mempool TTL dropped it. Such a drop is now
  recorded as `nonce_gap {expected: 68}`, and the wallet and agent submit
  path no longer sends N+1 after N was refused (see below).

### Bug #5 (High, now visible and recoverable): queued transactions were silently dropped when the B5 price rose

**What happened (run 2).** Wallets signed the state fee cap at the *current*
base price, with no headroom:

- the CLI `fee_caps` copies `base_fee.state`;
- the app's `fee_caps` reserves the floor `STATE_UNIT_PRICE`.

After the burst block, the debt is about 97,000 units, so the price is
e^((97,000 − 50,000) / 12,500) ≈ 43× the floor. Every transaction still in the
mempool now has `max_fee.state` below the price, so every block's builder skips
it (`check_budget`: "state fee cap below the state base price").

The refill brings the price down only slowly. After 600 blocks the debt is
still about 78,000, which is about 9× the floor. So the transactions stay
unincludable until the 10-minute `MEMPOOL_TTL` drops them. They arrived at
18:40:23, the window closed at 18:50:25, and the mempool was empty at
18:50:45.

**What the user saw.** The wallet showed a transaction hash. The transaction
never ran, and no error was ever returned.

**What changed (`claude/b5-stuck-tx`; no consensus rule, `fees.rs`
pricing or state root changed).**

1. **Headroom.** The app (`crates/ffi` `fee_caps`) and the CLI sign
   `max_fee.state = 2 × max(price, STATE_UNIT_PRICE)`
   (`fees::signed_state_cap`), like the exec cap. The charge is still the
   actual price. The send sheet's maximum prices every state unit at that
   signed cap, so the maximum shown is the maximum signed (pre-audit 7 M1).
2. **Tombstones.** When a transaction leaves a node's mempool without a
   block, the node remembers `hash → reason` (4,096 entries, LRU, memory
   only): `state_price_above_cap {cap, price}`, `nonce_gap {expected}`,
   `expired`, `replaced`, `evicted`, `fee_cap_below_base`, `unaffordable`.
   The rule that drops it is unchanged.
3. **Pending visibility.** `aether_getReceipt` keeps `{height, receipt}`
   and `{pending: true}`, and adds `status: "pending"` with `waiting` (for a
   state cap: the price, the cap and the estimated blocks until the refill
   brings the price to the cap) or `status: "dropped"` with `reason` and
   `resendable`. An unknown hash is still `null`.
4. **Wallet and agent.** The activity row says in plain Korean what a
   pending send waits for, or why it was dropped, for the whole TTL. A drop
   that a fresh fee can fix offers "새 가격으로 다시 보내기": the normal send
   sheet, filled in, signed at the same nonce. `aether-agent` returns the
   same `status`, `reason`, `why` and `why_ko`.
5. **Submit order.** The ffi submits one transaction at a time per process
   and refuses one whose nonce sits above a refused or dropped one, so
   nonce N+1 is never queued behind a gap.

**Round 2 (`claude/b5-stuck-tx-2`): the six red-team findings.** The review
(`docs/research/b5-stuck-review-2026-10-07.md`, which has a "Round 2
resolution" section) found no consensus change or double-spend, but six
gaps. They are fixed as follows, still with no consensus change:

1. **The shown maximum is the signed maximum.** The quote is now the
   prepared envelope's own maximum (`draft_fees` + `tx::signed_fee_maximum`):
   exec cap × gas, state cap × state budget, prove cap × prove budget. The
   M1 check compares what was shown with that envelope. Transfers, the
   resend sheet and batches (`batch_quote`) all take this path.
2. **Pending status follows the admitting node.** The wallet remembers
   which validator admitted each send and asks it first. If a follower
   returns `null`, it falls back to a validator. A follower's `null` is
   never final. The table above shows the case.
3. **Contiguous queue.** The next nonce is the end of the unbroken run of
   still-pending sends from the chain nonce. When N drops while N+1 still
   waits, N+2 is refused.
4. **Receipt cost.** A sender → pending-nonces index means one receipt read
   costs that sender's ≤ 64 entries under the chain lock, and the reason is
   computed after the lock is released.
5. **A drop is not final.** `dropped` and `unknown` stay open and are worded
   "처리되지 않았어요 (아직 체인에 기록되지 않음)". The wallet's row state is
   "Not on chain yet"; the callback says `status=not_included`; the agent's
   history keeps the payment pending. Only a receipt, or the nonce used by
   another transaction (`replaced`), settles a transaction.
6. **CLI zero price.** The CLI signs a zero state cap only on the legacy
   chain 7780. Elsewhere a 0 or missing price takes the floor, and a
   malformed price is refused.

**Reproduce:** `scripts/contracts-live.sh reset bin chain stress stop`
(a follower is started for the stress phase; `NO_FOLLOWER=1` skips it).

## Wallet-facing findings that are not code bugs here

- **Toolbox frontends show "전송됨: 0x…" (sent) for a transaction that
  reverted.** The names app's second claim showed the same green "sent" line.
  The frontends never read the receipt; the shim check did, and found
  `success=false`. Fix belongs in eastsea-toolbox: poll `aether_getReceipt`
  and show failure.
- **The toolbox token app labels ERC-20 amounts "AETH"** (`uint-ether`
  decoding). LiveCoin's supply showed as "1000 AETH". The coin is also no
  longer called AETH. Fix belongs in eastsea-toolbox.
- **The extension advertises read methods that the node does not implement.**
  The node answers `method not found` for `eth_estimateGas`,
  `eth_getStorageAt`, `eth_gasPrice` and `net_version`, all of which
  `apps/extension` READ_METHODS forwards. The same is true of
  `eth_getTransactionReceipt`, `eth_getTransactionByHash`,
  `eth_getBlockByNumber` and `eth_sendRawTransaction`. A standard
  ethers/viem dapp that estimates gas or waits for a receipt fails on
  EastSea. The toolbox avoids this by never estimating and never reading
  receipts. This is feature work, not a fix for this lane.
- **One sender cannot burst more than 64 pending transactions**
  (`MAX_PER_SENDER`). In the first stress run, with one sender, the 65th
  submission was refused while a later nonce was accepted. That later
  transaction then sat behind the gap until the 10-minute mempool TTL dropped
  it, so the CLI had printed a hash for a transaction that never ran. This is
  correct node behaviour, but a wallet that submits concurrently must not
  leave nonce gaps.

## Consensus-level observations (reported, not changed)

- **Refill speed is the binding constraint for real contract use.** The 41
  deployments plus flows charged about 270,000 state units. The burst covers
  100,000 and the refill adds 32 units per height. At the new-genesis minimum
  of 1 s per block that is 115,200 units per hour.
  - The full suite took 94–113 minutes, of which 83–100 minutes was budget
    waiting.
  - One EastSeaAccount-size deployment (16,599 units) arriving after a busy
    period waits about 8.6 minutes, and admission refuses it outright rather
    than queueing it.
  - While the debt stays above 50,000, every state unit costs 11–52× the
    floor.

  That is the specified B5 economics (`fees.rs`: `MAX_STATE_UNITS_PER_BLOCK`
  100,000, `STATE_UNITS_PER_BLOCK` 32, `STATE_PRICE_FREE_BURST` 50,000). They
  were not changed. The founder should know that a launch day with a few dozen
  contract deployers will feel this immediately.
- **The devnet's state budget was not raised.** No dev-only consensus flag was
  added. The full suite was simply run at the real parameters.
- **The private `eth_getLogs` handler clamps any range to the newest 2,000
  blocks without an error.** The public gateway refuses the same request with
  a clear error. The explorer is written for the clamp, and the toolbox
  frontends assume a 5,000-block window. It is not consensus, but it changes
  what dapps see; it is left as a decision for the lead.

## What could not be tested

- **The browser extension itself.** An MV3 extension cannot be loaded in
  headless Chrome. The dapp checks use an EIP-1193 shim that signs through
  the same CLI path. The extension's own approval UI and its wasm builder were
  not exercised live. Its new `withTransferGas` has unit tests only.
- **The wallet app GUI** was never launched (hard rule). The ffi fix has unit
  tests; the app's send sheet was not driven.
- **Proving.** No node ran `AETHER_PROVE`. The prover sidecar was the lead
  lane's staging, used for verification only. Proof gas appears in the
  receipts, but no proofs were produced.
- **CommitteeRegistry and V3 as user deployments** can only revert. Their
  real flow is the predeploy, exercised through `aether candidate-register`.
  V3 is not the installed predeploy.
- **Randomness** has no state-changing API. Only deployment and
  unpublished-epoch reads ran. Threshold-BLS seeds were not produced.
- **EastSeaAccount owner, session and recovery mutations beyond
  `set-guardian`** were not exercised live: no session keys, no recovery
  execution. Those need signatures the CLI does not produce.
- **Time-dependent paths** were exercised only with short windows (seconds
  to minutes): Crowdfund deadline, raffle reveal, DAO timelock, TokenTimeLock
  and vesting cliffs. Grace periods and expiries measured in days were not.

## Log excerpt (run 2)

`tmp/` was deleted after the run, as the lane rules require. These lines are
kept from `scripts/contracts-live.sh`'s output.

```text
== registrar   finalized in block 5  success=true  gas=0  prove_gas=0
18:24:35 eth_getLogs whole-range ask for support/TestToken: {"blocks":5561,"windowed":36,"wholeRange":11}
18:24:35 eth_getLogs: 37/43 deployed contracts emitted events
18:24:35 flows done in 5660s — 152/152 steps ok
18:24:58 explorer → tmp/live/explorer.json (36/36 checks)
18:25:13 dapp → tmp/live/dapp.json (14/14 checks)
18:40:25 deploys: 5/20 got a tx hash
18:50:45 verdict: {"blocksNeverStopped":true,"maxStallSec":0,"heightBefore":3,"heightAfter":616,
         "submitted":220,"withHash":205,"receipts":127,
         "counts":{"included":127,"never-included":78,"client-refused":15},"drainSeconds":620}
         rejected: state budget: this transaction needs 16599 state units and 2341 are available
         right now; the budget refills 32 per block, retry in about 446 blocks
```

Rerun everything with `scripts/contracts-live.sh` (about 2 h). Single phases
can be run with `KEEP=1 scripts/contracts-live.sh bin deps chain registrar`
followed by `scripts/contracts-live.sh flows explorer dapp stop`. Print the
table with `node scripts/contracts-live/report-table.mjs`.
