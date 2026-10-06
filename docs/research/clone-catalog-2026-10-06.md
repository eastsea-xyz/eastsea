# Clone catalog: popular Ethereum and Solana contracts on EastSea

2026-10-06 · research only, no code · target repo `/Volumes/workspace/eastsea-toolbox` (MIT, 17 examples)

**Goal (founder, 10-06):** prove that popular Ethereum and Solana contracts run on EastSea.
- **Ethereum:** deploy the original contracts unmodified wherever the licence allows. Bytecode built from the same source with the same compiler settings as mainnet is the strongest proof of EVM equivalence.
- **Solana:** show that the same user-facing behaviour works in Solidity.

**Publishing:** everything is public. We publish code plus test, evaluation and benchmark results. Anyone who deploys or uses the code does so at their own risk.

---

## 0. Decisions and blockers (read first)

| # | Finding (verified in code unless marked) | Effect on the proof | Decision needed |
|---|---|---|---|
| **B1** | `EastSeaAccount` (the 7702 delegate, `contracts/src/EastSeaAccount.sol`) has **no ERC-1271 `isValidSignature`**. Launch plan E7 (`docs/design/12-launch-plan.md`) lists it but it is not built. A 7702 account has code, so Permit2, Seaport, Safe and 1inch try ERC-1271 for it, and that call reverts. | Every off-chain-signature flow fails for real EastSea users: Permit2 signature transfers, Seaport off-chain orders, 1inch orders, CoW off-chain orders, Safe contract signatures, OZ Governor `castVoteBySig`, Uniswap V3 NFT permit. | Add ERC-1271 to the account: a P-256 owner key checked with `P256VERIFY`. Bind the hash to the account (ERC-7739-style nested typed data) to stop the same key's signature being replayed across accounts. Support both selectors `0x1626ba7e` and legacy `0x20c13b0b`. Until then, mark these flows **"with changes: on-chain path only"**. |
| **B2** | `EastSeaAccount` has only `receive()`. It has **no `onERC721Received` / `onERC1155Received` / `onERC1155BatchReceived`** and no fallback. | `safeTransferFrom` and `_safeMint` to any delegated EastSea user revert. This hits Uniswap V3/V4 position NFTs (when sent with `safeTransferFrom`), every ERC-1155, ERC721A `_safeMint` and Seaport ERC-1155 fills. | Add the receiver hooks to the account (pure functions that return the selector). This is a node-side change, made with B1. |
| **B3** | Canonical addresses cannot be reproduced. The CREATE2 deployer `0x4e59b448…`, Multicall3 `0xcA11bde0…` and the Safe singleton factory are deployed with presigned keyless legacy RLP transactions. EastSea envelopes are not RLP (`crates/execution/src/tx.rs`). No predeploys exist in genesis. | Without them: no Permit2 at `0x000000000022D473…`, and no same-address Safe or Seaport. Tooling such as viem, wagmi and forge scripts hard-codes these addresses. | **Genesis allocation** of the Arachnid CREATE2 deployer (Unlicense) and Multicall3 (MIT) with mainnet runtime code. Permit2, Seaport and Safe then deploy through the CREATE2 deployer to their mainnet addresses (same initcode and salt). Genesis changes are on the mainnet critical path, so decide now. |
| **B4** | Design and code disagree. `docs/design/04-execution.md` says the precompile whitelist is `0x01,02,05,06-08,09,0x100` with "the rest disabled" and EIP-7667 hash gas. The code (`crates/execution/src/block.rs:111`) uses stock `Context::mainnet()`, whose revm 43 default spec is **OSAKA**: all mainnet precompiles, standard gas, the EIP-7825 per-tx gas cap of **16,777,216**, and `prevrandao = 0` (never set). | Contracts run as on Ethereum Osaka today. If the whitelist or the gas changes are added later, the proof suite catches it. | Decide which is the spec. The probe suite (H1) pins today's behaviour. |
| **B5** | State budget (`crates/execution/src/fees.rs`, design 27 B5): 100,000-unit burst, refill **32 units/block**, 512 new slots/block. Code costs 1 unit/byte; a new slot or account costs 100 units. | A ~22 KB Uniswap V3 pool deploy uses ~22% of the burst, and refilling that takes ~11.5 min. Deploying the whole originals suite live (~0.4–0.6 M units, estimate) needs **~3–5 h of refill** on one chain. Executor-harness runs are unaffected (fresh state). | Schedule live deployments, or deploy on a devnet with a raised budget and report both numbers. |
| **B6** | The existing toolbox `SimpleMultisig` (#8) and `SimpleDAO` (#11) verify signers with `ecrecover`. Their harness tests use secp256k1 keys. | P-256 users cannot sign for them. | Note in their READMEs. Fix together with B1, using OZ `SignatureChecker`. |

**Corrections to the brief's licence assumptions:**
- **Solmate is AGPL-3.0-only**, not MIT.
- **Aave V3 core (v3.0.x) is MIT now**: its BUSL changed to MIT on 2023-01-27. Aave v3.2+ (`aave-v3-origin`) is BUSL with an anti-migration grant.
- **Compound III (Comet) is GPL-2.0-or-later** since 2025-12-31 at the latest.
- **Orca Whirlpools moved from Apache-2.0 to a proprietary "Orca License"** on 2025-02-27.
- **Curve grants no licence at all.**
- **Sablier Lockup is BUSL until 2029-07-01.**

**Publishing posture** (applies to every item):
- Code ships "AS IS", with the original licence's no-warranty clause plus our own.
- Pipln deploys, hosts and operates nothing on mainnet, and runs no front end for these contracts.
- No yield, price or "safe/verified/audited" claims.
- Benchmarks are neutral measurements. We publish the method, the pinned commits, the raw logs, and failures as prominently as passes.
- Protocol names are used only to identify the original (nominative use): no logos, and no wording that implies endorsement by the original teams.
- Originals keep any admin roles they have (Aave ACL, Compound admin, Maker `auth`, Chainlink owner). In our test deployments the throwaway test deployer holds them, and the README says so. These are proofs, not shipped templates.
- BUSL items are run only in tests and on testnets (non-production use).

---

## 1. EastSea compatibility profile → hazards → tests

Facts are from code at the `lead` worktree on 2026-10-06.

| ID | Hazard | EastSea fact | Affected originals | Test (every one runs in the executor harness) |
|---|---|---|---|---|
| H1 | Opcode support | revm 43, spec OSAKA: PUSH0, TSTORE/TLOAD, MCOPY, CLZ (EIP-7939) present. BLOBHASH returns 0 (no blobs). BLOBBASEFEE comes from revm's default blob excess. | V4 and Seaport 1.6 (TSTORE), Balancer V3, EntryPoint 0.8, solc ≥0.8.20 output (PUSH0) | Probe contract runs each opcode and asserts results. Golden file per spec. |
| H2 | Precompiles | Stock Osaka set: 0x01–0x0a (incl. KZG point eval), BLS12-381 0x0b–0x11, P256VERIFY 0x100. **Contradicts design 04 whitelist** (B4). | Pyth/Wormhole (ecrecover), zk verifiers (bn254), Permit2/Seaport (ecrecover) | Call each precompile with EIP test vectors. Record gas and prove gas. |
| H3 | Signatures | User accounts are P-256 with 7702 delegation (the tx signature is the authorisation). Protocol also accepts secp256k1 tx signers (`SignerScheme::Secp256k1`). `ecrecover` can never return a P-256 user's address. | EIP-2612 permit (Uni V2 LP, OZ ERC20Permit, DAI, aTokens), Permit2, Seaport, Safe, 1inch, CoW, Governor-bySig, ERC20Votes `delegateBySig` (ecrecover only) | Per flow, three cases: (a) P-256 user via ERC-1271 (expected to fail until B1), (b) on-chain alternative (approve + action batched in one `execute(Call[])`, Safe `approveHash`, Seaport `validate`, CoW `setPreSignature`), (c) secp256k1 test signer. Report which ones pass. |
| H4 | 7702 code on EOAs | Delegated accounts have 23-byte code. `extcodesize(user) > 0` and `tx.origin == msg.sender` still holds. Account lacks token receiver hooks (B2). | `safeTransferFrom` / `_safeMint`, "no contracts" mint guards, Permit2/Seaport ECDSA-vs-1271 branching | Mint, transfer and receive NFTs/1155s to a delegated user. Expected to fail until B2. |
| H5 | Block cadence | ~1 s blocks; `block.number` ≈ seconds. | Compound V2 (`blocksPerYear = 2102400` constant → 15× rates unless deploy parameters are divided by 15), OZ Governor block clock (voting period in blocks), MasterChef-style per-block rewards | Assert annualised rate after 86,400 blocks. Governor period in blocks. Record parameter changes as "deploy params only, bytecode unmodified". |
| H6 | Timestamps | Set by consensus; same-second blocks possible? (unverified) | Uni V2/V3 TWAP (`timeElapsed > 0` branches), Seaport start/end, Sablier, Chainlink staleness | Property: timestamp is non-decreasing. TWAP over consecutive blocks with equal and unequal timestamps. |
| H7 | Randomness | `prevrandao = 0` always (`Context::mainnet` default, never set). Threshold-BLS beacon is readable before its epoch, so commit-reveal is needed. | Any mint or raffle using `block.prevrandao`; Candy-Machine-style reveals | Assert `prevrandao == 0`. Reveal flows use the beacon with commit-reveal (toolbox #14 pattern). |
| H8 | Gas limits | Block: 30 M exec, 200 M prove. Per-tx 16,777,216 (EIP-7825). Code 24,576 B / initcode 49,152 B (stock revm, assumed). Prove gas is separate and declared per tx. | Aave/Seaport/V3 NPM near 24 KB; EntryPoint `handleOps` bundles; Balancer Vault | Deploy each at the size limit. Bundle sizes up to the cap. Record exec vs prove gas. |
| H9 | Paid state | 100 u per new slot or account, 1 u per code byte, receipts 1 u/32 B. Burst 100,000 u, refill 32 u/block, 512 new slots/block. Floor price 10¹² wei/unit; exec/prove base fee 0 at or under target. | V2/V3 per-pool contracts, V3 ticks (each newly initialised tick ≈ 4–5 slots + bitmap word, estimate), Seaport `orderStatus`, CLOB resting orders, airdrops (new holders) | Per action: new slots, units. Saturation runs until the binding limit (slots, units or gas). Sustained/day = 2,764,800 u ÷ units. |
| H10 | SELFDESTRUCT | EIP-6780 semantics (Cancun+). | Old factories relying on redeploy-after-destruct | Assert code persists after SELFDESTRUCT outside the creation tx. |
| H11 | Canonical addresses | No keyless deploys (B3). | Multicall3, CREATE2 deployer, Permit2, Safe, Seaport, EntryPoint | If the genesis allocation is accepted: check `extcodehash` equals mainnet at the same address. Otherwise report "different address". |
| H12 | Native staking | Contracts cannot stake the coin. | Lido, Rocket Pool, Marinade, Jito | Verdict "No" for real staking. Behaviour proof with a mock reward source only. |
| H13 | Tx format and tooling | Custom P-256 envelope, not RLP. `eth_sendTransaction` goes through the wallet. | forge `script --broadcast`, Hardhat deploy scripts | Deployment goes through the harness or SDK. Document the forge → EastSea deploy path. |
| H14 | Old compilers | Originals use solc 0.4.18–0.8.28 and Vyper 0.3.x. | WETH9 (0.4.x), V2 (0.5.16/0.6.6), V3 and Safe (0.7.6), Maker (0.6.12), Yearn V3 (Vyper) | Fidelity check (§5): rebuilt runtime equals mainnet `eth_getCode`, modulo immutables and metadata. |

---

## 2. Ethereum catalog (ranked by usage × proof value)

Usage figures are from the DefiLlama API (`api.llama.fi/protocols`, `/overview/dexs|aggregators|fees`), pulled **2026-10-06**, unless another source is noted.

- **Approach:** **U** = unmodified original (same source, compiler and settings). **U*** = unmodified bytecode, chain-specific constructor parameters only. **CR** = clean-room rebuild from spec or whitepaper (licence forbids copying).
- **Expected verdict:** a pre-test hypothesis. The tests produce the real verdict.

| # | Original (pin) | What it does | Usage (source, date) | Licence | Approach | Expected verdict on EastSea | Hazards | Size |
|---|---|---|---|---|---|---|---|---|
| E1 | **Multicall3** | Batch read/write calls | 315 chains (`mds1/multicall3/deployments.json`, 10-06) | MIT | U (+ genesis address) | Yes; canonical address only with B3 | H11 | S |
| E2 | **Arachnid CREATE2 deployer** | Deterministic deployer | 375★; underlies Permit2/Seaport addresses | Unlicense | U (genesis) | Yes; needs B3 | H11 | S |
| E3 | **WETH9** | Wrapped native coin | De facto standard (no usage figure; unverified) | GPL-3.0 (Dapphub header) | U (solc 0.4.19) | Yes | H14 | S |
| E4 | **Uniswap V2** core + Router02 | Constant-product AMM | 30-day volume $1.66 B, TVL $1.04 B | GPL-3.0 | U | Yes. LP `permit` only "with changes" (H3) | H3, H9 (pair ≈ 10k u, estimate) | M |
| E5 | **Uniswap V3** core + periphery (NPM, SwapRouter, QuoterV2) | Concentrated-liquidity AMM | 30-day volume $43.4 B, TVL $1.71 B | core BUSL→**GPL-2.0-or-later 2023-04-01**; periphery GPL-2.0-or-later | U (0.7.6) | Yes. NFT permit and safe-transfer need B1/B2 | H2, H4, H9 (pool ≈ 22k u; ticks) | L |
| E6 | **Uniswap V4** PoolManager + v4-periphery PositionManager/V4Router + Universal Router | Singleton AMM with hooks | 30-day volume $42.4 B, TVL $1.21 B | core **BUSL-1.1 until earlier of 2027-06-15** → MIT; periphery MIT; Universal Router GPL-3.0 | U, **tests/testnet only** | Yes (needs TSTORE ✓). Pools are slots, not contracts, so much cheaper per pool on EastSea | H1, H3, H4 | L |
| E7 | **Permit2** | Signature-based token approvals | Required by Universal Router and UniswapX (usage figure unverified) | MIT | U (via E2 → mainnet address) | **With changes:** `AllowanceTransfer.approve` works; signature paths need B1 | H3, H11 | S |
| E8 | **Safe v1.5.0** + ProxyFactory + CompatibilityFallbackHandler + MultiSend | Multisig smart account | 63.4 M accounts; $35 B secured Q1 (Safe Q2-2026 report) | LGPL-3.0 | U | Yes via `approveHash` or owner-as-executor. Contract sigs need B1 | H3, H11 | M |
| E9 | **OpenZeppelin 5.x** Governor + TimelockController + ERC20Votes | On-chain governance | Most common DAO stack (unverified figure) | MIT | U | Yes. `castVoteBySig` needs B1; `delegateBySig` is ecrecover-only | H3, H5 | M |
| E10 | **Seaport 1.6** + ConduitController | NFT/token order book with off-chain orders | Fees $2.0 M / 30 days (DefiLlama "Opensea Seaport") | MIT | U (via E2) | **With changes:** on-chain `validate()` orders work; off-chain orders need B1; ERC-1155 fills need B2 | H1, H3, H4, H9 | L |
| E11 | **ERC-4337 EntryPoint v0.8** + SimpleAccount | Account abstraction (bundler) | ~1.22 B UserOps, ~63 M accounts mid-2026 (BundleBear via blockeden.xyz; secondary source) | GPL-3.0 | U; plus a P-256 account | Yes (bundler = any sender). Compare with native 7702 | H8 (prove gas unknown to bundlers) | M |
| E12 | **ERC721A** | Batch-mint NFT | Widely used in PFP drops (unverified) | MIT | U | Yes; fits paid state well (one slot per batch). `_safeMint` needs B2 | H4, H9 | S |
| E13 | **Aave V3 core v3.0.2** | Pooled lending | TVL $18.4 B | **MIT** (BUSL changed 2023-01-27); v3.2+ origin = BUSL, do not use | U | Yes, with a mock Chainlink feed. aToken `permit` is ecrecover-only | H3, H8 (library linking, 24 KB) | XL |
| E14 | **Morpho Blue** | Minimal isolated lending, immutable | TVL $11.5 B | GPL-2.0-or-later | U (0.8.19) | Yes | H3 (`setAuthorizationWithSig` is ecrecover) | M |
| E15 | **Compound V2** (Comptroller, CToken, JumpRateModelV2) | Pooled lending (most forked) | TVL $112 M (v2); v3 TVL $1.53 B | BSD-3-Clause; Comet BUSL→GPL-2.0-or-later (≤2025-12-31) | U* (rate parameters ÷ 15) | With changes: deploy parameters only | **H5**, H3 (COMP `delegateBySig`) | L |
| E16 | **Liquity V1** | Immutable CDP stablecoin, native collateral, no admin | TVL $196 M | File headers MIT; repo LICENSE GPL-3.0 (conflict; treat as GPL, unverified) | U (0.6.11) with a mock price feed | Yes; ownerless, which matches our principles | H6 | L |
| E17 | **ERC-4626**: OZ ERC4626 + **Yearn V3 VaultV3.vy** | Tokenised vault | Yearn TVL $205 M | OZ MIT; Yearn **AGPL-3.0** | U (OZ) + U (Vyper 0.3.7) | Yes. Proves Vyper bytecode too | H14 | M |
| E18 | **Chainlink** push aggregator + `AggregatorV3Interface` consumers | Price oracle | De facto EVM oracle (unverified) | MIT outside `ccip`/`keystone`/`workflow` directories | U (aggregator) with our test reporters | Yes (data comes from our reporters; no DON) | H6 | M |
| E19 | **Pyth EVM receiver + Wormhole core** | Pull oracle | 2,853 feeds, 113 chains (Pyth KPI, Dec 2025) | Apache-2.0 | U | Yes. **Strong proof:** verify a real Hermes update on EastSea | H2 (ecrecover over guardian sigs) | L |
| E20 | **1inch Limit Order Protocol v4** | Off-chain limit orders | 1inch Swap 30-day volume $3.0 B | MIT | U | **With changes:** P-256 makers need B1 (`fillContractOrder` = ERC-1271) | H3 | M |
| E21 | **CoW Protocol GPv2Settlement** | Batch auctions, intents | 30-day volume $3.56 B | LGPL-3.0 | U | Yes via `setPreSignature` (on-chain). Off-chain orders need B1 | H3 | M |
| E22 | **Sablier Lockup** | Token streams and vesting | Usage unverified | **BUSL-1.1 until 2029-07-01** | U, **tests/testnet only** | Yes | H6 | M |
| E23 | **Balancer V2** Vault + WeightedPool | Weighted multi-asset pools | TVL $20 M (after the Nov-2025 V2 exploit; details unverified) | GPL-3.0 | U | Yes | H8 (Vault size) | L |
| E24 | **Curve StableSwap** | Low-slippage stable AMM | TVL $1.29 B, 30-day volume $3.07 B | **No licence** ("all rights reserved") | **CR** from the StableSwap whitepaper | Yes (CR); original bytecode cannot be used | — | L |
| E25 | **MakerDAO DSS** (Vat, Jug, Spot, Join, Dai, Dog/Clipper) | CDP stablecoin | Sky Lending TVL $6.0 B | AGPL-3.0 | U (0.6.12) | Yes; DAI `permit` is ecrecover-only | H3 | XL (stretch) |
| E26 | **Lido stETH** | Liquid staking | TVL $26.8 B | GPL-3.0 | — | **No:** contracts cannot stake the coin (H12). Behaviour shown by L6 (§3) | H12 | — |
| E27 | **Gnosis Conditional Tokens + Polymarket CTF Exchange** | Prediction-market positions and order book | Usage unverified | LGPL-3.0 / MIT | U | Yes; off-chain orders need B1. App-registry classification = gambling-like (see design 31 §15.1) | H3, H4 (ERC-1155 → B2) | M (optional) |
| E28 | **ENS** registry + registrar | Names | — | MIT | U (optional) | Yes. **Low priority:** EastSeaNames already exists | H6 | M |

**Skipped as duplicates of toolbox examples or of each other:**
- Uniswap merkle-distributor (GPL-3.0; toolbox #13 and core `MerkleDistributor` already prove this).
- Snapshot (off-chain; #11).
- UniswapX (needs B1; Permit2 and V4 cover it).
- Superfluid (MIT but XL with upgradeable host and governance; Sablier covers streams).
- Euler EVK (BUSL to 2029).
- Liquity V2 (BUSL to 2027-09-01).

---

## 3. Solana catalog → EVM re-design (proof = same user-facing behaviour)

Solana programs cannot run on EVM. Each one maps to an EVM original from §2 where one exists, otherwise to a clean-room contract. Solana licences matter only if we read their code. **Rule: design from public docs, never from the source of non-permissive programs** (Orca, Marinade, Metaplex).

| # | Solana program | Usage (DefiLlama 10-06 unless noted) | Licence | EVM design on EastSea | What changes vs Solana | Behaviour tests | Verdict | Size |
|---|---|---|---|---|---|---|---|---|
| S1 | SPL Token | Every Solana token | Apache-2.0 | ERC-20 (OZ) | No token accounts or ATAs. Balances are mapping slots, and a first receive costs 100 u | mint, transfer, approve (delegate), burn, freeze (role) | Yes | S |
| S2 | Token-2022 extensions | — | Apache-2.0 | CR `ERC20Ext` on OZ: transfer fee, transfer hook (callback), metadata pointer (events + URI), non-transferable (ERC-5192), interest-bearing *display*, default-frozen allowlist | Confidential transfers need ElGamal and ZK proofs: **No** (out of scope; bn254 exists but no verifier work planned). Permanent delegate is an admin power: demonstrated only, never default | One test per extension, plus FoT handling against the original Uniswap V2 router (`supportingFeeOnTransfer`) | Yes, except confidential | M |
| S3 | Metaplex Token Metadata / Core | Standard NFT stack | Metaplex NFT licence (restrictive) | ERC-721A + ERC-2981 + metadata URI | Royalties are advisory (as on Solana pNFT-less) | mint, update URI (creator-only, freezable), royalties | Yes | S |
| S4 | Candy Machine | NFT drops | Metaplex licence | ERC-721A drop: phases, merkle allowlist, per-wallet cap, commit-reveal randomness from the beacon | No prevrandao (H7); reveal uses the beacon after commit | allowlist, cap, sold-out, reveal integrity | Yes | M |
| S5 | Bubblegum compressed NFTs | 120.7 M cNFTs minted in Feb 2024 alone (Metaplex blog) | Metaplex licence | CR **Merkle-committed collection**: one root slot; leaves in events; an NFT is materialised (ERC-721A) only when claimed or transferred | Solana keeps the leaf ledger in an off-chain indexer with concurrent Merkle trees. EVM equivalent: root + proofs; transfers before claim are proof-updating owner-signed leaf swaps (state-cheap) | 1 M-leaf root, claim, proof-transfer, double-claim rejection; state units per 1,000 owners | With changes (claim to own on-chain) | M |
| S6 | OpenBook v2 / Phoenix | OpenBook TVL $1.1 M | OpenBook v2: GPL-3.0 (instructions), rest mixed | CR **on-chain CLOB**: price-level linked lists, post-only/IOC, crank-less match-on-take | Each resting order costs ≥2 slots (≥200 u); cancels refund via delete | place, match, cancel, partial fill; order-book saturation under 512 slots/block | Yes; benchmark is the interesting result | L |
| S7 | Raydium AMM / CPMM; PumpSwap | Raydium 30-day volume $9.5 B; PumpSwap $12.7 B | Apache-2.0 | **Uniswap V2 original** (E4) | No OpenBook coupling | swap, add/remove, TWAP | Yes | (E4) |
| S8 | Orca Whirlpools; Raydium CLMM; Meteora DLMM | Orca $8.0 B, Meteora DLMM $6.0 B / 30 days | Orca proprietary since 2025-02-27; Raydium CLMM Apache-2.0 | **Uniswap V3 original** (E5). DLMM bin model: none (optional later) | Tick state costs paid slots (H9) | range LP, fee accrual, cross-tick swap | Yes | (E5) |
| S9 | Jupiter aggregator | 30-day volume $14.9 B | closed | **Universal Router** (E6) executes off-chain-built routes across V2, V3 and V4 | Routing engine is off-chain (our test builds routes) | 3-hop mixed route, slippage, deadline | Yes | (E6) |
| S10 | pump.fun bonding curve → PumpSwap graduation | 30-day volume $3.6 B; >15 M tokens to May 2026 (tokenomist; unverified) | closed | Toolbox **#5 BondingLaunchpad** (exists), plus a new test graduating into the **original** Uniswap V2 | — | buy, sell, graduate into a real V2 pair | Yes | S |
| S11 | Marinade / Jito / Sanctum LSTs | Jito TVL $1.26 B, Marinade $276 M | Marinade: all rights reserved; Jito: SPL stake-pool Apache-2.0 | ERC-4626 vault (E17) with a share price that rises from a **mock** reward source | **No real staking on EastSea** (H12) | deposit, rate update, redeem, no-loss rounding | **No** for staking; vault mechanics yes | S |
| S12 | Pyth (Solana) | — | Apache-2.0 | **Pyth EVM original** (E19) | Same pull model | update with real VAA, staleness | Yes | (E19) |
| S13 | Squads v4 multisig | Usage unverified | AGPL-3.0 | **Safe v1.5.0** (E8) | — | k-of-n, spending-limit module equivalent (Safe Allowance module, LGPL) | Yes | (E8) |
| S14 | Streamflow vesting / payments | TVL $17 M | proprietary SDK (unverified) | **Sablier Lockup** (E22) or OZ `VestingWallet` (MIT) | — | cliff, linear, cancel, withdraw-max | Yes | (E22) |
| S15 | SPL Governance (Realms) | — | Apache-2.0 | **OZ Governor** (E9) | Block clock (H5) | propose, vote, queue, execute | Yes | (E9) |
| S16 | Solana Name Service | — | — | **EastSeaNames** (exists) | — | none new | Yes (existing) | — |
| S17 | Drift / Mango / Jupiter Perps | Jupiter Perps TVL $817 M | Drift Apache-2.0 | **Out of scope** | Keepers, oracles and liquidation engines are XL; little value as a proof | — | Not attempted | — |

---

## 4. Licences: what we may do, and where the code lives

All licence data comes from GitHub `LICENSE` files and SPDX headers, read through `gh api` on 2026-10-06. BUSL change dates are quoted from each licence's Parameters block.

| Licence class | Items | May we copy, publish and deploy? | Placement |
|---|---|---|---|
| MIT / BSD-3 / Apache-2.0 / Unlicense | Multicall3, CREATE2 deployer, Permit2, Seaport, OZ 5.x, Solady, ERC721A, Aave v3.0.2 core (converted), 1inch LOP v4, Chainlink (non-CCIP), Pyth + Wormhole, Compound V2 (BSD-3), Uni v4-periphery, Polymarket CTF exchange, ENS | Yes. Keep the original copyright notices | Vendored under `originals/<name>/` (git submodule pinned to an exact commit) |
| GPL-2.0-or-later / GPL-3.0 / LGPL-3.0 | WETH9, Uniswap V2, **V3 (core BUSL→GPL-2.0-or-later 2023-04-01; files still carry the `BUSL-1.1` header, so add a NOTICE citing the change date)**, Universal Router, Safe (LGPL), CoW (LGPL), Gnosis CTF (LGPL), Morpho Blue, Balancer V2, EntryPoint, Lido, Liquity V1 (conservative), Comet | Yes, if the published derivative is GPL-compatible. **Our tests and scripts that compile together with GPL code are licensed GPL-3.0-or-later** | `originals/gpl/` with its own `LICENSE` (GPL-3.0-or-later). The toolbox root stays MIT |
| AGPL-3.0 | MakerDAO DSS, Yearn V3, **Solmate**, Squads v4 (not used), CreateX (not used) | Yes, with network-use source offer (moot: we host nothing) | `originals/agpl/` with an AGPL `LICENSE`. **Never import Solmate into MIT examples** (the toolbox currently uses OZ 5.1.0 only, verified in the fixture provenance) |
| BUSL-1.1 (still active) | **Uniswap V4 core** (→ MIT on the earlier of 2027-06-15 or the ENS-set date), **Sablier Lockup** (→ GPL-3.0-or-later or similar on the earlier of 2029-07-01 or the ENS date; unverified change licence), Aave ≥3.2, Euler, Liquity V2 | BUSL grants copy, modify, redistribute and **non-production use**. Tests and testnet benchmarks only; no mainnet deployment by anyone under our instructions | `originals/busl/` as **submodules only** (we redistribute nothing modified), with a README warning about production use. **Lawyer check: is a public testnet benchmark "non-production"?** |
| No licence / proprietary | Curve (all rights reserved), Orca Whirlpools (Orca License since 2025-02-27), Marinade (all rights reserved), Metaplex (Metaplex NFT licence), Jupiter, pump.fun | Do not copy. Clean-room rebuild from public specs; the person writing the code must not read the source | Normal MIT `examples/` or `contracts/src/clones/`. Record the spec sources used |

**Executor harness note:** `crates/contracts-onchain/fixtures` (aether-node repo, MIT OR Apache-2.0) checks in a source snapshot and artifacts. Put GPL, AGPL and BUSL artifacts in **separate fixture subfolders** that carry the original LICENSE and pinned source URL, as aggregation rather than combination. Apache-2.0 node code only loads the bytecode as data. **Lawyer check.**

---

## 5. Proof method and benchmarks (published per contract)

1. **Fidelity.** Build from the pinned upstream commit with the original solc or Vyper version, optimizer settings and evmVersion. Compare runtime bytecode with Ethereum mainnet `eth_getCode` at the canonical address, masking immutables and the CBOR metadata tail. Result per contract: **identical / differs only in immutables / not comparable (no mainnet instance)**.
2. **Behaviour.** A scripted user-journey scenario per item (the "Behaviour tests" in §2 and §3). It runs twice:
   - in Foundry (fork-free; unit + fuzz, plus invariants where funds are held, reusing the upstream invariant suites where they exist);
   - **through the EastSea executor harness** (`crates/contracts-onchain`, `scripts/run-contracts-onchain.sh`): real P-256 envelopes, paid state, comparison of build / execute / sequential execution, rollback on revert.
3. **Hazards.** H1–H14 probes (§1), each with a pass/fail line per item.
4. **Benchmarks.** For deploy and for each user action:
   - exec gas, prove gas, state units, new slots, persisted bytes;
   - fee at the floor (units × 10¹² wei; exec/prove base 0 at or under target);
   - Ethereum mainnet gas for the same action, as a neutral reference;
   - **actions per block** with the binding limit named (30 M exec / 200 M prove / 100,000 u burst / 512 slots / 2 MiB payload), and **sustained actions per day** (2,764,800 u ÷ units) and per second;
   - **live-chain latency** on devnet/testnet: submit → inclusion → finality, p50/p95 over ≥100 tx.

   Use the existing report format in `docs/research/contracts-onchain-2026-10-06.md` (it already reports e.g. ERC-20 transfer to a new holder = 125 u, 512/block slot-bound, 22,118/day).
5. **Summary table.** One row per item: **Runs on EastSea: Yes / With changes (what) / No (why)**, plus links to logs. Expected "With changes" rows today come from B1–B3, H5 and H12.

---

## 6. Build plan: 4 lanes, one coder each, one branch each in `eastsea-toolbox`

**Lane 0 (blocking, ~1 day, lead):**
- Scaffold `originals/` (MIT, gpl, agpl and busl subfolders; one foundry profile per compiler setting).
- Write the fidelity script (`cast code` comparison).
- Build the H1–H14 probe suite.
- Add the benchmark recorder that extends the harness report.
- File the node tickets B1/B2 (account ERC-1271 and receiver hooks) and the B3 genesis decision.

Lanes are ordered most-used first. Tests per item: fidelity + behaviour (Foundry + harness) + listed hazards + benchmarks. Funds-holding items also get fuzz and invariants: conservation, no-loss rounding, and exits still open where the original has them.

| Lane / branch | Items in order | Notes | Rough size |
|---|---|---|---|
| **A `proof/infra`**: accounts, signatures, tooling | E1 Multicall3 → E2 CREATE2 → E3 WETH9 → E7 Permit2 → E8 Safe 1.5 (+MultiSend, Allowance module = S13) → E11 EntryPoint v0.8 + P-256 account → E9 OZ Governor (= S15) → E12 ERC721A (= S3) | Owns the H3/H4 three-way signature matrix and the B1/B2 regression tests that flip to pass when the account is fixed | ~1 L + 3 M + 4 S |
| **B `proof/amm`**: swaps and routing | E4 Uniswap V2 (= S7; + S10 graduation test) → E5 V3 (= S8) → E6 V4 + Universal Router (= S9; BUSL folder) → E23 Balancer V2 → E24 Curve StableSwap (clean-room; a separate coder or a strict no-source rule) | Owns the H9 saturation runs (pool creation, tick crossing) | 3 L + 1 L(CR) + M |
| **C `proof/credit`**: lending, CDP, vaults, oracles | E18 Chainlink aggregator (mock feeds first; others depend on it) → E13 Aave v3.0.2 → E14 Morpho Blue → E15 Compound V2 (H5 parameters) → E16 Liquity V1 → E17 OZ ERC4626 + Yearn V3 (= S11 LST vault, "No" for staking) → E19 Pyth + Wormhole (real VAA) → E25 Maker DSS (stretch) | Heaviest lane; Maker is the first item to drop | 2 XL + 3 L + 2 M |
| **D `proof/markets-solana`**: orders, streams, Solana behaviours | E10 Seaport 1.6 → E20 1inch LOP v4 → E21 CoW GPv2 (pre-sign) → E22 Sablier Lockup (= S14; BUSL) → S2 `ERC20Ext` (Token-2022) → S4 Candy-Machine drop → S5 Merkle-compressed NFTs → S6 on-chain CLOB → E27 CTF + Polymarket (optional) → E28 ENS (optional) | S2/S4/S5/S6 are clean-room MIT code under `contracts/src/clones/`, each with the toolbox's 4 docs (README, SECURITY, GAS, manifest) | 1 L + 4 M + 3 M(CR) + 1 L(CR) |

**Merge rules (all lanes):**
- `forge fmt --check` covers our code only; vendored originals are excluded.
- CI runs `FOUNDRY_PROFILE=ci` for our tests plus the upstream test suites we vendor, fork-free.
- **Every item must also pass the executor harness run before merge** (`scripts/run-contracts-onchain.sh`, with fixtures regenerated by `scripts/generate-contract-fixtures.py`, licence-separated subfolders per §4).
- Reviewer is the lead; red team is Codex, per existing delegation rules.
- Nothing from `originals/busl/` is deployed outside tests and testnets.

---

## 7. Sources (all accessed 2026-10-06)

- **EastSea code:**
  - `crates/execution/src/block.rs` (L111 `Context::mainnet`, no spec, prevrandao or precompile override)
  - `crates/execution/src/fees.rs` (L36–60 state constants)
  - `crates/execution/src/tx.rs` (signer schemes, 7702 delegate)
  - `contracts/src/EastSeaAccount.sol` (no `isValidSignature` or receiver hooks; L481 `receive`)
  - revm-context 43.0.3 `cfg.rs` L448 (EIP-7825 cap under OSAKA) and `block.rs` L122 (`prevrandao: Some(ZERO)`)
  - revm-primitives 43.0.0 `hardfork.rs` L71 (`#[default] OSAKA`)
- **EastSea docs:**
  - `docs/design/04-execution.md`, `27-state-fee.md` (B5), `31-app-registry.md` §8 (brake)
  - `docs/design/12-launch-plan.md` E7
  - `docs/research/contracts-onchain-2026-10-06.md`
- **Licences** (GitHub `LICENSE` / SPDX via `gh api`):
  - Uniswap/v2-core, v3-core (BUSL params: change 2023-04-01 → GPL-2.0-or-later), v4-core `licenses/BUSL_LICENSE` (2027-06-15 → MIT), v4-periphery, permit2, universal-router
  - aave/aave-v3-core (BUSL → MIT 2023-01-27), aave-dao/aave-v3-origin
  - compound-finance/compound-protocol, comet (→ GPL-2.0-or-later 2025-12-31)
  - curvefi/curve-contract, stableswap-ng
  - balancer v2/v3; makerdao/dss; lidofinance/core; yearn-vaults-v3; morpho-blue; liquity/dev, bold (2027-09-01)
  - safe-global/safe-smart-account (v1.5.0, 2025-07-03; `ISignatureValidator` 0x1626ba7e)
  - ProjectOpenSea/seaport (1.6; seaport-core TSTORE guard, ERC-1271 fallback)
  - mds1/multicall3; Arachnid/deterministic-deployment-proxy
  - eth-infinitism/account-abstraction (v0.9.0 latest; EntryPoint GPL-3.0)
  - smartcontractkit/chainlink; pyth-network/pyth-crosschain; sablier-labs/lockup (2029-07-01)
  - 1inch/limit-order-protocol; chiru-labs/ERC721A; transmissions11/solmate (AGPL-3.0-only headers); OpenZeppelin (ERC20Permit ECDSA-only; Governor SignatureChecker)
  - cowprotocol/contracts; Polymarket/ctf-exchange; gnosis/conditional-tokens-contracts
  - Solana repos: solana-program/token, token-2022; metaplex mpl-*; openbook-dex/openbook-v2; raydium-io/*; orca-so/whirlpools (Orca License from 2025-02-27); marinade-finance/liquid-staking-program; Squads-Protocol/v4; drift-labs/protocol-v2
- **Usage:**
  - DefiLlama API `api.llama.fi/protocols`, `/overview/dexs`, `/overview/aggregators`, `/overview/fees` (snapshot 2026-10-06)
  - [Safe Q2-2026 report](https://forum.safefoundation.org/t/safe-q2-2026-quarterly-report-is-live/7072); [Safe Q1-2026](https://safefoundation.org/blog/safe-q1-2026-quarterly-report)
  - [blockeden.xyz on 4337 (secondary)](https://blockeden.xyz/blog/2026/02/10/account-abstraction-40m-wallets-erc-4337/)
  - [Pyth KPI](https://docs.pyth.network/home/metrics/kpi)
  - [tokenomist pump.fun (secondary, unverified)](https://tokenomist.ai/pump-fun)
  - [Metaplex round-ups](https://www.metaplex.com/blog/articles/metaplex-may-round-up-2025)
  - `safe-global/safe-singleton-factory` (660 chain artifacts)
