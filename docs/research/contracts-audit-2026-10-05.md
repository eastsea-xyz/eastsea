# Aether/EastSea smart-contract security and execution audit — 2026-10-05

**Question answered:** Have all smart contracts been checked to run correctly and securely? **No unconditional clearance.** All 15 deployable contracts in `contracts/src` were inventoried and their representative flows were exercised against `aether_execution::execute_block` or the identical node-pinned runtime. Five funds-conservation fuzz properties ran for 5,000 cases each. Two escrow insolvency PoCs and a permanent Merkle underfunding PoC passed. DEX, launchpad, and TokenFactory source is absent from `contracts/src`, and no contract brake exists for them; they must remain undeployed at genesis.

This is an independent source and local-execution audit of this detached worktree, not a proof that every parameter, third-party token, wallet UI, external EVM chain, or future deployed bytecode is safe. No contract source was changed. The audit-only Rust harness, Foundry PoCs, and logs are in ignored `tmp/`; the durable result is this report.

## Scope, chain rules, and method

- Source set: 11 Solidity files in `contracts/src`, containing 15 deployable contracts, plus the abstract `TokenEscrow` and `IERC20` interface. `contracts/base/AllowanceAccount.sol` and `P256Fallback.sol` are Base-chain support, outside this chain's genesis and the requested `contracts/src` inventory.
- Actual genesis: `ChainConfig::genesis_state` installs the EIP-7702 target at `0x…7702`; with `node_rewards && history_v2` it installs the read-only randomness view at `0x…7704`, and with a registrar it installs the registry at `0x…7703`, replacing it with V3 for a new rewards/history-v2 genesis (`crates/node/src/chain.rs:170-220`). Protocol upgrades can replace registry code/parameters; other listed application contracts are deployed by transactions later. Legacy 7780 retains earlier account/registry runtime variants.
- P-256 verification at precompile `0x100` is exercised by real signed registry, vault, and account-session calls in the Rust harness. SHA-256 and ordinary revm precompiles are used by the contracts. The precompile is native execution code, not a Solidity contract with owner or funds. This report does not independently audit revm or the P-256 implementation.
- New-genesis state pricing: one unit per code byte, 100 per newly occupied slot or account, and one per 32 persistent transaction/receipt bytes, at `10^12` wei per state unit (`crates/execution/src/fees.rs:37-48`, `crates/execution/src/block.rs:214-248`). Limits are 100,000 state units, 512 new slots and 2 MiB persistent bytes per block/transaction; the transaction execution-gas admission cap observed here is 16,777,216. Fees have separate execution/state/prove dimensions. The harness used chain ID 7799, state base price `10^12` wei, zero execution/prove prices, 100,000 signed state budget, and real P-256 envelopes. Costs vary with prior state, calldata, fee vector, and token implementation; values below are reproducible samples, not maximum quotes.
- Tooling: `FOUNDRY_FUZZ_RUNS=5000 forge test --root contracts` passed **133/133** existing tests. The ignored `tmp/contracts_audit_forge/test/Audit.t.sol` passed **10/10**, including five 5,000-run fuzz tests for native HTLC, delegated account, Merkle distributor, locker/vesting, and vault funds conservation, and five adversarial PoCs. `tmp/contracts_audit_harness` ran through `aether_execution::execute_block`; output is `tmp/contracts-audit-chain-flows.log`. `slither` is **not installed**, so no Slither result is claimed.

## Complete inventory and genesis verdict

“Genesis?” describes the current node's actual predeploy behavior. “Verdict” answers whether this exact contract is appropriate in a new genesis **now**; optional later-deployed tools need their own release gate. All contracts below have no proxy/admin code-upgrade or global pause unless stated. “No” for a not-yet-built product is a deployment blocker, not a claim that it is currently running.

