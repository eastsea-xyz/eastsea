# Next-gen consensus frontier (verified 2026-09-26)

## 1. Solana Alpenglow

- **Votor** (voting/finalization): one round when >=80% stake responsive (~100 ms), two rounds otherwise (~150 ms). Votes are off-ledger P2P messages (removes ~59% of Solana txs). Fault model "20+20": safety up to 20% Byzantine, liveness with a further 20% offline (finality halts past 40% non-responsive). Validator Admission Ticket replaces vote fees.
- **Rotor** (Turbine replacement, single-hop erasure-coded relay) is deferred to a later SIMD; the shipping upgrade is Votor only.
- **Status**: SIMD-0326 approved; community cluster since May 2026; bug bounty (50k SOL) Aug 5-19; **public testnet switched to Votor at slot 444625255 on 2026-09-24**; Agave 4.3 mainnet feature-gate tentatively 2026-09-28, but Anza says mainnet-beta only after observation windows (expect Q4 2026). Informal Systems published a **Quint spec** (informalsystems/Alpenglow-spec); Superteam ran a formal-verification bounty (Lean/TLA+/Stateright submissions, no published machine-checked proof yet).
- **Reusable**: the 20+20 dual-quorum idea (fast 80% / slow 60%) and off-chain vote gossip. Agave code is Rust but deeply Solana-specific; the concept maps cleanly onto Simplex-style protocols (see Minimmit).

Sources: solanacompass.com (Alpenglow adoption/testnet), helius.dev/blog/alpenglow, alpenglow.anza.xyz, github.com/informalsystems/Alpenglow-spec.

## 2. Simplex family and DAG BFT

