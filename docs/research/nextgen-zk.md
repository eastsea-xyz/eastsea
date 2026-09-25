# Next-Generation ZK Proving Frontier (as of 2026-09-26)

Scope: what makes EVM-block proving 10-100x cheaper in 1-3 years, with a Metal-first (Apple Silicon) lens. "Metal/NEON" column = friendliness of the field arithmetic to 32-bit SIMD / Apple GPU. "unverified" = secondary-source claim not confirmed against primary.

## 1. Proof systems

| Name | Claim / speedup | Maturity | Who | Metal/NEON | Source |
|---|---|---|---|---|---|
| Lattice Jolt + Akita (Module-SIS PCS) | 2-3x prover+verifier vs Dory Jolt; 65-80 KB proofs (smallest PQ zkVM); ~200 B/cycle RAM; >2M cyc/s CPU, **>10M cyc/s with Metal on a MacBook** | Released 2026-09-09, open source; numbers not independently verified | a16z crypto + LayerZero/CMU/USC | **High** (shipped Metal backend) | eprint 2026/1983; thequantuminsider 2026-09-09; a16zcrypto.com |
| Greyhound / LaBRADOR | Lattice PCS, 53 KB eval proof at N=2^30, O(sqrt N) verifier | Reference impl only; no zkVM ships it | Nguyen/Seiler (LayerZero) | Medium (NTT over small moduli) | eprint 2024/1293; github LayerZero-Labs/greyhound-reference |
| Binius64 (binary fields) | Single-core CPU beats SP1/R0VM on L40S GPU ~5x (ECDSA agg), 80-100x (XMSS agg); direct circuits, 64-bit words | Alpha: ZK and succinct verifier still missing; Polygon building a Binius zkVM | Irreducible | **High** on CPU (carry-less mul, NEON PMULL); GPU story weaker | irreducible.com/posts/announcing-binius64; github binius-zk/binius64 |
| Circle STARK / M31 (S-two) | 32-bit Mersenne arithmetic, fastest CPU/GPU STARK per op; CUDA via ICICLE-Stwo / NitrooZK | Production (Starknet), Apache-2 | StarkWare, Ingonyama, AntChain | **High** (M31 mul 112 GOP/s on Apple GPU via Metal) | github starkware-libs/stwo; zkmopro.org 2026-01-26 |
| WHIR / STIR / Basefold (hash-based PCS) | Prover same O(n log n) as FRI; proof size O(log d + lambda log log d); WHIR verifier ~100s of us; Basefold field-agnostic | WHIR in production paths (EF leanVM, Airbender-style stacks); SoK eprint 2026/1367 | EF, Ligero/ZK Security, Tsinghua | High (small-field, hash-bound) | eprint 2024/1586; eprint 2026/1367 |
| Sumcheck-centric (Jolt, Spartan, SP1 Hypercube "multilinear") | Removes FFT from the hot path; Hypercube took SP1 from 160x4090 (May 2025) to 16x5090 (Nov 2025) | Production | a16z, Succinct | High (memory-bound dot products) | blog.succinct.xyz/sp1-hypercube; blog.succinct.xyz/real-time-proving-16-gpus |
| Folding: Nova/HyperNova/ProtoStar, Neo, LatticeFold+ | LatticeFold+: 5-10x faster prover than LatticeFold, PQ; Neo: small-field pay-per-bit lattice folding | Research / prototypes; no EVM zkVM ships lattice folding yet | Boneh-Chen, Nguyen-Setty | Medium | eprint 2025/247; eprint 2025/294 |
| Small-field co-design (MamaBearZKP, 49-bit) | 18x single-thread over Plonky3 AVX-512 BabyBear; 42-64x vs Goldilocks | Paper (2026) | academic | Medium-high (fits 64-bit NEON lanes) | eprint 2026/1698; eprint 2026/1371 "Small-Field Turn" |

Trend: hash-based + small-field is the default (EF requires 128-bit provable security, <=300 KiB proofs by end-2026); lattices are the new entrant for tiny proofs + PQ.

## 2. zkVM architecture

| Name | Claim | Maturity | Who | Metal/NEON | Source |
|---|---|---|---|---|---|
| ZisK | 99.7% blocks <10 s on **4x RTX 5090** (p99 9.62 s, 2026-08-18); single-4090 reporting at 128-bit | Production on Ethproofs | Polygon/ZisK | Low (CUDA-first) | github Ricosworks1 sept-2026 analysis (unverified aggregator); ethproofs |
| SP1 Hypercube | 95.4% <10 s on 16x5090; Jagged PCS/multilinear; formal RISC-V verification | Production, audited | Succinct | Low (CUDA) | blog.succinct.xyz 2025-11-18 |
| Pico Prism | 99.9% <12 s on 64x5090, 6.9 s avg (Oct 2025) | Production | Brevis | Low | x.com/drakefjustin/1978435449489158312 |
| Airbender | Full block on 1 H100 in 35 s; ~$0.0001/transfer | Production (ZKsync) | Matter Labs | Low | zksync.io/airbender |
| OpenVM 2.0 / 2.1 | RV64IM_Zicclsm target; real-time on 4 GPUs (2x over 2.0); SWIRL + continuations audited; Halo2 GPU wrap 8.1 s | Production | Axiom/Scroll | Low | blog.openvm.dev/2.0-production, /2.1 |
| Ziren (MIPS) | Groth16 wrap, Poseidon2/AES precompiles, distributed proving | Production | ZKM | Low | (release notes, unverified detail) |
| Jolt (RISC-V, lookup-centric) | See Lattice Jolt row; CPU-competitive by design | Released | a16z | **High** | above |
| Ethproofs real-time milestone | Definition: 1 proof per block, <=10 s p99, <=$100k capex, <=10 kW, 128-bit, <=300 KiB, OSS; 2026 stretch: **sub-8 s p99**; cost down to <0.5 cent/block (2026-09-02) vs $1.69 (Jan 2025) | 4 teams meet all constraints | EF / Ethproofs | -- | hackmd @willcorcoran; blog.ethereum.org/2025/07/10/realtime-proving |
| Client-side: Mopro, Ligetron, WebGPU | Metal MSM v2 40-100x over v1; ICICLE Metal up to 5x; Metal vs WebGPU: 1.9x on M31 mul, 7.6x on BN254 | Tooling, not block-scale | PSE/Mopro, Ligero | **High** | zkmopro.org 2026-01-26 |