| Contract | Purpose; callable authority | Genesis?; funds | Pause / upgrade and genesis verdict |
|---|---|---|---|
| `EastSeaAccount` | EIP-7702 target. Self-signed account calls `execute`, guardian/session/owner configuration; guardian quorum proposes delayed recovery; anyone relays signed owner/session/recovery execution. | Yes, `0x…7702`; funds reside in each delegating EOA, not the target. | Per-account delegation can be cleared or pointed to new code by the original signer; target is fixed for that genesis. **Conditional:** original-key compromise cannot be revoked by recovery (F-05), and exact runtime pin must be checked. |
| `CommitteeRegistry` | Legacy candidate registry: registrar P-256 attests `register`; registered beaconer calls `beacon`; anyone reads. | Legacy/testnet at `0x…7703`; no funds. | Committee-signed protocol migration can replace runtime/registrar; no user pause. **No** for new genesis: use V3, not legacy V1/V2. |
| `CommitteeRegistryV3` | V3 extends registry with beaconer-only leave/back notices and paid beacon fallback; system also writes free beacons. | Conditional new-genesis `0x…7703`; no funds. | Registrar can be revoked/rotated by committee-signed change; registry runtime may migrate. **Conditional:** registrar key and committee controls need operating assurance (F-06). |
| `Randomness` | Read-only epoch word from node-written storage; anyone reads. | Conditional new-genesis `0x…7704`; no funds. | No contract pause/upgrade except protocol migration. **Yes as a view**, with the published-seed warning (F-08). |
| `ReleaseLog` | Anyone appends bounded release hash/metadata and emits payload; clients validate pinned builder signatures/code hash. | Later; no funds. | Immutable; no publisher/admin/pause. **Conditional:** safe only with the client proof/signature checks; events alone are not authorization (F-07). |
| `EastSeaNames` | Anyone commits, clears expired commitments, reveals or renews; live owner changes resolver and proposes transfer; recipient accepts. | Later; burns fees/bonds immediately, no lasting escrow. | Immutable; no registrar/admin/pause. **Yes, optional**, subject to exact deployment and wallet fee display. |
| `AtomicSwap` | Anyone locks native/ERC-20; anyone reveals preimage to pay fixed recipient or triggers timed refund to fixed sender. | Later; holds native/ERC-20. | Immutable, no pause. **Yes, optional**, with wallet-enforced cross-chain finality and refund gap. |
| `AtomicSwapEVM` | Same inherited HTLC for external EVM networks. | Never this chain's genesis; holds funds where separately deployed. | Immutable, no pause. **Not an Aether genesis item**; external-chain deployment needs its own execution check. |
| `EastSeaVaultFactory` | Anyone deploys a CREATE2 vault or predicts its address. | Later; no funds. | Immutable; no pause. **Yes, optional**, after exact CREATE2 bytecode/address pin. |
| `EastSeaVault` | P-256 owners: one signature spends native coin under daily cap; threshold approves delayed withdrawals/settings; any one owner vetoes; anyone executes ready proposal or funds vault. | Created later by factory; holds native/ERC-20. | Threshold can replace owners/limits/delay, not code; no emergency global pause. **Conditional:** standard-token policy and F-03 event issue; operational key custody matters. |
| `TokenLocker` | Anyone escrows ERC-20 for beneficiary; creator/beneficiary extends; beneficiary withdraws at unlock. | Later; holds ERC-20. | Immutable; no pause. **No until F-01 fixed**; F-04 also affects on-chain/UI views. |
| `TokenVesting` | Anyone creates stream; beneficiary claims; creator may cancel only if originally authorized. | Later; holds ERC-20. | Immutable; no pause. **No until F-01 fixed**. |
| `MerkleDistributorFactory` | Anyone creates/funds CREATE2 campaign. | Later; transfers directly to child, no lasting funds. | Immutable; no pause. **No until F-02/F-03 fixed**. |
| `MerkleDistributor` | Anyone claims for a Merkle leaf; creator alone sweeps after a nonzero end time. | Child deployed later; holds ERC-20. | Immutable; perpetual campaigns have no exit. **No until F-02/F-03 fixed**. |
| `TokenBatch` | Anyone sends ERC-20 to a supplied array; direct transfers, no custody. | Later; no funds. | Immutable; no pause. **No until F-03 fixed**; split large batches under admission limits. |

