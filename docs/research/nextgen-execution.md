# Next-gen execution layer: fast + proof-friendly (research, 2026-09-26)

Context: Rust/revm chain, rewritten Block-STM executor, blocks proven by a RV64IM zkVM guest on Apple Silicon.

## 1. Parallel EVM state of the art

| Name | Claim (measured) | Maturity | Proof-friendly? | Rust | Source |
|---|---|---|---|---|---|
| Monad (async/deferred exec + MonadDB) | Consensus agrees on tx order first; execution lags 10 blocks (~400x time budget); >10k TPS; mainnet 2025-11-24 | Mainnet | Neutral: deferred exec == prover-friendly (proof lags too) | No (C++) | [docs.monad.xyz](https://docs.monad.xyz/), [monad.xyz/blog](https://monad.xyz/blog/how-monad-works) |
| Sei Giga (Autobahn) | Full consensus/execution decoupling, Block-STM OCC, ~200k TPS vs 12.5k (16x) | Mainnet 2026 | Neutral | No (Go) | [arXiv 2505.14914](https://arxiv.org/html/2505.14914), [docs.sei.io](https://docs.sei.io/learn/sei-giga-specs) |
| MegaETH mini-blocks | Sub-block "mini blocks" for ~10ms latency; standard EVM blocks kept for compat | Mainnet | Neutral; two block types complicates proof granularity | Partial (reth-based) | [docs.megaeth.com](https://docs.megaeth.com/miniblocks) |
| grevm 2.1 (Gravity) | DAG scheduler + Block-STM fallback; 2.96 Ggas/s hybrid (5.5x over v1), 11.25 Ggas/s Uniswap, 95% less CPU under contention; gravity-reth 4x reth 1.4.8 | Prod (Gravity L1); reth upstreaming | Neutral | **Yes** (Apache) | [docs.gravity.xyz](https://docs.gravity.xyz/research-and-development/grevm2), [github](https://github.com/Galxe/grevm) |
| RISE pevm | Block-STM for reth: 2x avg on mainnet blocks, 22x on independent swaps; plans mempool-derived static hints, finer-grain memory locations | Beta | Neutral | **Yes** | [risechain/pevm](https://github.com/risechain/pevm) |
| Block-STM v2 (Aptos) | Targets 256 cores; Deltalane aggregators for hot-key commutative writes | Mainnet (Move) | Neutral | Rust, but Move-bound | [Aptos](https://x.com/Aptos/status/1839379657122345248) |
| EIP-7928 BALs (static hints) | Block-level access list as **validity condition**: parallel exec, prefetch, parallel state root; Glamsterdam headliner, 200M gas target | Scheduled Q4 2026 | **High**: prover gets exact witness set up-front | reth impl. in progress | [EIP-7928](https://eips.ethereum.org/EIPS/eip-7928), [Forkcast](https://forkcast.org/eips/7928/) |
| Caveat: workload study | Re-execution on stale state changes gas for 46% Base / 13.9% ETH txs; access patterns limit static hints | Research 2026 | - | - | [arXiv 2606.19869](https://arxiv.org/abs/2606.19869) |

Reusable: grevm 2.1's DAG-then-STM hybrid and BAL-as-validity-rule are the two ideas worth lifting.

## 2. Proof-friendly execution

| Item | Claim | Maturity | Proof-friendliness | Rust | Source |
|---|---|---|---|---|---|
| zkVM precompiles | SP1/RISC Zero/OpenVM ship keccak, sha256, secp256k1, ed25519, bn254, bls12-381 as ECALL circuits; 5-10x cycle cuts, 95% on bn254 pairing | Prod | Critical | Yes | [Succinct](https://blog.succinct.xyz/succinctshipsprecompiles/), [RISC Zero 1.2](https://risczero.com/blog/risczero-zkvm-1.2) |
| EIP-7667 keccak repricing | Worst-case 30M-gas block = ~305 MB hashing / 83,650 keccaks; raises hash gas ~1.7x | Draft | High | - | [EIP-7667](https://eips.ethereum.org/EIPS/eip-7667) |
| EIP-8037/8038 state gas | Per-byte state cost + separate state-growth reservoir (multidimensional) | Glamsterdam Q4 2026 | Medium | alloy-evm has it | [EF blog](https://blog.ethereum.org/2026/08/24/glamsterdam-repricing-testing) |
| Modexp | Named "dramatically mispriced" vs proving cost | - | Restrict/reprice | - | [Succinct](https://blog.succinct.xyz/succinctshipsprecompiles/) |
| EOF (EIP-7692) | Removed from Fusaka; not in Glamsterdam core | Stalled | Low relevance | revm has it behind flag | [The Defiant](https://thedefiant.io/news/blockchains/ethereum-removes-evm-object-format-fusaka-upgrade-eyes-glamsterdam-b97edac0) |
| RISC-V replacing EVM on L1 | Vitalik re-pushed Mar 2026; >=18 months, no ACD consensus; L2s (Linea) moving first | Roadmap only | n/a | - | [BlockEden](https://blockeden.xyz/blog/2026/03/07/ethereum-risc-v-evm-replacement/) |

## 3. Execution/consensus decoupling and proofs as finality

| Name | Claim | Maturity | Source |
|---|---|---|---|
| Monad deferred execution | Order first, execute N blocks later | Mainnet | above |
| Alpenglow (Solana) | Votor ~150 ms finality; Rotor deferred; execution/SVM unchanged; mainnet target Oct 2026 | Testing | [solana.com](https://solana.com/upgrades/alpenglow) |
| EIP-8025 optional execution proofs | Validators may verify a proof instead of re-executing; targeted at Hegota (post-Glamsterdam), built on BALs + ePBS | Devnets | [zkEVM blog](https://zkevm.ethereum.foundation/blog/eip-8025-optional-execution-proofs-hegota) |
| Real-time proving | EF target <10 s for 99% blocks, <=$100k rig, 10 kW, <300 KiB proof; SP1 Hypercube 93% real-time on 200 GPUs; cost/block ~$0.04 -> ~half cent (Sep 2026) | Prod-adjacent | [EF](https://blog.ethereum.org/2025/07/10/realtime-proving), [ethproofs](https://hackmd.io/@willcorcoran/S1A840ZMZg) |

## 4. Gas for proving cost
"Proof gas" = per-opcode metering derived from empirical zkVM cycle profiling; academic (arXiv 2509.17126) plus EF worst-case-block work; only EIP-8037 (state dimension) and 7667 (hash) are near-shipping. No standard multidimensional-gas EIP for prover cycles yet.

## 5. Native rollups
EIP-8079 EXECUTE precompile: PoC demoed 2026-03-11, not in any scheduled fork; years out. ([EIP-8079](https://eips.ethereum.org/EIPS/eip-8079), [The Block](https://www.theblock.co/news/ecosystems/2026-03-11-ethereum-researchers-demo-native-rollups-prototype-that-could-simplify-layer-2-verification-393160))

## 6. Rust stack
- revm 43.0.1 (avoid 43.0.0 selfdestruct-finalization regression); alloy-evm carries EIP-8037 state-gas (mismatch bug noted).
- reth v2.4/2.5: sparse-trie caching, parallel `update_leaves`, parallel storage history; `ParallelStateRoot`/`ParallelSparseTrie` APIs removed/renamed, so pin. ExEx stable for indexing/side-tasks. ([reth releases](https://github.com/paradigmxyz/reth/releases))
- grevm 2.1 and pevm both target reth's `BlockExecutor` surface.

## Recommendation: "fast enough + cheapest to prove"

1. **Order-then-execute (Monad-style), execute lag = 1-2 blocks.** Consensus commits tx order + a BAL; execution and proving run off the critical path. Proof lag becomes the finality signal, mirroring EIP-8025.
2. **Make BALs mandatory (EIP-7928 semantics).** Producer emits BAL; your Block-STM uses it as a static DAG (grevm 2.1 style) and falls back to dynamic validation only on mismatch, which invalidates the block. The prover consumes the same BAL as its witness set, killing MPT-walk guesswork.
3. **Precompile whitelist tuned to your zkVM:** keccak, ecrecover, P-256 (RIP-7212), bn254 add/mul/pairing. Reprice modexp (or cap size) and hashes per EIP-7667 now; adopt EIP-8037 state gas so state growth is a separate dimension.
4. **Add a third gas dimension, "prove-gas",** metered from your guest's cycle counter per opcode/precompile, with a per-block cap. Cheap to implement in revm via an Inspector; it is the only lever that bounds worst-case proof time.
5. **State root:** hash the BAL-derived diff set with a parallel sparse trie (reth 2.5 primitives); consider Poseidon2/binary trie for the *proof* commitment while keeping keccak MPT only if EVM compat demands it.

**Build now:** BAL emission + static scheduling, prove-gas inspector, precompile whitelist, deferred execution pipeline, pin revm 43.0.1.
**Wait:** EOF, RISC-V-as-EVM, native rollup EXECUTE, Block-STM v2 256-core work (your Apple Silicon core count makes it moot).