ISA: RISC-V (RV32IM -> RV64IM_Zicclsm per EF) has won; MIPS (Ziren) and custom (Cairo, leanVM) are niche.

## 3. Hardware

| Name | Claim | Maturity | Who | Metal/NEON | Source |
|---|---|---|---|---|---|
| NVIDIA RTX 5090 | De-facto Ethproofs unit; kWh/proof metric | Production | -- | n/a | hackmd Ethproofs 2026 |
| Cysic C1 / ZK-Air / ZK-Pro ASIC | "10-100x over GPU"; ship in 2026 | Announced, unverified perf | Cysic | n/a | docs.cysic.xyz |
| Fabric VPU | 10-100x target; Polygon bought $5M of systems | Pre-production, unverified | Fabric | n/a | fabriccryptography.com |
| Ingonyama ICICLE | GPU SDK standard; Metal backend shipped (v3.6+), M31/Stwo support | Production | Ingonyama | **High** | ingonyama.com |
| Apple ANE / AMX for ZK | No published ZK use found | -- | -- | unverified/none | (search returned nothing) |
| Memory-bandwidth bound | Sumcheck/Merkle-hash phases are bandwidth-bound; M-series unified memory (up to 800 GB/s M-Ultra) vs 5090 1.8 TB/s | Analysis | -- | Medium | eprint 2026/525 SoK |

## 4. Aggregation / recursion

| Name | Claim | Maturity | Who | Source |
|---|---|---|---|---|
| Succinct Prover Network | Open global prover market, live | Production | Succinct | github succinctlabs/network |
| Boundless | Reverse Dutch auction, many small nodes | Mainnet beta | RISC Zero | medium CFrontier comparison |
| Pianist / DeVirgo / distributed sumcheck | Linear-scaling distributed provers; arXiv 2605.14015 distributed statistical ZK via sumcheck | Research -> partially productized (Pico, Ziren) | Berkeley et al. | arxiv 2605.14015 |
| Recursion cost | leanVM target: 2-to-1 recursion ~200 ms; OpenVM Halo2 wrap 8.1 s on 5090 | Improving | EF, OpenVM | leanroadmap.org |

## 5. Ethereum direction
- L1 zkEVM: EF requires soundcalc-verified >=100-bit by May 2026, 128-bit + <=300 KiB by end-2026; home prover budget $100k / 10 kW; real-time = <=10 s p99 (stretch 8 s). Source: blog.ethereum.org 2025-07-10; EF Dec-18-2025 roadmap.
- Lean Consensus: leanSig (XMSS, hash-based) + leanMultisig aggregation in leanVM (WHIR-based); pq-devnet-2 Jan 2026; target 1,000 XMSS/s; production ~2029-30. Source: leanroadmap.org, pq.ethereum.org.

## 6. Apple Silicon specifics
- Only zkVM with a first-party Metal backend at block-relevant scale: **Lattice Jolt (>10M cyc/s on a MacBook)**. RISC Zero's Metal feature is deprecated.
- Small-field (M31, KoalaBear/BabyBear, 49-bit) and binary (GF(2^64)) arithmetic map to 32/64-bit NEON lanes and Apple GPU u32 ALUs; BN254/BLS12 do not (7.6x worse on Metal, per Mopro). Goldilocks needs 64x64->128 mul (NEON lacks it; done via 32-bit splits).
- Lattice PCS (Akita) = many small-modulus NTTs + hashing -> good fit for unified memory; no bandwidth-hungry MSM.

## (a) Ranked bets for a Metal-first prover, 1-3 yrs
1. **Lattice Jolt / Akita** - already runs on Metal, PQ, tiny proofs, sumcheck-only (no FFT). Risk: young, unaudited, verifier gas unproven.
2. **Circle STARK M31 + WHIR** (Stwo stack) - best raw ops/s on Apple GPU, production-grade, EF-aligned hash-only security. Risk: 200-600 KB proofs; need recursion to wrap.
3. **Binius64** - CPU/NEON-native, fastest for hash/signature-heavy work (Keccak, XMSS). Risk: no ZK/succinctness yet.
4. Lattice folding (LatticeFold+/Neo) - future streaming/continuations; research only.

## (b) Build now vs wait
Build now: RV64IM_Zicclsm guest target; segmented/continuation executor with Metal-resident traces; M31/KoalaBear NEON+Metal kernels (sumcheck, Merkle/Poseidon2, DCCT); pluggable PCS trait so WHIR today and Akita can be swapped; soundcalc-based 128-bit config; proof-size budget <=300 KiB.
Wait for: Binius64 ZK/succinct release; Akita audit + on-chain verifier; ASICs (Cysic/Fabric, unverified); lattice folding; any ANE/AMX ZK path (none exists).