`TokenEscrow` is abstract shared code used by locker and vesting (F-01); `IERC20` is an interface, neither is deployed independently. No DEX pair/router/factory, bonding-curve launchpad, or TokenFactory Solidity contract is present in this source tree, and `ChainConfig::genesis_state` does not predeploy one. Their ABI, reports, and design documents do not substitute for executable source or an invariant-based **contract brake**. **DEX/launchpad/TokenFactory verdict: NO at genesis or later until implementation, pause/brake design, adversarial tests, node-path cost measurements, and another audit exist.** A brake must stop unsafe swaps, liquidity changes, curve buys/sells and graduation before public deployment; governance, trigger conditions, and an escape path for held funds must be specified and tested.

## On-chain execution measurements

Each row is a successful transaction through the node executor unless marked as a read-only `call`. `state_units × 10^12 wei` is the state fee in this test; `new_slots` excludes slots cleared in the same transaction. Deployments are included because paid code bytes materially affect affordability. `Token.*` fixture mint/approval calls are excluded from the table but present in the log. `AtomicSwapEVM` was exercised **on Aether's EVM** to check inherited logic, not on Ethereum/BNB/Base. The legacy registry row uses the node-pinned V2 runtime; the current Solidity source's executable runtime core matches that V2 artifact. The V3/account pinned runtimes likewise match compiled executable cores, but Solidity metadata bytes (and thus full hashes) differ from this Foundry build; deploy/client pins must use the actual bytecode, never a locally rebuilt hash.

| Flow | Execution gas | State units | New slots |
|---|---:|---:|---:|
| ReleaseLog.deploy | 361440 | 1632 | 0 |
| ReleaseLog.publish | 139223 | 536 | 5 |
| EastSeaNames.deploy | 1971055 | 9552 | 0 |
| EastSeaNames.commit | 103495 | 326 | 2 |
| EastSeaNames.register | 100876 | 236 | 2 |
| EastSeaNames.renew | 45953 | 29 | 0 |
| EastSeaNames.setText | 98505 | 335 | 3 |
| AtomicSwap.deploy | 791255 | 3747 | 0 |
| AtomicSwap.lock native | 161592 | 632 | 6 |
| AtomicSwap.claim native | 93549 | 229 | 1 |
| AtomicSwap.lock then refund native | 144492 / 65749 | 532 / 123 | 5 / 1 |
| AtomicSwapEVM.deploy | 791243 | 3747 | 0 |
| AtomicSwapEVM.lock / claim native | 161580 / 68549 | 632 / 129 | 6 / 1 |
| TokenLocker.deploy | 925344 | 4406 | 0 |
| TokenLocker.lock / withdraw | 218835 / 42601 | 830 / 24 | 8 / 0 |
| TokenVesting.deploy | 1005922 | 4802 | 0 |
| TokenVesting.create / claim / cancel | 225361 / 73107 / 79975 | 836 / 124 / 123 | 8 / 1 / 1 |
| MerkleDistributorFactory.deploy | 910411 | 4366 | 0 |
| MerkleDistributorFactory.create perpetual / finite | 628232 / 611187 | 2888 / 2788 | 3 / 2 |
| MerkleDistributor.claim / sweep | 56449 / 33287 | 127 / 21 | 1 / 0 |
| TokenBatch.deploy / send one | 289935 / 58077 | 1281 / 130 | 0 / 1 |
| CommitteeRegistryV3.register / beacon | 200588 / 44132 | 728 / 23 | 7 / 0 |
| CommitteeRegistryV3.announceLeaving / announceBack | 77153 / 37829 | 223 / 23 | 2 / 0 |
| CommitteeRegistryV2.register / beacon | 200588 / 39470 | 728 / 23 | 7 / 0 |
| Randomness.randomness (`eth_call`) | 23591 | 0 | — |
| EastSeaVaultFactory.deploy / create vault | 2587144 / 2038471 | 12588 / 9943 | 0 / 4 |
| EastSeaVault.fund / spend | 21055 / 101521 | 16 / 229 | 0 / 2 |
| EastSeaVault.proposeWithdrawal / execute | 184556 / 56396 | 637 / 24 | 6 / 0 |
| EastSeaAccount.delegate and execute | 47516 | 50 | 0 |
| EastSeaAccount.setGuardian / addSession | 124602 / 231437 | 436 / 945 | 4 / 9 |
| EastSeaAccount.sessionExecute | 95923 | 132 | 1 |

