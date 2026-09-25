# zkVM provers on Apple Silicon (Metal) — status as of 2026-09-25

Verified via GitHub API (`gh`), project docs, and press. "unverified" = claim found but not confirmed against source/repo.

## 1. zkVM prover matrix

| zkVM (latest tag) | Metal support | CPU on macOS | License | Apple Silicon benchmark (source) | Status |
|---|---|---|---|---|---|
| **Jolt** (a16z) — crates.io tags stuck at v0.3.0-alpha; `main` moves daily | **Native Metal**, on branch `feat/akita-metal` (2026-09-09) / `feat/metal` (2026-09-03): `crates/jolt-kernels/src/metal/*` (sumcheck/RAF/booleanity kernels). Not yet on `main` (main has `crates/jolt-akita` lattice PCS, CPU only). Also `--features icicle` (CUDA). | Yes, arm64 CPU (Lattice Jolt >2 M RV64IMAC cyc/s "on a laptop CPU") | MIT / Apache-2.0 | **>10 M cycles/s with Metal on a MacBook** (Lattice Jolt, a16z, 2026-09-09, chip not specified; curve-based Jolt+Metal ≈4 M/s). Not independently reproduced. | Metal code verified in repo; numbers unverified |
| **RISC Zero** r0vm v3.0.6 (2026-07-17) | **Native Metal, on by default on Apple Silicon** (`metal` feature deprecated). Kernels in `risc0/circuit/rv32im-sys/cxx/hal/metal/kernels/*.metal` (ntt, fri, poseidon2, witgen) + recursion circuit. | Yes (Metal always used) | Apache-2.0 / MIT (dual). Note: Groth16 wrap is x86-only (Docker), not on Apple Silicon | Official report site `reports.risczero.com/.../macOS-apple_m2_pro` exists but unreachable today; no cyc/s number captured | Metal verified; benchmark unverified |
| **SP1** v6.8.1 (2026-09-24) | **None.** GPU = CUDA only (Linux x86_64, CC ≥8.0, 24 GB VRAM), `sp1-gpu` dir; Hypercube RTP uses 16×RTX 5090 | Yes but slow: SP1 CPU path is AVX256/512-tuned; no NEON path documented | MIT / Apache-2.0 (prover+verifier open) | none | verified |
| **OpenVM** v2.0.2 (2026-08-14); 2.1 preview on Ethproofs | **None**; CUDA prover only (open-source, MIT/Apache) | Yes (Plonky3/NEON) | MIT / Apache-2.0 | none | verified |
| **ZisK** v1.3.0-alpha (2026-09-21) | **None**; CUDA 12.9+ only. Docs: macOS proof generation "not yet optimized, may take longer" | Yes (emulation + slow proving) | Apache-2.0 / MIT | none | verified |
| **Airbender** (ZKsync) v0.5.2 (2025-12-19) | **None**; `gpu_prover` is CUDA; final SNARK on CPU needs ~150 GB RAM | Yes (basic/recursion layers) | Apache-2.0 / MIT | none | verified |
| **Pico / Pico Prism** v2.1.2 (2026-08-17) | **None**; Pico-GPU is CUDA | Yes (KoalaBear/BabyBear/M31 STARK backends) | Apache-2.0 / MIT | none | verified |
| **Nexus** zkVM 3.x | **None**; built on Stwo (SIMD); "experimental, not for production" | Yes (Stwo NEON) | unverified (LICENSE not read) | none | partially verified |
| **Ziren** (ZKM, MIPS32) | **None**; CUDA distributed prover | Yes | unverified | none | partially verified |

## 2. Building blocks

| Library | Metal | Notes |
|---|---|---|
| Plonky3 | none | AVX2/AVX-512/**NEON** via `-Ctarget-cpu=native`; MIT/Apache |
| Stwo (StarkWare) | none | SIMD backend incl. **NEON** + wasm; no GPU crate in repo; Apache-2.0. Community WebGPU NTT shaders exist (mopro) |
| Binius64 (Irreducible) | none | CPU single-core claims vs GPU zkVMs; Metal unverified |
| ICICLE (Ingonyama) | **Yes** since v3.6 (v4.0 latest): MSM, NTT, sumcheck on Metal; missing Poseidon/Poseidon2/Merkle; backend is **binary, free for R&D only** via license server | Jolt uses it for CUDA, not Metal |
| mopro Metal MSM v2 | **Yes** | BN254 MSM on M3 Air ~0.59 s, ~2× CPU; also WebGPU field ops |
| arkworks | none | CPU only; used as baseline above |
| Apple Accelerate/AMX | none in any zkVM | no zk project found using AMX/Accelerate |

## 3. Mina precedent
o1js native Kimchi prover (o1Labs, 2026-02-24): rayon CPU prover, 2× faster than WASM, circuits to 2^18. No Metal/GPU; runs on any Mac. Confirms "prove on consumer Mac" is viable for small Plonk circuits, not EVM blocks.

## 4. Practical numbers
- Ethproofs (today): ZisK 8×5090 and OpenVM 16×5090 are the only "eligible" real-time provers; ZisK p99 9.6 s on 4×5090; SP1 Hypercube/Pico Prism ~6–12 s on 16×5090. Cost ~$0.005–0.007/block.
- Ethereum mainnet block ≈ 30–60 M gas ≈ roughly 1–3 G RISC-V cycles (zkVM-dependent). At Jolt-Metal's claimed 10 M cyc/s a Mac needs **~2–5 min per mainnet block**; at typical CPU-only NEON rates (1–2 M cyc/s) 15–50 min. A single 4090/5080 with CUDA zkVMs does ~50–150 M cyc/s, i.e. 10–30× a Mac.
- Your target (100–1000 TPS blocks, ~2–20 M gas at 12 s): ~100 M–1 G cycles → **10 s–2 min per block on Jolt-Metal (if claims hold), 1–15 min CPU-only**. Minutes-scale is plausible; real-time is not. RAM: 32–64 GB unified recommended (RISC Zero segments fit in 16 GB; Jolt needs more for large traces).

## Recommendation
1. **Metal-first path: Jolt** (`a16z/jolt`, branch `feat/akita-metal`, Lattice Jolt + `jolt-kernels/metal`). Only zkVM with a native Metal prover and a published Mac number (>10 M cyc/s). Risks: alpha, unaudited, not on `main`, no reth/revm guest yet, on-chain verifier for Akita lattice PCS immature.
2. **Production-safe fallback: RISC Zero 3.0.6** — Metal on by default, audited, reth guest (Steel/zeth), but Groth16 wrap must run on an x86 box (your poc-cuda) and expect CPU-class throughput.
3. Avoid SP1/OpenVM/ZisK/Airbender/Pico for Mac proving: CUDA-only GPU paths; CPU fallbacks are AVX-tuned.

Realistic expectation for one M1 Max/M3 (32–64 GB): prove small-to-mid EVM blocks (≤1 G cycles) in **1–15 min**; mainnet-size blocks in **tens of minutes**; no real-time. Verification on laptops is trivial for all of the above.