| Name | Latency / finality | Fault model | Offline tolerance | Rust / maturity | Source |
|---|---|---|---|---|---|
| Simplex (Commonware `simplex`, incl. BLS threshold scheme) | 3 msg delays, ~2-3 delta; O(n^2) votes or O(n) w/ threshold certs | f < n/3, partial sync | liveness needs 2f+1 online | `commonware-consensus 2026.9.0`; production (Alto, Tempo); the threshold variant is now a `scheme` inside `simplex` | docs.rs/commonware-consensus; decentralizedthoughts "Chapter: Simplex" (2026-05) |
| Minimmit (Commonware, FC'26) | 2 delta finality, 1 vote round; 2f+1 notarize / n-f finalize; 130 ms block / 250 ms finality on 50 global nodes | f < n/5 (n >= 5f+1) | tighter: needs 80% honest-online | Spec + `commonware-estimator` sims; **not yet a crate module** | commonware.xyz/blogs/minimmit; arXiv 2508.10862; Multimmit arXiv 2607.21021 |
| Kudzu (Shoup et al., DISC'25) | 2 rounds fast path, 3 delta slow path; erasure-coded dispersal | n = 3f+2p+1 | as Simplex | paper only, no public Rust | arXiv 2505.08771 |
| Autobahn (Berkeley/Cornell) | data lanes + 1-round fast path (3 mds); "seamless" recovery after blips | f < n/3 | good: lanes keep streaming during partitions | Rust/tokio artifact (research) | github.com/neilgiri/autobahn-artifact |
| Mysticeti v2 (Sui, prod) | ~3 msg delays commit, sub-second; v2 folds validation into consensus (-25-35% latency) | f < n/3 | DAG tolerates lagging nodes | Rust, mainnet | blog.sui.io/mysticeti-v2-sui-consensus |
| Mahi-Mahi | 5-6 msg delays but **asynchronous** (no timers) | f < n/3 async | best under churn | Rust fork of Mysticeti (~14k LOC), research | arXiv 2410.08670 |
| Shoal++ / Raptr (Aptos, prod) | ~700 ms median at 18k tps; Raptr prefix-consensus keeps latency within 15% at 1% loss | f < n/3 | Raptr strongest under packet loss | Rust, Aptos mainnet | arXiv 2405.20488, 2504.18649 |

Newer papers: Gatling (parallel composition, arXiv 2606.18220), Hermes (prefix consensus, 2607.25916), Cadence multi-proposer (2607.02275).

## 3. Ethereum lean consensus

- **3SF**: finality in 3 slots under bounded delay; "seconds not minutes". Devnets pq-devnet-0..4 (Oct 2025-Mar 2026) done; pq-devnet-5 (block-level proofs + Goldfish fork choice) next. FOCIL ships in Hegota (2027). No testnet/mainnet date; core PQ milestones ~2029.
- **leanSig/leanMultisig**: Winternitz-XMSS signatures aggregated inside **leanVM** (a minimal zkVM; SP1/OpenVM/Jolt variants). Benchmarks: ~400 XMSS agg/s; sig verification at 39% of target on M1, aggregation 97% of target on M4 Max, aggregate size still 3-4x over target.
- Rust clients: ream, ethlambda (LambdaClass), ethean; Go: gean, zeam.
- Sources: leanroadmap.org, hackmd.io/@tcoratger/ryS1ElrWbx, arXiv 2411.00558, github.com/leanEthereum/pm.

## 4. Home validators with intermittent uptime

| Approach | Finality | Fault model | Offline tolerance | Rust / maturity |
|---|---|---|---|---|
| Ouroboros Praos/Genesis (VRF lottery, sleepy) | probabilistic, minutes | 1/2 honest **of online** stake | excellent (dynamic availability) | Cardano (Haskell); `pragma-org/ouroboros` Rust |
| Committee rotation + BFT (Alpenglow, Aptos epochs) | sub-second | 1/3 or 1/5 of committee | poor inside epoch; fine if committee = high-uptime subset | Rust prod |
| Ebb-and-flow / 3SF (available chain + finality gadget) | seconds | 1/3 for finality, 1/2 for availability | good: chain keeps growing, finality pauses | Lean devnets |
| Mahi-Mahi / async DAG | 5-6 delays | 1/3 async | good | Rust research |

MEV/leader-rotation: FOCIL (16-member inclusion committees, EIP-7805, Hegota 2027); multi-concurrent proposers (arXiv 2509.23984; AMP 2605.23677; Cadence); encrypted mempools converging on batched threshold decryption with silent setup (SoK eprint 2026/1643; BEAST-MEV; Shutter on Gnosis). Fino/Sphinx not found in 2026 literature.

## 5. Sybil resistance without a token

- Proof of personhood (World ID, Human Passport, zk credentials): easiest for a permissioned research testnet, weakest decentralization story.
- Invite graphs / web-of-trust (Brightid-style, social-graph ranking): cheap, needs a rooted set.
- Proof of space/time (Chia): permissionless but favors large disks; poor fit for Macs.
- Practical: testnet = invite-graph allowlist + hardware attestation (Apple Secure Enclave keys) with stake-free VRF lottery weighted by reputation.

## 6. Deterministic simulation and formal verification

Trend is **Quint spec -> model-based tests against Rust** (Malachite `specs/consensus/quint`, Quint Connect; Fast Tendermint shipped in ~1 week, arXiv 2608.13434), Quint spec of Alpenglow, TLA+/TLAPS for Moonshot, Lean 4 for ChonkyBFT (ZKsync) and DAG nonforking (arXiv 2504.16853). Commonware ships `commonware-runtime` deterministic executor + `commonware-estimator` latency sims. SoK: arXiv 2608.21935.

## Recommendation (Mac home validators + ZK-proven blocks)

1. **Build now**: stay on Commonware `simplex` with the BLS12-381 threshold scheme (constant-size certs are what a laptop/phone light client and a ZK circuit want). Use `commonware-runtime` deterministic sim and write a Quint spec of your fork choice + committee rotation, with model-based tests.
2. **Mimic Alpenglow's 20+20** by adding a fast 80% path only if you have >50 validators; otherwise plain Simplex is simpler and safer.
3. **Uptime**: do not put every Mac in the BFT committee. Use VRF-based epoch rotation of a small high-uptime committee (Praos-style eligibility, reputation-weighted), and let offline nodes stay as ZK-verifying full nodes. Design the availability layer ebb-and-flow style so finality pauses but the chain does not halt.
4. **Sybil** for testnet: invite graph + Secure Enclave attestation; token later.
5. **Wait on**: Minimmit (no crate yet; adopt once it lands), Rotor, leanSig/XMSS-in-zkVM (aggregation still 3-4x oversized; watch pq-devnet-5), FOCIL/multi-proposer (2027), Kudzu/Autobahn (no production Rust).