Highest measured transaction was `EastSeaVaultFactory.deploy` at 12,588 state units (0.012588 AETH at this state price); no measured normal flow approached 100,000 units or 512 new slots. This does **not** include future DEX/launchpad state growth, pathological arrays, or combined same-block competition. A 513-recipient `TokenBatch.send` constructed with a 29M execution gas limit is rejected at admission by the 16,777,216 gas cap before slot accounting. A separate audit-only `StateFlood.fill(513)` at 14M gas was rejected by `check_admission_cost` with `new storage slots 513 exceed transaction limit 512`. Large batch transactions must be chunked; paid-state budgeting is a wallet requirement, not merely a fee display.

## Findings, attacks, and smallest fixes

### F-01 — High — reentrant token deposit can make locker and vesting insolvent — **CONFIRMED-by-test**

`contracts/src/TokenLocker.sol:35-41`, `:89-95`, `:207-221`. `_pull` snapshots the token balance, calls untrusted `transferFrom`, then credits the entire delta. During that call, a callback-capable token reenters the same escrow and completes a nested deposit. The nested deposit is credited to its own lock/stream; the outer call then credits **both** deposits again. In separate PoCs, each escrow held 200 token units while its records promised 300; paying the outer record left the inner claim reverting. Other users of the same token can be stranded, and a lock badge can overstate collateral. The ordinary-token fuzz tests do not catch this callback behavior.

Smallest fix: add a reentrancy guard around every entry point that can call `_pull` (both `lock` and `create`), and keep state changes before payout calls. Then test nested deposit, nested cross-function entry, and callback failure paths. Alternatively reject callback-capable tokens with an explicit token policy; a guard is the more direct accounting repair. Do not deploy either escrow until re-tested on the node path.

### F-02 — High — fee-on-transfer funding strands a perpetual Merkle campaign — **CONFIRMED-by-test**

`contracts/src/MerkleDistributor.sol:119-126`, `:74-85`, `:91-97`. The factory emits `Campaign(... total=100)` after a `transferFrom(100)` that can deliver only 99. The leaf still promises 100, so `claim` reverts; with `ends == 0`, `sweep` always reverts and the 99 received tokens are trapped indefinitely. This violates the documented “fully funded at birth” invariant.

Smallest fix: require an ERC-20 contract and measure child `balanceOf` before/after the funding call, reverting unless the increase equals `total`. If unusual tokens remain supported, publish and enforce an explicit funded-total policy that matches the Merkle root. Add a perpetual-campaign fee-token regression.

### F-03 — Medium — no-code or dishonest token can produce success events without payment — **CONFIRMED-by-test for factory and batch; read-only for vault**

`contracts/src/MerkleDistributor.sol:21-29`, `:119-126`, `:74-85`, `:140-148`; `contracts/src/EastSeaVault.sol:258-274`. `_erc20Move` treats a successful low-level call with empty return data as an ERC-20 success, including a call to an EOA. A PoC creates a campaign for an EOA token and marks its sole leaf claimed with zero token transfers. Another PoC makes `TokenBatch.send` emit `Sent` for an EOA token and zero transfers. The vault's queued ERC-20 branch uses the same empty-return convention and can emit `WithdrawalExecuted` without a token transfer if owners sign a no-code token. Malicious contracts returning true without moving balances can have the same event effect. A wallet or indexer relying on these events could display nonexistent payment; a user can also pay gas for a nonexistent asset.

