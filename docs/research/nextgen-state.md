# Proof-friendly state trees & state DBs — frontier as of 2026-09-26

Context: Rust EVM chain, small-field hash-based zkVM (M31/KoalaBear, lattice PCS), Apple Silicon provers, ≤300 KiB proofs.

## 1. Ethereum direction (verified)

- **Verkle is dead.** Replaced by **EIP-7864 unified binary tree** (accounts+storage+code in one 32-byte-keyed binary tree; ~75% smaller branches, no trusted setup, PQ-safe because hash-only). Draft since Jan 2025, on the 2026+ execution roadmap; overlay-migration (freeze old MPT, accrue into new tree) is the open engineering problem. [EIP-7864](https://eips.ethereum.org/EIPS/eip-7864), [EthCC "Verkle→Binary"](https://ethcc.io/archives/completing-the-circle-transitioning-from-verkle-to-binary-trees-in-ethereum)
- **Hash for 7864 still TBD**: reference impl uses **BLAKE3** (`ubt` crate also ships SHA-256 for geth compat); Keccak and Poseidon2 were listed candidates. [ubt 0.5.1](https://docs.rs/ubt/latest/ubt/)
- **Big shift — 2026-08-13: Justin Drake announced EF is abandoning Poseidon for L1, "pivoting to SHA or BLAKE."** Reason is not a break: hash-friendly SNARKs (Binius, **Flock**, SNARK.fast) made standard hashes cheap enough. Flock (Bünz/Rothblum/Wang, ePrint 2026/1329, Jun 2026): 82k BLAKE3 compressions/s on one M4 Max core, ~660k/s on 10 cores, ≈250× native overhead; 9× faster than Binius64 on SHA-256. The $992k Poseidon collision prize was paused 2026-08-01. [postquantum.com](https://postquantum.com/security-pqc/ethereum-roadmap-drops-poseidon/), [crypto.news](https://crypto.news/ethereum-l1-drops-poseidon-in-post-quantum-move/), [Flock](https://eprint.iacr.org/2026/1329)
- **Lean Ethereum / lean consensus** (published 2025-07-31): hash-only L1 (leanXMSS sigs, hash commitments, leanVM over **KoalaBear + Poseidon2**, WHIR). 2026 plan still says "exclusively Poseidon2" for the signature-aggregation layer; state-tree design explicitly *open*. leanVM production 2027, deployment ~2028. Expect the Aug-2026 pivot to migrate leanVM's hashing toward BLAKE3/SHA-256 in 2027 specs. [Lean 2026 plan](https://hackmd.io/@tcoratger/ryS1ElrWbx)

## 2. Hash choice for Merkle trees

| Hash | Design | Cost inside small-field STARK | Security status (2026) | Rust |
|---|---|---|---|---|
| **Poseidon2** (KoalaBear/M31, width 16) | algebraic SPN, x^3 / x^5 | Cheapest: ~1 AIR row set per perm; Plonky3 benchmark ≈2^20 perms in seconds; state-root proof ~0.5 s on M3 Pro | No full-round break. Initiative broke reduced rounds (KoalaBear 24–32-bit, M31 up to 40-bit partial); Phase 2 refocused on Poseidon1/KoalaBear; EF *deprioritized* it for L1 (Aug 2026). New "kleptographic MDS backdoor" paper (ePrint 2026/1901) hits Plonky3/Plonky2 matrices → demand nothing-up-my-sleeve matrix generation | `p3-poseidon2`, `p3-koala-bear`, `p3-mersenne-31` (MIT/Apache) |
| **BLAKE3** | ARX, 64-byte compression | ~10–30× Poseidon2 in classic AIR; with Flock-style batch Boolean proving ≈250× native (fast enough for EF) | Native, 10+ yrs review, hardware-fast (SIMD/NEON) | `blake3`, `p3-blake3` |
| **SHA-256** | Boolean | ~2× BLAKE3 cost in Flock (42k/s) | Best review history, hardware ISA | `sha2`, `p3-sha256` |
| **Keccak-256** | Boolean, f[1600] | Worst of the standard hashes (30k perm/s in Flock; 341 AIR calls vs 1024 for BLAKE3) | EVM compatibility only | `p3-keccak`, `tiny-keccak` |
| Rescue-Prime | algebraic, inverse S-box | Fewer rounds but slow native | Solid; little new work | `winterfell` crates |
| Monolith / Skyscraper(-v2) | lookup / Feistel on big primes | Skyscraper-v2 native ≈15× faster than Poseidon2 on BLS scalar; Monolith Goldilocks-oriented; neither suits 31-bit fields | Young, under-analyzed | research repos only |

Note: Plonky3 Merkle tree soundness with non-collision-resistant compression proven only when leaves are pre-hashed ("Billion Dollar Merkle Tree", CCS 2026, ePrint 2026/089) — always hash leaves.

## 3. State DB engines

| Name | Design | Proof generation cost | Maturity / license | Rust | Source |
|---|---|---|---|---|---|
| **NOMT** (Thrum, Sovereign SDK) | Binary sparse Merkle trie (Bitbox hashtable, page-aligned) + Beatree B-tree KV; io_uring; multiproofs/witnesses for batch changes | Cheap: binary tree, hasher pluggable (BLAKE3 default in examples) → maps 1:1 to zkVM verification | v1.0.4 (2026-05), ~1k commits, MIT/Apache-2 | native | [thrumdev/nomt](https://github.com/thrumdev/nomt) |
| **QMDB** (LayerZero) | Append-only "twig" Merkle over entries, in-memory Merkleization, SSD-friendly; inclusion/exclusion/historical proofs; 2.28M updates/s | SHA-256 hashing, fixed; fine with Flock-class provers, poor for classic AIR | Research-grade (24 commits main, 330★), MIT/Apache-2 | native | [LayerZero-Labs/qmdb](https://github.com/LayerZero-Labs/qmdb) |
| **Firewood** (Ava Labs) | Compaction-less on-disk MPT nodes, no generic KV; keeps recent revisions | Keccak MPT → expensive to prove | Beta v0.3.x (2026-03); **Ava Labs Ecosystem License** (Avalanche-only, forbids forks) → unusable | native | [ava-labs/firewood](https://github.com/ava-labs/firewood) |
| **MonadDB** | Custom on-disk Patricia trie, io_uring, raw block device | Keccak MPT; proof cost as Ethereum | Production (Monad mainnet); **GPL-3.0**, C++ | no | [docs.monad.xyz](https://docs.monad.xyz/monad-arch/execution/monaddb) |
| **reth 2.0** (Paradigm, Apr 2026) | MDBX hashed-state only; history in static files; RocksDB indices; `reth_trie_sparse` partial proofs/witnesses; <300 GB minimal node | Keccak MPT, but best-in-class witness/multiproof tooling (`SparseStateTrie`) | Production; MIT/Apache-2 | native | [reth 2.0](https://paradigm.xyz/2026/04/releasing-reth-2-0) |
| jmt / jmt-blake3 (Aptos/Penumbra JMT) | 16-ary Jellyfish Merkle, versioned | 16-ary → 4× more hashing per level than binary; BLAKE3 or SHA3 | Stable, Apache-2 | native | [jmt-blake3](https://lib.rs/crates/jmt-blake3) |
| ubt | EIP-7864 reference tree (BLAKE3 / SHA-256, rayon) | Binary, cheap | 0.5.1 (2026-09-24), early, MIT/Apache-2 | native | [ubt](https://docs.rs/ubt/latest/ubt/) |

## 4. Verkle/IPA vs Merkle in a PQ world
Verkle = Pedersen/IPA on elliptic curves → broken by Shor; needs no trusted setup but still not PQ. Lattice PCS (Greyhound, Cini et al.) can in principle rebuild "verkle-like" vector commitments PQ-safely, but they are research-only with KB-scale openings. Ethereum concluded that with a fast zkVM, a plain hash Merkle tree + a proof of the branch beats Verkle on both PQ and simplicity. For a hash-based zkVM chain there is no reason to touch Verkle.

## 5. State expiry / rent
Still no consensus EIP scheduled. EIP-2026 (rent prepay, 2019) stagnant; Vitalik's "one tree per period" expiry note remains a design sketch; ethereum.org lists expiry as post-statelessness. Observed mainnet growth ~326 MiB/week (~116 GiB/yr) at raised gas limits. Practical mitigations shipping: history expiry (EIP-4444), reth minimal mode, EIP-8252 reorg-window retention. Binary tree with 32-byte keys is designed so per-epoch trees can be layered later.

## 6. Recommendation (Metal-first, proof-first)

**Build now**
1. **Binary sparse Merkle tree with 32-byte unified keys mirroring EIP-7864 layout** (account/storage/code chunks co-located). Follow `ubt` key derivation so you stay Ethereum-compatible for tooling.
2. **Make the node hasher a trait** with two backends: `Poseidon2<KoalaBear, w=16>` (Plonky3, your prover's native cost) and **BLAKE3** (Ethereum's likely final choice, hardware-fast on Apple Silicon). Commit the root under *both* is unnecessary; pick Poseidon2 as consensus hash *today* for prover throughput, keep BLAKE3 selectable by chain config, and generate Poseidon2 matrices with a public nothing-up-my-sleeve procedure (2026/1901).
3. **Storage engine: NOMT** (Rust, MIT/Apache, binary, multiproofs, io_uring on Linux, works on macOS for dev). Wrap it behind a repository trait so QMDB can be swapped in if you need >1M updates/s. Do not use Firewood (license) or MonadDB (GPL, C++).
4. Pre-hash leaves before Merkle compression (CCS 2026 result); use `reth_trie_sparse` as the reference for witness/partial-proof APIs.

**Wait / watch**
- Final EIP-7864 hash decision and the leanVM 2027 spec (likely BLAKE3/SHA-256 after the Aug-2026 pivot). If your prover adds a Flock-style batch-Boolean gadget, flip the chain hash to BLAKE3 and drop the Poseidon security dependency.
- Poseidon Cryptanalysis Initiative through Dec 2026 (no full-round attacks yet; watch KoalaBear rounds).
- Lattice vector commitments and state expiry: no action; keep per-epoch tree layering possible in the schema.