Smallest fix: check `token.code.length > 0` before the call and verify recipient/escrow balance deltas where an exact amount is promised. For vault withdrawal, reject no-code token at proposal and verify actual movement at execution. Wallets must not treat `Sent`, `Claimed`, or `WithdrawalExecuted` alone as proof of an ERC-20 balance change.

### F-04 — Medium — historical lock arrays permit view denial of service — **read-only**

`contracts/src/TokenLocker.sol:84-95`, `:132-152`. Anyone can create arbitrarily many dust locks for a chosen beneficiary; withdrawn IDs remain in `_idsOf`. `lockedTotal` and `lockedUntil` always scan the whole historical array. Paid state makes the grief costly but does not bound it; eventually an on-chain consumer or RPC call can exceed gas or latency budgets, breaking a launchpad “locked until” badge even when a current lock is valid.

Smallest fix: maintain bounded active aggregates or expose paginated IDs and let the UI/indexer compute the view; never depend on a single unbounded scan for a security badge. Add a high-cardinality regression and state-cost budget.

### F-05 — Medium — delegated-account recovery cannot revoke the original signer — **read-only, architectural**

`contracts/src/EastSeaAccount.sol:18-19`, `:143-153`; `crates/execution/src/tx.rs:18-21`. A stolen original P-256 signing key can continue to submit account transactions or redelegate the EOA despite guardian recovery and added owner keys. The namespaced slot avoids ordinary storage collisions and self-only configuration prevents third-party initialization, but neither protects against original-key theft. The tested self-delegation path confirms the original signer retains execution authority.

Smallest viable containment is wallet/protocol level: clearly disclose the limit, provide immediate migration of funds to a fresh address and monitoring, and do not claim guardian recovery revokes compromised originals. Full revocation requires a different account-authority design. New delegation targets must reserve the same storage namespace or migrate state deliberately.

### F-06 — Medium — registrar compromise can seed validator candidates — **read-only**

`contracts/src/CommitteeRegistry.sol:80-101`; `crates/execution/src/registry.rs:75-113`. A single registrar P-256 key approves registrations. If stolen or abused, it can attest attacker-controlled validator/node identities; V2's per-epoch cap slows but does not prevent continued candidate seeding. Final voting selection has separate chain rules, so this is not an immediate funds-drain claim.

Smallest fix before a new genesis: operational threshold approval or bounded/monitored issuance, tested key revocation/rotation by committee upgrade, and an incident runbook. Pin the actual V3 runtime and ensure the registry predeploy key is the intended ceremony output.

### F-07 — Low — open ReleaseLog events can impersonate an approved or emergency release to event-only clients — **read-only**

`contracts/src/ReleaseLog.sol:38-46`. Anyone may publish arbitrary manifest/signature bytes and set `emergency=true`; the contract does not authenticate builders by design. An explorer or updater that treats `Published` as authorization can be fooled. The intended app flow validates the pinned code hash, proved storage, builder signatures, and delay; with that flow this is expected open-log behavior, not a bypass.

Smallest fix: retain and test client-side proof/signature gating and label unverified log entries clearly. Do not add an admin key merely to silence spam.

### F-08 — Low — randomness is readable before the target epoch — **read-only, usage constraint**

`contracts/src/Randomness.sol:4-16`. The threshold seed is published ahead of the epoch; a caller can compute or inspect the future word before joining a game or choosing a commitment. A future lottery/launchpad that treats the word as secret or unpredictable can be gamed.

Smallest fix: require commitment before seed publication and reveal after the target epoch, or use a separate unpredictability mechanism. The view contract itself works as specified.

## Per-contract threat review

| Contract | Access, reentrancy, arithmetic, MEV, signatures and state risk |
|---|---|
| `EastSeaAccount` | `onlySelf` gates owner configuration; relayed recovery/owner/session actions require P-256 signatures bound to chain, account, domain and nonce/unique session ID. ERC-7201 storage avoids ordinary delegated-EOA slot collisions; no public initializer can seize an account. Arrays are capped (8 keys/sessions, 16 payees), but arbitrary owner `execute` batches consume bounded transaction gas. Native/token daily limits use conservative UTC-day overlap accounting. Original-key redelegation remains F-05. |
| `CommitteeRegistry` | Only a registrar attestation can add a validator key; it binds operator, chain and registry address. `beacon` requires the recorded beaconer. Genesis-only params and committee migrations are the effective admin surface, with F-06 key risk. V2 limits new registrations per epoch; it holds no funds and has no oracle/AMM path. |
| `CommitteeRegistryV3` | Inherits registrar gate and lets only the recorded beaconer announce leave/back; the node's free beacon system writes share the registry state. Sentinel handling and announcement flow were exercised on the pinned V3 runtime. No public upgrade or funds path; committee migration remains privileged. |
| `Randomness` | Pure read of a node-maintained slot. It never transfers funds or accepts a write; F-08 forbids treating the word as secret. |
| `ReleaseLog` | Anyone may append, including arbitrary `emergency` data; entries cannot be edited. Manifest/signature byte caps bound storage/log grief, but approval is entirely a client-side proof/signature decision (F-07). No funds, oracle, proxy, or signed on-chain action. |
| `EastSeaNames` | Commit binds name, intended owner, salt and relayer; 60-second delay and 24-hour expiry deter reveal theft and stale commitments. Owner-only resolver edits and two-step transfer avoid direct ownership theft. Burn/refund occurs after storage effects; text records are bounded to four. Integer fees are fixed buckets; transaction-order races for an available name remain ordinary registration competition. |
| `AtomicSwap`, `AtomicSwapEVM` | Claim preimage is public, but claim/refund destinations are fixed at lock. Status changes before external payment prevent reentrant double settlement; exact ERC-20 balance deltas reject taxed transfers. Deadline and cross-chain finality are wallet responsibilities; no price oracle or signature replay path. Neither can be paused, so unsupported tokens must be screened before locking. |
| `EastSeaVaultFactory` | Public CREATE2 deployment/prediction; salt and constructor args fix address. No treasury, privileged caller or selfdestruct. Pin full init code including metadata before displaying a predicted address. |
| `EastSeaVault` | P-256 digests bind chain, vault, action and nonce/proposal ID; threshold approval and delay protect queued transfers/settings, while any one owner can veto. Proposal count and owner loops are bounded by gas/eight owners; spend increments nonce before native call and queued proposal is deleted before payout. Integer daily accounting is conservative across UTC days. A signed invalid ERC-20 address can produce F-03's false success event. No proxy/pause; threshold settings change is the admin surface. |
| `TokenLocker` | Only beneficiary withdraws; creator or beneficiary may extend, never shorten. Payout state clears before token call, but deposit callback breaks solvency (F-01); historical view arrays allow F-04. Transfer-tax tokens may make the beneficiary receive less on exit than the nominal recorded amount, as existing tests document. |
| `TokenVesting` | Beneficiary alone claims, and only the creator can cancel an explicitly cancelable stream. Vesting division rounds down until `end`, when the full balance vests; state settles before token payout. Shared deposit callback bug F-01 applies. No unilateral change to an existing stream or upgrade path. |
| `MerkleDistributorFactory` | Anyone creates a campaign; CREATE2 salt includes caller and global sequence. The token callback is untrusted and the factory fails to verify exact funding (F-02) or even token code (F-03). Factory itself does not retain funds; campaign metadata can mislead if token behavior is dishonest. |
| `MerkleDistributor` | Leaf fixes index, account and amount; anyone can sponsor claim but payment goes to leaf account, and claim state is set before token call. Merkle-proof loops are calldata/gas bounded. Only creator can sweep an ended campaign; `ends=0` has no recovery, amplifying F-02. No signature or oracle dependence. |
| `TokenBatch` | Public direct transfers with atomic revert on an explicit token failure; arrays are not length-capped, so execution/state admission rejects large batches. EOA or dishonest token can produce F-03's false `Sent` event. No custody, price oracle, or administrator. |

## Cross-cutting security review and residual gaps

- **Access and signatures:** Vault, account owner/session/recovery, and registry digests bind chain ID and contract/account address; nonces or unique proposal/session IDs limit replay. P-256 verification at `0x100` succeeded on the node path. Names uses commit–reveal with committer/relayer binding and a frozen commit age. HTLC preimage submission is intentionally public but payment addresses are fixed. No EIP-712 domain is used; these are bespoke SHA-256 domains, so external signers must reproduce exact encoding.
- **Reentrancy and funds:** HTLC sets status before payout and checks ERC-20 in/out deltas; vault settles a proposal before external payment. Escrow payout paths settle state first, but deposit paths fail F-01. Merkle claims mark claimed before transfer, and a failing transfer reverts that mark. Native and ordinary ERC-20 funds-conservation tests pass; malicious/rebasing tokens remain outside a general solvency guarantee.
- **Rounding/MEV/oracles:** Vesting's integer division leaves dust until `end`, when the full deposit becomes vested. No AMM, oracle, or bonding-curve contract exists here to test sandwich resistance, price manipulation, graduation, or LP-share conservation. Names reveal can race for a name, but a copied reveal cannot redirect ownership without the committed tuple. HTLC secret revelation permits any observer to execute a claim, but only the fixed recipient receives funds; the wallet must enforce refund timing across chains.
- **CREATE2/selfdestruct/upgrades:** Vault and distributor factory addresses include factory, salt, init-code/constructor data; duplicate deployment reverts. No reviewed application contract exposes `selfdestruct` or a proxy upgrade. The chain can replace system registry code through a signed protocol migration, and an EIP-7702 account can redelegate via its original signer. These are the material upgrade/admin surfaces. Exact runtime metadata differs between the pinned genesis bytes and this local Foundry compilation despite identical executable cores; code-hash-sensitive clients and address predictions must pin the deployed artifact, not this audit build.
- **Boundedness and events:** Names limits name/text sizes and text keys; account limits guardians, owners, sessions, and allowed recipients; release payload bytes are capped. Vault proposal loops are bounded by eight owners. TokenBatch and locker historical views remain unbounded; TokenBatch can hit the execution cap before the 512-slot limit. `ReleaseLog.Published`, `TokenBatch.Sent`, `MerkleDistributor.Campaign/Claimed`, and vault token-withdrawal events require the trust/transfer checks noted above. Paid state increases the cost of griefing but is not a semantic safety control.
- **Verification limits:** No live devnet or external-chain RPC transaction was sent. No Slither executable was available. Representative main flows ran on the local node execution engine; full adversarial state-space coverage and future DEX/launchpad security are not established. Re-run costs and code hashes against the final ceremony `network.json`, pinned binaries, fee vector, and actual deployment transaction before genesis.

## Required release gates

1. Fix F-01 in both escrows and F-02/F-03 in the token distribution/batch path; repeat the adversarial PoCs, conservation fuzz tests, and node-path measurements on the exact new runtime.
2. Keep DEX, launchpad, and TokenFactory **out of genesis**. Before any public deployment, supply source and a tested contract brake, plus pair/curve conservation, sandwich/price-manipulation, graduation, funds-exit, and paid-state cap tests.
3. Verify the exact genesis predeploy runtime hashes and ceremony registrar key; test registrar revocation, account redelegation/migration, wallet release-log verification, and affordability at the finalized fee vector.

**Short summary:** 133 existing Foundry tests and 10 audit tests passed, and representative contracts executed on Aether's node path. The audit confirmed reentrant escrow insolvency, permanently underfunded Merkle campaigns, and false ERC-20 success events. The current contract set is **not collectively safe for an unconditional genesis deployment**; DEX and launchpad have no source or brake here and must stay out.
